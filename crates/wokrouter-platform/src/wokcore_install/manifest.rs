use std::str;

use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

use super::WokCoreInstallError;

pub(super) const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub(super) const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
pub(super) const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PUBLIC_KEY_BYTES: usize = 4 * 1024;

const TARGETS: [TargetContract; 5] = [
    TargetContract::new("x86_64-pc-windows-msvc", "zip", "wokcore.exe"),
    TargetContract::new("x86_64-apple-darwin", "tar.gz", "wokcore"),
    TargetContract::new("aarch64-apple-darwin", "tar.gz", "wokcore"),
    TargetContract::new("x86_64-unknown-linux-gnu", "tar.gz", "wokcore"),
    TargetContract::new("aarch64-unknown-linux-gnu", "tar.gz", "wokcore"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseCandidate {
    pub(super) version: Version,
    pub(super) artifact: ReleaseArtifact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseArtifact {
    pub(super) file: String,
    pub(super) executable: String,
    pub(super) size: u64,
    pub(super) sha256: String,
    pub(super) url: String,
}

pub(super) fn verify_manifest(
    manifest_bytes: &[u8],
    signature_bytes: &[u8],
    public_key_text: &str,
    target: &str,
) -> Result<ReleaseCandidate, WokCoreInstallError> {
    if manifest_bytes.is_empty() || manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(WokCoreInstallError::InvalidManifest);
    }
    if signature_bytes.is_empty() || signature_bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    let key_id = verify_signature(manifest_bytes, signature_bytes, public_key_text.as_bytes())?;
    let document = parse_manifest(manifest_bytes)?;
    validate_document(document, &key_id, target)
}

pub(super) fn validate_public_key(public_key: &str) -> Result<(), WokCoreInstallError> {
    public_key_id(public_key).map(|_| ())
}

fn parse_manifest(manifest: &[u8]) -> Result<ManifestDocument, WokCoreInstallError> {
    serde_json::from_slice(manifest).map_err(|_| WokCoreInstallError::InvalidManifest)
}

fn verify_signature(
    manifest: &[u8],
    signature: &[u8],
    public_key: &[u8],
) -> Result<String, WokCoreInstallError> {
    if public_key.is_empty() || public_key.len() > MAX_PUBLIC_KEY_BYTES {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    let public_key_text =
        str::from_utf8(public_key).map_err(|_| WokCoreInstallError::InvalidSignature)?;
    let signature_text =
        str::from_utf8(signature).map_err(|_| WokCoreInstallError::InvalidSignature)?;
    let key_id = public_key_id(public_key_text)?;
    if signature_text.lines().count() != 4 {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    let decoded_key =
        PublicKey::decode(public_key_text).map_err(|_| WokCoreInstallError::InvalidSignature)?;
    let decoded_signature =
        Signature::decode(signature_text).map_err(|_| WokCoreInstallError::InvalidSignature)?;
    decoded_key
        .verify(manifest, &decoded_signature, false)
        .map_err(|_| WokCoreInstallError::InvalidSignature)?;
    Ok(key_id)
}

fn public_key_id(public_key: &str) -> Result<String, WokCoreInstallError> {
    let lines = public_key.lines().collect::<Vec<_>>();
    let [comment, payload] = lines.as_slice() else {
        return Err(WokCoreInstallError::InvalidSignature);
    };
    let decoded = STANDARD
        .decode(payload)
        .map_err(|_| WokCoreInstallError::InvalidSignature)?;
    if decoded.len() != 42 || decoded[..2] != [0x45, 0x64] {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    let key_id = decoded[2..10]
        .iter()
        .rev()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    if *comment != format!("untrusted comment: minisign public key {key_id}") {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    Ok(key_id)
}

fn validate_document(
    document: ManifestDocument,
    key_id: &str,
    target: &str,
) -> Result<ReleaseCandidate, WokCoreInstallError> {
    if document.schema_version != 1 || document.api_major != 1 {
        return Err(WokCoreInstallError::IncompatibleManifest);
    }
    if document.product != "wokcore"
        || document.signing_key_id != key_id
        || document.artifacts.len() != TARGETS.len()
    {
        return Err(WokCoreInstallError::InvalidManifest);
    }
    let version =
        Version::parse(&document.version).map_err(|_| WokCoreInstallError::InvalidManifest)?;
    if document.version.len() > 128 || version.to_string() != document.version {
        return Err(WokCoreInstallError::InvalidManifest);
    }

    let mut selected = None;
    for (artifact, contract) in document.artifacts.into_iter().zip(TARGETS) {
        let validated = validate_artifact(artifact, contract, &document.version)?;
        if contract.target == target {
            selected = Some(validated);
        }
    }
    selected
        .map(|artifact| ReleaseCandidate { version, artifact })
        .ok_or(WokCoreInstallError::IncompatibleManifest)
}

fn validate_artifact(
    artifact: ArtifactDocument,
    contract: TargetContract,
    version: &str,
) -> Result<ReleaseArtifact, WokCoreInstallError> {
    let expected_file = format!(
        "wokcore-v{version}-{}.{}",
        contract.target, contract.extension
    );
    let expected_url = format!(
        "https://github.com/hongjiadev/wokcore/releases/download/v{version}/{expected_file}"
    );
    if artifact.target != contract.target
        || artifact.file != expected_file
        || artifact.executable != contract.executable
        || artifact.size == 0
        || artifact.size > MAX_ARTIFACT_BYTES
        || !is_lower_hex_sha256(&artifact.sha256)
        || artifact.url != expected_url
    {
        return Err(WokCoreInstallError::InvalidManifest);
    }
    Ok(ReleaseArtifact {
        file: artifact.file,
        executable: artifact.executable,
        size: artifact.size,
        sha256: artifact.sha256,
        url: artifact.url,
    })
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) const fn current_target() -> &'static str {
    if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    schema_version: u32,
    product: String,
    api_major: u32,
    version: String,
    signing_key_id: String,
    artifacts: Vec<ArtifactDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactDocument {
    target: String,
    file: String,
    executable: String,
    size: u64,
    sha256: String,
    url: String,
}

#[derive(Clone, Copy)]
struct TargetContract {
    target: &'static str,
    extension: &'static str,
    executable: &'static str,
}

impl TargetContract {
    const fn new(target: &'static str, extension: &'static str, executable: &'static str) -> Self {
        Self {
            target,
            extension,
            executable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WokCoreInstallError, parse_manifest};

    #[test]
    fn manifest_parser_rejects_unknown_and_duplicate_members() {
        for malformed in [
            br#"{"schema_version":1,"product":"wokcore","api_major":1,"version":"1.2.3","signing_key_id":"0000000000000000","artifacts":[],"extra":true}"#.as_slice(),
            br#"{"schema_version":1,"product":"wokcore","api_major":1,"version":"1.2.3","version":"1.2.4","signing_key_id":"0000000000000000","artifacts":[]}"#.as_slice(),
        ] {
            assert!(matches!(
                parse_manifest(malformed),
                Err(WokCoreInstallError::InvalidManifest)
            ));
        }
    }
}
