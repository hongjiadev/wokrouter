use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use fs4::fs_std::FileExt;
use secrecy::SecretString;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, Item, Table, value};
use url::Url;
use uuid::Uuid;
use wokrouter_wokcore_client::{IntegrationRuntime, ManagementError, WokCoreClient};

use super::{
    atomic_edit::{
        create_private_directory, private_file, remove_private_file, replace_private_file,
        secure_existing_file,
    },
    journal::{MutationError, MutationId, MutationJournal, MutationOperation, RestoreResult},
    token_store::ClientTokenStore,
};

const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Codex,
    Claude,
    Copilot,
}

impl ClientKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Copilot => "copilot",
        }
    }

    pub const fn client_id(self) -> &'static str {
        match self {
            Self::Codex => "wokrouter.codex",
            Self::Claude => "wokrouter.claude",
            Self::Copilot => "wokrouter.copilot",
        }
    }

    const fn required_protocol(self) -> &'static str {
        match self {
            Self::Codex => "openai.responses.v1",
            Self::Claude => "anthropic.messages.v1",
            Self::Copilot => "openai.chat_completions.v1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientRoots {
    pub home: PathBuf,
    pub codex_config: PathBuf,
    pub claude_settings: PathBuf,
    pub copilot_data: PathBuf,
}

