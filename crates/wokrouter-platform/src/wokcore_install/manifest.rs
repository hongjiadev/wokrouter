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

const V1_TARGETS: [TargetContract; 5] = [
    TargetContract::legacy("x86_64-pc-windows-msvc", "zip", "wokcore.exe"),
    TargetContract::legacy("x86_64-apple-darwin", "tar.gz", "wokcore"),
    TargetContract::legacy("aarch64-apple-darwin", "tar.gz", "wokcore"),
    TargetContract::legacy("x86_64-unknown-linux-gnu", "tar.gz", "wokcore"),
    TargetContract::legacy("aarch64-unknown-linux-gnu", "tar.gz", "wokcore"),
];

const V2_TARGETS: [TargetContract; 6] = [
    TargetContract::friendly(
        "x86_64-pc-windows-msvc",
        "Windows",
        "x86_64",
        "zip",
        "wokcore.exe",
    ),
    TargetContract::friendly(
        "aarch64-pc-windows-msvc",
        "Windows",
        "arm64",
        "zip",
        "wokcore.exe",
    ),
    TargetContract::friendly(
        "x86_64-apple-darwin",
        "macOS",
        "x86_64",
        "tar.gz",
        "wokcore",
    ),
    TargetContract::friendly(
        "aarch64-apple-darwin",
        "macOS",
        "arm64",
        "tar.gz",
        "wokcore",
    ),
    TargetContract::friendly(
        "x86_64-unknown-linux-gnu",
        "Linux",
        "x86_64",
        "tar.gz",
        "wokcore",
    ),
    TargetContract::friendly(
        "aarch64-unknown-linux-gnu",
        "Linux",
        "arm64",
        "tar.gz",
        "wokcore",
    ),
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
    expected_schema_version: u32,
) -> Result<ReleaseCandidate, WokCoreInstallError> {
    if manifest_bytes.is_empty() || manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(WokCoreInstallError::InvalidManifest);
    }
    if signature_bytes.is_empty() || signature_bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(WokCoreInstallError::InvalidSignature);
    }
    let key_id = verify_signature(manifest_bytes, signature_bytes, public_key_text.as_bytes())?;
    let document = parse_manifest(manifest_bytes)?;
    if document.schema_version != expected_schema_version {
        return Err(WokCoreInstallError::IncompatibleManifest);
    }
    validate_document(document, &key_id, target)
}

pub(super) fn validate_public_key(public_key: &str) -> Result<(), WokCoreInstallError> {
    public_key_id(public_key).map(|_| ())
}

pub(super) fn is_release_file(version: &str, file: &str) -> bool {
    matches!(
        file,
        "wokcore-update-v1.json"
            | "wokcore-update-v1.json.minisig"
            | "wokcore-update-v2.json"
            | "wokcore-update-v2.json.minisig"
    ) || V1_TARGETS
        .iter()
        .copied()
        .any(|contract| legacy_file(contract, version) == file)
        || V2_TARGETS
            .iter()
            .copied()
            .any(|contract| friendly_file(contract, version) == file)
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
    let targets = match document.schema_version {
        1 => V1_TARGETS.as_slice(),
        2 => V2_TARGETS.as_slice(),
        _ => return Err(WokCoreInstallError::IncompatibleManifest),
    };
    if document.api_major != 1 {
        return Err(WokCoreInstallError::IncompatibleManifest);
    }
    if document.product != "wokcore"
        || document.signing_key_id != key_id
        || document.artifacts.len() != targets.len()
    {
        return Err(WokCoreInstallError::InvalidManifest);
    }
    let version =
        Version::parse(&document.version).map_err(|_| WokCoreInstallError::InvalidManifest)?;
    if document.version.len() > 128 || version.to_string() != document.version {
        return Err(WokCoreInstallError::InvalidManifest);
    }

    let mut selected = None;
    for (artifact, contract) in document.artifacts.into_iter().zip(targets.iter().copied()) {
        let validated = validate_artifact(
            artifact,
            contract,
            &document.version,
            document.schema_version,
        )?;
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
    schema_version: u32,
) -> Result<ReleaseArtifact, WokCoreInstallError> {
    let expected_file = if schema_version == 1 {
        legacy_file(contract, version)
    } else {
        friendly_file(contract, version)
    };
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

fn legacy_file(contract: TargetContract, version: &str) -> String {
    format!(
        "wokcore-v{version}-{}.{}",
        contract.target, contract.extension
    )
}

fn friendly_file(contract: TargetContract, version: &str) -> String {
    if contract.system == "Windows" {
        format!(
            "WokCore-v{version}-{}-{}-Portable.zip",
            contract.system, contract.architecture
        )
    } else {
        format!(
            "WokCore-v{version}-{}-{}.{}",
            contract.system, contract.architecture, contract.extension
        )
    }
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn target_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

pub(super) fn current_target() -> &'static str {
    target_for(std::env::consts::OS, std::env::consts::ARCH)
        .expect("release builds use a supported target")
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
    system: &'static str,
    architecture: &'static str,
    extension: &'static str,
    executable: &'static str,
}

impl TargetContract {
    const fn legacy(
        target: &'static str,
        extension: &'static str,
        executable: &'static str,
    ) -> Self {
        Self {
            target,
            system: "",
            architecture: "",
            extension,
            executable,
        }
    }

    const fn friendly(
        target: &'static str,
        system: &'static str,
        architecture: &'static str,
        extension: &'static str,
        executable: &'static str,
    ) -> Self {
        Self {
            target,
            system,
            architecture,
            extension,
            executable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WokCoreInstallError, parse_manifest, target_for, validate_document};

    const V1_MANIFEST: &[u8] =
        include_bytes!("../../tests/fixtures/wokcore-install/wokcore-update-v1.json");
    const V2_MANIFEST: &[u8] =
        include_bytes!("../../tests/fixtures/wokcore-install/wokcore-update-v2.json");

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

    #[test]
    fn v1_manifest_keeps_the_exact_legacy_target_triple_archive() {
        let document = parse_manifest(V1_MANIFEST).unwrap();

        let candidate =
            validate_document(document, "7E411BA469CB14B6", "x86_64-pc-windows-msvc").unwrap();

        assert_eq!(
            candidate.artifact.file,
            "wokcore-v1.2.3-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn v2_manifest_selects_the_friendly_windows_arm64_archive() {
        let document = parse_manifest(V2_MANIFEST).unwrap();

        let candidate =
            validate_document(document, "7E411BA469CB14B6", "aarch64-pc-windows-msvc").unwrap();

        assert_eq!(
            candidate.artifact.file,
            "WokCore-v1.2.3-Windows-arm64-Portable.zip"
        );
        assert_eq!(
            target_for("windows", "aarch64"),
            Some("aarch64-pc-windows-msvc")
        );
    }
}