impl ClientRoots {
    pub fn discover() -> Result<Self, IntegrationError> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(IntegrationError::MissingHome)?;
        let codex_root = configured_client_root("CODEX_HOME", || home.join(".codex"))?;
        let claude_root = configured_client_root("CLAUDE_CONFIG_DIR", || home.join(".claude"))?;
        let copilot_data = platform_copilot_data(&home)?;
        Ok(Self {
            codex_config: codex_root.join("config.toml"),
            claude_settings: claude_root.join("settings.json"),
            copilot_data,
            home,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntegrationStatus {
    NotInstalled,
    Native,
    Injected { revision: String },
    Drifted,
    Conflict,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RemoteRuntimeStatus {
    Healthy,
    Changed,
    Missing,
    Unsupported,
    IdentityMismatch,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RemoteInspection {
    pub runtime: RemoteRuntimeStatus,
    pub token_active: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CopilotSetup {
    pub base_url: String,
    pub provider_type: &'static str,
    pub api_format: &'static str,
    pub api_key_command: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ClientIntegrationManager {
    roots: ClientRoots,
    state_root: PathBuf,
    token_command: PathBuf,
    writable: bool,
    tokens: ClientTokenStore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IntegrationError {
    #[error("the client home directory is unavailable")]
    MissingHome,
    #[error("the client integration is not installed")]
    NotInstalled,
    #[error("the client integration is unsupported by this WokCore runtime")]
    Unsupported,
    #[error("the client configuration conflicts with WokRouter ownership")]
    Conflict,
    #[error("the client configuration is invalid")]
    InvalidConfig,
    #[error("the client integration state is invalid")]
    InvalidState,
    #[error("the client integration belongs to a different WokCore installation")]
    RuntimeChanged,
    #[error("the client integration operation failed")]
    Operation,
}

impl From<MutationError> for IntegrationError {
    fn from(error: MutationError) -> Self {
        match error {
            MutationError::UnsafeTarget => Self::Conflict,
            MutationError::InvalidRecord => Self::InvalidState,
            MutationError::Io | MutationError::UnsupportedPlatform => Self::Operation,
        }
    }
}

impl From<ManagementError> for IntegrationError {
    fn from(error: ManagementError) -> Self {
        match error {
            ManagementError::Missing | ManagementError::Stopped => Self::NotInstalled,
            ManagementError::Incompatible => Self::Unsupported,
            ManagementError::InvalidRuntime
            | ManagementError::InvalidResponse
            | ManagementError::InvalidInput => Self::InvalidState,
            ManagementError::Unauthorized
            | ManagementError::Forbidden
            | ManagementError::Conflict => Self::Operation,
        }
    }
}

impl ClientIntegrationManager {
    pub fn new(
        roots: ClientRoots,
        state_root: impl Into<PathBuf>,
        token_command: impl Into<PathBuf>,
    ) -> Result<Self, IntegrationError> {
        validate_roots(&roots)?;
        let state_root = state_root.into();
        let token_command = token_command.into();
        if !state_root.is_absolute()
            || !token_command.is_absolute()
            || token_command
                .as_os_str()
                .to_string_lossy()
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0'))
        {
            return Err(IntegrationError::InvalidState);
        }
        create_private_directory(&state_root)?;
        let tokens = ClientTokenStore::new(state_root.join("tokens"))?;
        create_private_directory(&state_root.join("registry"))?;
        Ok(Self {
            roots,
            state_root,
            token_command,
            writable: true,
            tokens,
        })
    }

    pub fn open_read_only(
        roots: ClientRoots,
        state_root: impl Into<PathBuf>,
    ) -> Result<Self, IntegrationError> {
        validate_roots(&roots)?;
        let state_root = state_root.into();
        if !state_root.is_absolute() {
            return Err(IntegrationError::InvalidState);
        }
        Ok(Self {
            roots,
            tokens: ClientTokenStore::open(state_root.join("tokens")),
            state_root,
            token_command: PathBuf::new(),
            writable: false,
        })
    }

    pub fn status(&self, client: ClientKind) -> Result<IntegrationStatus, IntegrationError> {
        let record = self.read_registry(client)?;
        match client {
            ClientKind::Codex => codex_status(
                config_root(&self.roots.codex_config)?,
                &self.roots.codex_config,
                record.as_ref(),
            ),
            ClientKind::Claude => claude_status(
                config_root(&self.roots.claude_settings)?,
                &self.roots.claude_settings,
                record.as_ref(),
            ),
            ClientKind::Copilot => {
                if let Some(record) = record {
                    if record.phase == IntegrationPhase::Active {
                        Ok(IntegrationStatus::Injected {
                            revision: record.revision,
                        })
                    } else {
                        Ok(IntegrationStatus::Conflict)
                    }
                } else if self.roots.copilot_data.exists() {
                    Ok(IntegrationStatus::Native)
                } else {
                    Ok(IntegrationStatus::NotInstalled)
                }
            }
        }
    }

    pub async fn inject(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        self.ensure_writable()?;
        let _lock = self.operation_lock().await?;
        self.inject_locked(client, core, management_token).await
    }

    async fn inject_locked(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        self.recover_pending(client, core, management_token).await?;
        if client == ClientKind::Copilot {
            self.copilot_setup_locked(core, management_token).await?;
            return self.status(client);
        }
        match self.status(client)? {
            IntegrationStatus::Injected { .. } => {
                let runtime = self.verified_runtime(client, core).await?;
                let record = self
                    .read_registry(client)?
                    .ok_or(IntegrationError::InvalidState)?;
                self.ensure_same_installation(&record, &runtime)?;
                if !core
                    .client_token_active_for_runtime(
                        &runtime,
                        management_token,
                        client.client_id(),
                        &record.token_id,
                    )
                    .await?
                {
                    return Err(IntegrationError::InvalidState);
                }
                return self
                    .sync_injected(client, &runtime, record, core, management_token)
                    .await;
            }
            IntegrationStatus::Drifted | IntegrationStatus::Conflict => {
                return Err(IntegrationError::Conflict);
            }
            IntegrationStatus::Unsupported => return Err(IntegrationError::Unsupported),
            IntegrationStatus::NotInstalled | IntegrationStatus::Native => {}
        }
        let runtime = self.verified_runtime(client, core).await?;
        self.activate_new(client, &runtime, core, management_token)
            .await
    }

    pub async fn copilot_setup(
        &self,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<CopilotSetup, IntegrationError> {
        self.ensure_writable()?;
        let _lock = self.operation_lock().await?;
        self.copilot_setup_locked(core, management_token).await
    }

    async fn copilot_setup_locked(
        &self,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<CopilotSetup, IntegrationError> {
        if !self.roots.copilot_data.is_dir() {
            return Err(IntegrationError::NotInstalled);
        }
        self.recover_pending(ClientKind::Copilot, core, management_token)
            .await?;
        let runtime = self.verified_runtime(ClientKind::Copilot, core).await?;
        match self.read_registry(ClientKind::Copilot)? {
            Some(mut record) => {
                self.ensure_same_installation(&record, &runtime)?;
                let local_token = self.tokens.read(ClientKind::Copilot).is_ok();
                let remote_token = core
                    .client_token_active_for_runtime(
                        &runtime,
                        management_token,
                        ClientKind::Copilot.client_id(),
                        &record.token_id,
                    )
                    .await?;
                if !local_token || !remote_token {
                    self.rotate_token(
                        ClientKind::Copilot,
                        &runtime,
                        record,
                        core,
                        management_token,
                    )
                    .await?;
                } else if record.runtime.instance_id != runtime.instance_id()
                    || record.runtime.base_url != runtime.base_url()
                {
                    record.runtime = RuntimeBinding::from_runtime(&runtime);
                    self.write_registry(&record)?;
                }
            }
            None => {
                self.activate_new(ClientKind::Copilot, &runtime, core, management_token)
                    .await?;
            }
        }
        Ok(CopilotSetup {
            base_url: runtime.base_url().to_owned(),
            provider_type: "openai",
            api_format: "chat_completions",
            api_key_command: self.token_command_arguments(ClientKind::Copilot),
        })
    }

    pub async fn restore(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<RestoreResult, IntegrationError> {
        self.ensure_writable()?;
        let _lock = self.operation_lock().await?;
        self.recover_pending(client, core, management_token).await?;
        let Some(record) = self.read_registry(client)? else {
            return Ok(RestoreResult::AlreadyRestored);
        };
        let runtime = self.verified_runtime(client, core).await?;
        self.ensure_same_installation(&record, &runtime)?;
        if core
            .client_token_active_for_runtime(
                &runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?
        {
            core.revoke_proxy_token_for_runtime(
                &runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?;
        }
        let result = self.restore_mutations(client, &record)?;
        if matches!(result, RestoreResult::Conflict { .. }) {
            return Ok(result);
        }
        self.tokens.remove(client)?;
        remove_private_file(&self.registry_path(client))?;
        Ok(if client == ClientKind::Copilot {
            RestoreResult::ManualActionRequired
        } else {
            result
        })
    }

    pub async fn repair(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        self.ensure_writable()?;
        let _lock = self.operation_lock().await?;
        self.recover_pending(client, core, management_token).await?;
        let status = self.status(client)?;
        match status {
            IntegrationStatus::Injected { .. } => {
                let runtime = self.verified_runtime(client, core).await?;
                let mut record = self
                    .read_registry(client)?
                    .ok_or(IntegrationError::InvalidState)?;
                self.ensure_same_installation(&record, &runtime)?;
                if client != ClientKind::Copilot {
                    self.sync_injected(client, &runtime, record, core, management_token)
                        .await?;
                    record = self
                        .read_registry(client)?
                        .ok_or(IntegrationError::InvalidState)?;
                }
                let local_token = self.tokens.read(client).is_ok();
                let remote_token = core
                    .client_token_active_for_runtime(
                        &runtime,
                        management_token,
                        client.client_id(),
                        &record.token_id,
                    )
                    .await?;
                if local_token && remote_token {
                    if client == ClientKind::Copilot {
                        record.runtime = RuntimeBinding::from_runtime(&runtime);
                        self.write_registry(&record)?;
                    }
                    Ok(IntegrationStatus::Injected {
                        revision: record.revision,
                    })
                } else {
                    self.rotate_token(client, &runtime, record, core, management_token)
                        .await
                }
            }
            IntegrationStatus::NotInstalled | IntegrationStatus::Native => {
                self.inject_locked(client, core, management_token).await
            }
            IntegrationStatus::Drifted | IntegrationStatus::Conflict => {
                Err(IntegrationError::Conflict)
            }
            IntegrationStatus::Unsupported => Err(IntegrationError::Unsupported),
        }
    }

    pub fn read_token(&self, client: ClientKind) -> Result<SecretString, IntegrationError> {
        self.tokens.read(client).map_err(Into::into)
    }

    pub(super) async fn inspect_remote(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: Option<&SecretString>,
    ) -> RemoteInspection {
        let runtime = match self.verified_runtime(client, core).await {
            Ok(runtime) => runtime,
            Err(IntegrationError::MissingHome | IntegrationError::NotInstalled) => {
                return RemoteInspection {
                    runtime: RemoteRuntimeStatus::Missing,
                    token_active: None,
                };
            }
            Err(IntegrationError::Unsupported) => {
                return RemoteInspection {
                    runtime: RemoteRuntimeStatus::Unsupported,
                    token_active: None,
                };
            }
            Err(_) => {
                return RemoteInspection {
                    runtime: RemoteRuntimeStatus::Invalid,
                    token_active: None,
                };
            }
        };
        let record = match self.read_registry(client) {
            Ok(Some(record)) if record.phase == IntegrationPhase::Active => record,
            _ => {
                return RemoteInspection {
                    runtime: RemoteRuntimeStatus::Invalid,
                    token_active: None,
                };
            }
        };
        if !record.runtime.same_installation(&runtime) {
            return RemoteInspection {
                runtime: RemoteRuntimeStatus::IdentityMismatch,
                token_active: None,
            };
        }
        let runtime_status = if record.runtime.matches_runtime(&runtime) {
            RemoteRuntimeStatus::Healthy
        } else {
            RemoteRuntimeStatus::Changed
        };
        let Some(management_token) = management_token else {
            return RemoteInspection {
                runtime: runtime_status,
                token_active: None,
            };
        };
        let token_active = core
            .client_token_active_for_runtime(
                &runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await
            .ok();
        RemoteInspection {
            runtime: runtime_status,
            token_active,
        }
    }

    async fn verified_runtime(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
    ) -> Result<IntegrationRuntime, IntegrationError> {
        let runtime = core.integration_runtime().await?;
        if !runtime.supports_capability("client_token.issue")
            || !runtime.supports_capability("client_token.inspect")
            || !runtime.supports_capability("client_token.revoke")
            || !runtime.supports_protocol(client.required_protocol())
        {
            return Err(IntegrationError::Unsupported);
        }
        Ok(runtime)
    }

    async fn activate_new(
        &self,
        client: ClientKind,
        runtime: &IntegrationRuntime,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        let mut record = IntegrationRecord {
            schema_version: SCHEMA_VERSION,
            client,
            phase: IntegrationPhase::Preparing,
            token_id: Uuid::new_v4().to_string(),
            mutation_ids: Vec::new(),
            config_hash: None,
            revision: Uuid::new_v4().to_string(),
            runtime: RuntimeBinding::from_runtime(runtime),
        };
        self.write_registry(&record)?;
        let result = self
            .finish_activation(&mut record, runtime, core, management_token)
            .await;
        if let Err(error) = result {
            return if self
                .recover_pending(client, core, management_token)
                .await
                .is_ok()
            {
                Err(error)
            } else {
                Err(IntegrationError::InvalidState)
            };
        }
        result
    }

    async fn finish_activation(
        &self,
        record: &mut IntegrationRecord,
        runtime: &IntegrationRuntime,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        let issued = core
            .issue_proxy_token_for_runtime_with_preallocated_id(
                runtime,
                management_token,
                record.client.client_id(),
                &record.token_id,
            )
            .await?;
        self.tokens.write(record.client, issued.token())?;
        if record.client == ClientKind::Copilot {
            record.phase = IntegrationPhase::Active;
            self.write_registry(record)?;
            return Ok(IntegrationStatus::Injected {
                revision: record.revision.clone(),
            });
        }
        let client = record.client;
        let (target, replacement, operation) = match client {
            ClientKind::Codex => {
                let target = &self.roots.codex_config;
                create_target_parent(target)?;
                let current = read_optional_text(config_root(target)?, target)?;
                let rendered = render_codex(
                    current.as_deref().unwrap_or(""),
                    runtime,
                    &self.token_command,
                    &record.revision,
                    None,
                )?;
                (
                    target,
                    rendered.into_bytes(),
                    MutationOperation::CodexConfig,
                )
            }
            ClientKind::Claude => {
                let target = &self.roots.claude_settings;
                create_target_parent(target)?;
                let current = read_optional_text(config_root(target)?, target)?;
                let rendered = render_claude(
                    current.as_deref().unwrap_or("{}"),
                    runtime,
                    &self.token_command,
                    &record.revision,
                    None,
                )?;
                (
                    target,
                    rendered.into_bytes(),
                    MutationOperation::ClaudeConfig,
                )
            }
            ClientKind::Copilot => return Err(IntegrationError::InvalidState),
        };
        let journal = self.journal(client)?;
        let mut mutation = journal.begin(target, &replacement, operation)?;
        record.mutation_ids.push(mutation.id().clone());
        record.config_hash = Some(content_hash(&replacement));
        self.write_registry(record)?;
        mutation.apply()?;
        mutation.commit()?;
        record.phase = IntegrationPhase::Active;
        self.write_registry(record)?;
        Ok(IntegrationStatus::Injected {
            revision: record.revision.clone(),
        })
    }

    async fn sync_injected(
        &self,
        client: ClientKind,
        runtime: &IntegrationRuntime,
        mut record: IntegrationRecord,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        let (target, current, operation) = match client {
            ClientKind::Codex => (
                &self.roots.codex_config,
                read_optional_text(
                    config_root(&self.roots.codex_config)?,
                    &self.roots.codex_config,
                )?,
                MutationOperation::CodexConfig,
            ),
            ClientKind::Claude => (
                &self.roots.claude_settings,
                read_optional_text(
                    config_root(&self.roots.claude_settings)?,
                    &self.roots.claude_settings,
                )?,
                MutationOperation::ClaudeConfig,
            ),
            ClientKind::Copilot => return Err(IntegrationError::InvalidState),
        };
        let current = current.ok_or(IntegrationError::Conflict)?;
        if record.config_hash.as_deref() != Some(content_hash(current.as_bytes()).as_str()) {
            return Err(IntegrationError::Conflict);
        }
        let desired = match client {
            ClientKind::Codex => render_codex(
                &current,
                runtime,
                &self.token_command,
                &record.revision,
                Some(&record.revision),
            )?,
            ClientKind::Claude => render_claude(
                &current,
                runtime,
                &self.token_command,
                &record.revision,
                Some(&record.revision),
            )?,
            ClientKind::Copilot => return Err(IntegrationError::InvalidState),
        };
        if desired == current {
            record.runtime = RuntimeBinding::from_runtime(runtime);
            self.write_registry(&record)?;
            return Ok(IntegrationStatus::Injected {
                revision: record.revision,
            });
        }

        let revision = Uuid::new_v4().to_string();
        let replacement = match client {
            ClientKind::Codex => render_codex(
                &current,
                runtime,
                &self.token_command,
                &revision,
                Some(&record.revision),
            )?,
            ClientKind::Claude => render_claude(
                &current,
                runtime,
                &self.token_command,
                &revision,
                Some(&record.revision),
            )?,
            ClientKind::Copilot => return Err(IntegrationError::InvalidState),
        }
        .into_bytes();
        let journal = self.journal(client)?;
        let mut mutation = journal.begin(target, &replacement, operation)?;
        record.phase = IntegrationPhase::Preparing;
        record.mutation_ids.push(mutation.id().clone());
        record.config_hash = Some(content_hash(&replacement));
        record.revision = revision.clone();
        record.runtime = RuntimeBinding::from_runtime(runtime);
        let result = (|| {
            self.write_registry(&record)?;
            mutation.apply()?;
            mutation.commit()?;
            record.phase = IntegrationPhase::Active;
            self.write_registry(&record)
        })();
        if let Err(error) = result {
            return if self
                .recover_pending(client, core, management_token)
                .await
                .is_ok()
            {
                Err(error)
            } else {
                Err(IntegrationError::InvalidState)
            };
        }
        Ok(IntegrationStatus::Injected { revision })
    }

    async fn rotate_token(
        &self,
        client: ClientKind,
        runtime: &IntegrationRuntime,
        mut record: IntegrationRecord,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<IntegrationStatus, IntegrationError> {
        if core
            .client_token_active_for_runtime(
                runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?
        {
            core.revoke_proxy_token_for_runtime(
                runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?;
        }
        record.phase = IntegrationPhase::Preparing;
        record.token_id = Uuid::new_v4().to_string();
        record.runtime = RuntimeBinding::from_runtime(runtime);
        self.write_registry(&record)?;
        let result = async {
            let issued = core
                .issue_proxy_token_for_runtime_with_preallocated_id(
                    runtime,
                    management_token,
                    client.client_id(),
                    &record.token_id,
                )
                .await?;
            self.tokens.write(client, issued.token())?;
            record.phase = IntegrationPhase::Active;
            self.write_registry(&record)?;
            Ok(IntegrationStatus::Injected {
                revision: record.revision.clone(),
            })
        }
        .await;
        if let Err(error) = result {
            return if self
                .recover_pending(client, core, management_token)
                .await
                .is_ok()
            {
                Err(error)
            } else {
                Err(IntegrationError::InvalidState)
            };
        }
        result
    }

    async fn recover_pending(
        &self,
        client: ClientKind,
        core: &WokCoreClient,
        management_token: &SecretString,
    ) -> Result<(), IntegrationError> {
        let Some(record) = self.read_registry(client)? else {
            return Ok(());
        };
        if record.phase == IntegrationPhase::Active {
            return Ok(());
        }
        let runtime = self.verified_runtime(client, core).await?;
        self.ensure_same_installation(&record, &runtime)?;
        if core
            .client_token_active_for_runtime(
                &runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?
        {
            core.revoke_proxy_token_for_runtime(
                &runtime,
                management_token,
                client.client_id(),
                &record.token_id,
            )
            .await?;
        }
        self.tokens.remove(client)?;
        if matches!(
            self.restore_mutations(client, &record)?,
            RestoreResult::Conflict { .. }
        ) {
            return Err(IntegrationError::Conflict);
        }
        remove_private_file(&self.registry_path(client))?;
        Ok(())
    }

    fn restore_mutations(
        &self,
        client: ClientKind,
        record: &IntegrationRecord,
    ) -> Result<RestoreResult, IntegrationError> {
        if record.mutation_ids.is_empty() {
            return Ok(RestoreResult::Restored);
        }
        let mut result = RestoreResult::AlreadyRestored;
        let journal = self.journal(client)?;
        for id in record.mutation_ids.iter().rev() {
            match journal.restore(id)? {
                RestoreResult::Restored => result = RestoreResult::Restored,
                RestoreResult::AlreadyRestored => {}
                conflict @ RestoreResult::Conflict { .. } => return Ok(conflict),
                RestoreResult::ManualActionRequired => {
                    return Err(IntegrationError::InvalidState);
                }
            }
        }
        Ok(result)
    }

    fn ensure_same_installation(
        &self,
        record: &IntegrationRecord,
        runtime: &IntegrationRuntime,
    ) -> Result<(), IntegrationError> {
        if record.runtime.same_installation(runtime) {
            Ok(())
        } else {
            Err(IntegrationError::RuntimeChanged)
        }
    }

    fn token_command_arguments(&self, client: ClientKind) -> Vec<String> {
        vec![
            self.token_command.to_string_lossy().into_owned(),
            "integration-token".to_owned(),
            client.as_str().to_owned(),
        ]
    }

    fn read_registry(
        &self,
        client: ClientKind,
    ) -> Result<Option<IntegrationRecord>, IntegrationError> {
        let path = self.registry_path(client);
        match fs::symlink_metadata(&path) {
            Ok(_) if !private_file(&path) => return Err(IntegrationError::InvalidState),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(IntegrationError::Operation),
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => return Err(IntegrationError::Operation),
        };
        if bytes.len() > 16 * 1024 {
            return Err(IntegrationError::InvalidState);
        }
        let record: IntegrationRecord =
            serde_json::from_slice(&bytes).map_err(|_| IntegrationError::InvalidState)?;
        if record.schema_version != SCHEMA_VERSION
            || record.client != client
            || !valid_uuid(&record.token_id)
            || !Uuid::parse_str(&record.revision)
                .is_ok_and(|revision| revision.to_string() == record.revision)
            || record.mutation_ids.iter().any(|id| !id.is_valid())
            || record.mutation_ids.iter().collect::<HashSet<_>>().len() != record.mutation_ids.len()
            || !valid_record_shape(&record)
            || record
                .config_hash
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || !valid_runtime_binding(&record.runtime)
        {
            return Err(IntegrationError::InvalidState);
        }
        Ok(Some(record))
    }

    fn write_registry(&self, record: &IntegrationRecord) -> Result<(), IntegrationError> {
        let bytes = serde_json::to_vec(record).map_err(|_| IntegrationError::InvalidState)?;
        replace_private_file(&self.registry_path(record.client), &bytes)?;
        Ok(())
    }

    fn registry_path(&self, client: ClientKind) -> PathBuf {
        self.state_root
            .join("registry")
            .join(format!("{}.json", client.as_str()))
    }

    fn ensure_writable(&self) -> Result<(), IntegrationError> {
        self.writable
            .then_some(())
            .ok_or(IntegrationError::InvalidState)
    }

    fn journal(&self, client: ClientKind) -> Result<MutationJournal, IntegrationError> {
        self.ensure_writable()?;
        let target = match client {
            ClientKind::Codex => &self.roots.codex_config,
            ClientKind::Claude => &self.roots.claude_settings,
            ClientKind::Copilot => return Err(IntegrationError::InvalidState),
        };
        let allowed_root = target.parent().ok_or(IntegrationError::InvalidState)?;
        MutationJournal::new(
            self.state_root.join("journal").join(client.as_str()),
            allowed_root,
        )
        .map_err(Into::into)
    }

    async fn operation_lock(&self) -> Result<std::fs::File, IntegrationError> {
        let path = self.state_root.join("operation.lock");
        tokio::task::spawn_blocking(move || {
            if fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
            {
                return Err(IntegrationError::InvalidState);
            }
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|_| IntegrationError::Operation)?;
            secure_existing_file(&path).map_err(IntegrationError::from)?;
            file.lock_exclusive()
                .map_err(|_| IntegrationError::Operation)?;
            Ok(file)
        })
        .await
        .map_err(|_| IntegrationError::Operation)?
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrationRecord {
    schema_version: u32,
    client: ClientKind,
    phase: IntegrationPhase,
    token_id: String,
    mutation_ids: Vec<MutationId>,
    config_hash: Option<String>,
    revision: String,
    runtime: RuntimeBinding,
}

#[derive(Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum IntegrationPhase {
    Preparing,
    Active,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeBinding {
    installation_id: String,
    instance_id: String,
    wokcore_version: String,
    management_api_major: u32,
    base_url: String,
}

impl RuntimeBinding {
    fn from_runtime(runtime: &IntegrationRuntime) -> Self {
        Self {
            installation_id: runtime.installation_id().to_owned(),
            instance_id: runtime.instance_id(),
            wokcore_version: runtime.wokcore_version(),
            management_api_major: runtime.management_api_major(),
            base_url: runtime.base_url().to_owned(),
        }
    }

    fn same_installation(&self, runtime: &IntegrationRuntime) -> bool {
        self.installation_id == runtime.installation_id()
    }

    fn matches_runtime(&self, runtime: &IntegrationRuntime) -> bool {
        self.same_installation(runtime)
            && self.instance_id == runtime.instance_id()
            && self.wokcore_version == runtime.wokcore_version()
            && self.management_api_major == runtime.management_api_major()
            && self.base_url == runtime.base_url()
    }
}

fn configured_client_root(
    variable: &str,
    fallback: impl FnOnce() -> PathBuf,
) -> Result<PathBuf, IntegrationError> {
    match std::env::var_os(variable) {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(IntegrationError::InvalidState)
            }
        }
        None => Ok(fallback()),
    }
}

fn config_root(path: &Path) -> Result<&Path, IntegrationError> {
    path.parent().ok_or(IntegrationError::InvalidState)
}

fn validate_roots(roots: &ClientRoots) -> Result<(), IntegrationError> {
    if !roots.home.is_absolute()
        || !roots.codex_config.is_absolute()
        || !roots.claude_settings.is_absolute()
        || !roots.copilot_data.is_absolute()
        || roots
            .home
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || roots
            .codex_config
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || roots
            .claude_settings
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || roots
            .codex_config
            .file_name()
            .and_then(|name| name.to_str())
            != Some("config.toml")
        || roots
            .claude_settings
            .file_name()
            .and_then(|name| name.to_str())
            != Some("settings.json")
    {
        return Err(IntegrationError::InvalidState);
    }
    Ok(())
}

fn codex_status(
    home: &Path,
    path: &Path,
    record: Option<&IntegrationRecord>,
) -> Result<IntegrationStatus, IntegrationError> {
    let Some(document) = read_optional_text(home, path)? else {
        return Ok(if record.is_some() {
            IntegrationStatus::Drifted
        } else {
            IntegrationStatus::NotInstalled
        });
    };
    let parsed = DocumentMut::from_str(&document).map_err(|_| IntegrationError::InvalidConfig)?;
    let has_provider = parsed
        .get("model_providers")
        .and_then(Item::as_table)
        .is_some_and(|providers| providers.contains_key("wokcore"));
    let current_hash = content_hash(document.as_bytes());
    match record {
        Some(record)
            if record.phase == IntegrationPhase::Active
                && record.config_hash.as_deref() == Some(current_hash.as_str()) =>
        {
            Ok(IntegrationStatus::Injected {
                revision: record.revision.clone(),
            })
        }
        Some(record) if record.phase == IntegrationPhase::Preparing => {
            Ok(IntegrationStatus::Conflict)
        }
        Some(_) => Ok(IntegrationStatus::Drifted),
        None if has_provider => Ok(IntegrationStatus::Conflict),
        None => Ok(IntegrationStatus::Native),
    }
}

fn claude_status(
    home: &Path,
    path: &Path,
    record: Option<&IntegrationRecord>,
) -> Result<IntegrationStatus, IntegrationError> {
    let Some(document) = read_optional_text(home, path)? else {
        return Ok(if record.is_some() {
            IntegrationStatus::Drifted
        } else {
            IntegrationStatus::NotInstalled
        });
    };
    let parsed: Value =
        serde_json::from_str(&document).map_err(|_| IntegrationError::InvalidConfig)?;
    let has_owned_fields = parsed.get("apiKeyHelper").is_some()
        || parsed
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| env.contains_key("ANTHROPIC_BASE_URL"));
    let current_hash = content_hash(document.as_bytes());
    match record {
        Some(record)
            if record.phase == IntegrationPhase::Active
                && record.config_hash.as_deref() == Some(current_hash.as_str()) =>
        {
            Ok(IntegrationStatus::Injected {
                revision: record.revision.clone(),
            })
        }
        Some(record) if record.phase == IntegrationPhase::Preparing => {
            Ok(IntegrationStatus::Conflict)
        }
        Some(_) => Ok(IntegrationStatus::Drifted),
        None if has_owned_fields => Ok(IntegrationStatus::Conflict),
        None => Ok(IntegrationStatus::Native),
    }
}

fn render_codex(
    document: &str,
    runtime: &IntegrationRuntime,
    command: &Path,
    revision: &str,
    expected_revision: Option<&str>,
) -> Result<String, IntegrationError> {
    let line_ending = document_original_line_ending(document);
    let mut document =
        DocumentMut::from_str(document).map_err(|_| IntegrationError::InvalidConfig)?;
    let has_provider = document
        .get("model_providers")
        .and_then(Item::as_table)
        .is_some_and(|providers| providers.contains_key("wokcore"));
    match expected_revision {
        Some(_) if has_provider => {}
        None if !has_provider => {}
        Some(_) | None => return Err(IntegrationError::Conflict),
    }
    document["model_provider"] = value("wokcore");
    let providers = document
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or(IntegrationError::InvalidConfig)?;
    let mut provider = Table::new();
    provider["name"] = value("WokCore");
    provider["base_url"] = value(runtime.base_url());
    provider["wire_api"] = value("responses");
    let mut auth = Table::new();
    auth["command"] = value(command.to_string_lossy().into_owned());
    let mut args = Array::new();
    args.push("integration-token");
    args.push("codex");
    auth["args"] = value(args);
    auth["refresh_interval_ms"] = value(0);
    provider["auth"] = Item::Table(auth);
    providers["wokcore"] = Item::Table(provider);
    let _ = revision;
    preserve_line_endings(document.to_string(), line_ending)
}

fn render_claude(
    document: &str,
    runtime: &IntegrationRuntime,
    command: &Path,
    revision: &str,
    expected_revision: Option<&str>,
) -> Result<String, IntegrationError> {
    let mut document: Value =
        serde_json::from_str(document).map_err(|_| IntegrationError::InvalidConfig)?;
    let object = document
        .as_object_mut()
        .ok_or(IntegrationError::InvalidConfig)?;
    let has_owned_fields = object.contains_key("apiKeyHelper")
        || object
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| env.contains_key("ANTHROPIC_BASE_URL"));
    match expected_revision {
        Some(_) if has_owned_fields => {}
        None if !has_owned_fields => {}
        Some(_) | None => return Err(IntegrationError::Conflict),
    }
    let root_url = Url::parse(runtime.base_url())
        .and_then(|url| url.join("../"))
        .map_err(|_| IntegrationError::InvalidState)?;
    object.insert(
        "apiKeyHelper".to_owned(),
        Value::String(shell_token_command(command, ClientKind::Claude)?),
    );
    let env = object
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(IntegrationError::InvalidConfig)?;
    env.insert(
        "ANTHROPIC_BASE_URL".to_owned(),
        Value::String(root_url.to_string()),
    );
    let _ = revision;
    let mut rendered =
        serde_json::to_string_pretty(&document).map_err(|_| IntegrationError::InvalidConfig)?;
    rendered.push('\n');
    Ok(rendered)
}

fn shell_token_command(command: &Path, client: ClientKind) -> Result<String, IntegrationError> {
    let command = command.to_string_lossy();
    #[cfg(windows)]
    {
        if command.contains('"') {
            return Err(IntegrationError::InvalidState);
        }
        Ok(format!(
            "\"{command}\" integration-token {}",
            client.as_str()
        ))
    }
    #[cfg(not(windows))]
    {
        let quoted = command.replace('\'', "'\\''");
        Ok(format!("'{quoted}' integration-token {}", client.as_str()))
    }
}

fn read_optional_text(root: &Path, path: &Path) -> Result<Option<String>, IntegrationError> {
    validate_read_target(root, path)?;
    match fs::read(path) {
        Ok(bytes) if bytes.len() <= 16 * 1024 * 1024 => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| IntegrationError::InvalidConfig),
        Ok(_) => Err(IntegrationError::InvalidConfig),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(IntegrationError::Operation),
    }
}

fn validate_read_target(root: &Path, target: &Path) -> Result<(), IntegrationError> {
    if !lexical_descendant(root, target) {
        return Err(IntegrationError::InvalidState);
    }
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(IntegrationError::Operation),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(IntegrationError::InvalidState);
    }
    let relative = target
        .strip_prefix(root)
        .map_err(|_| IntegrationError::InvalidState)?;
    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || (components.peek().is_some() && !metadata.is_dir())
                    || (components.peek().is_none() && !metadata.is_file()) =>
            {
                return Err(IntegrationError::InvalidState);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(IntegrationError::Operation),
        }
    }
    Ok(())
}

fn lexical_descendant(root: &Path, target: &Path) -> bool {
    target.starts_with(root)
        && target
            .components()
            .all(|component| !matches!(component, Component::ParentDir | Component::CurDir))
}

fn create_target_parent(path: &Path) -> Result<(), IntegrationError> {
    let parent = path.parent().ok_or(IntegrationError::InvalidState)?;
    fs::create_dir_all(parent).map_err(|_| IntegrationError::Operation)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| IntegrationError::Operation)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(IntegrationError::InvalidState);
    }
    Ok(())
}

fn preserve_line_endings(
    rendered: String,
    line_ending: &'static str,
) -> Result<String, IntegrationError> {
    if line_ending == "\r\n" {
        Ok(rendered.replace("\r\n", "\n").replace('\n', "\r\n"))
    } else {
        Ok(rendered)
    }
}

fn document_original_line_ending(document: &str) -> &'static str {
    if document.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

fn valid_record_shape(record: &IntegrationRecord) -> bool {
    match (record.client, record.phase) {
        (ClientKind::Copilot, _) => record.mutation_ids.is_empty() && record.config_hash.is_none(),
        (_, IntegrationPhase::Active) => {
            !record.mutation_ids.is_empty() && record.config_hash.is_some()
        }
        (_, IntegrationPhase::Preparing) => {
            record.mutation_ids.is_empty() == record.config_hash.is_none()
        }
    }
}

fn valid_runtime_binding(runtime: &RuntimeBinding) -> bool {
    if !valid_hash(&runtime.installation_id)
        || !valid_uuid(&runtime.instance_id)
        || Version::parse(&runtime.wokcore_version).is_err()
        || runtime.management_api_major != 1
    {
        return false;
    }
    Url::parse(&runtime.base_url).is_ok_and(|url| {
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some_and(|port| port != 0)
            && url.path() == "/v1/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn content_hash(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(windows)]
fn platform_copilot_data(_home: &Path) -> Result<PathBuf, IntegrationError> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("GitHub Copilot"))
        .ok_or(IntegrationError::MissingHome)
}

#[cfg(target_os = "macos")]
fn platform_copilot_data(home: &Path) -> Result<PathBuf, IntegrationError> {
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("GitHub Copilot"))
}

#[cfg(target_os = "linux")]
fn platform_copilot_data(home: &Path) -> Result<PathBuf, IntegrationError> {
    Ok(std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("github-copilot"))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn platform_copilot_data(_home: &Path) -> Result<PathBuf, IntegrationError> {
    Err(IntegrationError::MissingHome)
}
