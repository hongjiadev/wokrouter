use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(windows)]
use std::io::SeekFrom;

use fs4::fs_std::FileExt;
use reqwest::{Response, header::LOCATION};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::{Builder, NamedTempFile};
use url::Url;

use crate::{AppPaths, discover_wokcore_executable};

use super::{
    WokCoreInstallError, WokCoreInstallOutcome, WokCoreInstallSource,
    manifest::{
        MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES, MAX_SIGNATURE_BYTES, ReleaseArtifact,
        ReleaseCandidate, current_target, is_release_file, verify_manifest,
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RELEASE_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;
const RELEASE_DOCUMENTS: [&str; 4] = ["LICENSE-APACHE", "LICENSE-MIT", "NOTICE.md", "README.md"];

pub async fn install_missing_wokcore(
    paths: &AppPaths,
    source: &WokCoreInstallSource,
) -> Result<WokCoreInstallOutcome, WokCoreInstallError> {
    if let Some(executable) = existing_wokcore(paths)? {
        return Ok(WokCoreInstallOutcome::AlreadyInstalled { executable });
    }

    prepare_directory(&paths.wokcore_install_dir)?;
    let record_directory = paths
        .wokcore_install_record
        .parent()
        .ok_or(WokCoreInstallError::UnsafeInstallLocation)?;
    prepare_directory(record_directory)?;
    let _lease = acquire_install_lease(&paths.wokcore_install_dir)?;

    if let Some(executable) = existing_wokcore(paths)? {
        return Ok(WokCoreInstallOutcome::AlreadyInstalled { executable });
    }

    let client = release_client()?;
    let release = fetch_release(&client, source).await?;
    let archive = download_artifact(
        &client,
        source,
        &release.artifact,
        &paths.wokcore_install_dir,
    )
    .await?;
    let executable = install_archive(
        &archive,
        &release.artifact,
        &paths.wokcore_install_dir,
        &paths.wokcore_install_record,
    )?;
    Ok(WokCoreInstallOutcome::Installed {
        version: release.version,
        executable,
    })
}

fn existing_wokcore(paths: &AppPaths) -> Result<Option<PathBuf>, WokCoreInstallError> {
    discover_wokcore_executable(&paths.wokcore_install_record)
        .map_err(|_| WokCoreInstallError::InvalidInstallState)
}

struct InstallLease {
    file: File,
}

impl Drop for InstallLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn acquire_install_lease(directory: &Path) -> Result<InstallLease, WokCoreInstallError> {
    let lock_path = directory.join(".wokcore-install.lock");
    if let Ok(metadata) = fs::symlink_metadata(&lock_path)
        && !safe_regular_file(&metadata)
    {
        return Err(WokCoreInstallError::UnsafeInstallLocation);
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    configure_no_follow(&mut options);
    let file = options
        .open(&lock_path)
        .map_err(|_| WokCoreInstallError::UnsafeInstallLocation)?;
    secure_install_file_permissions(&lock_path)?;
    match file.try_lock_exclusive() {
        Ok(true) => Ok(InstallLease { file }),
        Ok(false) => Err(WokCoreInstallError::InstallInProgress),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err(WokCoreInstallError::InstallInProgress)
        }
        Err(_) => Err(WokCoreInstallError::UnsafeInstallLocation),
    }
}

fn release_client() -> Result<reqwest::Client, WokCoreInstallError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|_| WokCoreInstallError::DownloadFailed)
}

async fn fetch_release(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
) -> Result<ReleaseCandidate, WokCoreInstallError> {
    let manifest_url = source
        .origin
        .join("wokcore-update-v2.json")
        .map_err(|_| WokCoreInstallError::InvalidSource)?;
    match fetch_document(client, source, manifest_url, MAX_MANIFEST_BYTES).await? {
        FetchDocument::Found(manifest) => {
            let signature_url = source
                .origin
                .join("wokcore-update-v2.json.minisig")
                .map_err(|_| WokCoreInstallError::InvalidSource)?;
            let signature =
                fetch_bounded(client, source, signature_url, MAX_SIGNATURE_BYTES).await?;
            verify_manifest(
                &manifest,
                &signature,
                &source.public_key,
                current_target(),
                2,
            )
        }
        FetchDocument::NotFound => fetch_v1_release(client, source).await,
    }
}

async fn fetch_v1_release(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
) -> Result<ReleaseCandidate, WokCoreInstallError> {
    let manifest_url = source
        .origin
        .join("wokcore-update-v1.json")
        .map_err(|_| WokCoreInstallError::InvalidSource)?;
    let signature_url = source
        .origin
        .join("wokcore-update-v1.json.minisig")
        .map_err(|_| WokCoreInstallError::InvalidSource)?;
    let manifest = fetch_bounded(client, source, manifest_url, MAX_MANIFEST_BYTES).await?;
    let signature = fetch_bounded(client, source, signature_url, MAX_SIGNATURE_BYTES).await?;
    verify_manifest(
        &manifest,
        &signature,
        &source.public_key,
        current_target(),
        1,
    )
}

enum FetchDocument {
    Found(Vec<u8>),
    NotFound,
}

async fn fetch_bounded(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
    url: Url,
    maximum_bytes: usize,
) -> Result<Vec<u8>, WokCoreInstallError> {
    match fetch_document(client, source, url, maximum_bytes).await? {
        FetchDocument::Found(body) => Ok(body),
        FetchDocument::NotFound => Err(WokCoreInstallError::DownloadFailed),
    }
}

async fn fetch_document(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
    url: Url,
    maximum_bytes: usize,
) -> Result<FetchDocument, WokCoreInstallError> {
    let mut response = send_release_request(client, source, url).await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(FetchDocument::NotFound);
    }
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(WokCoreInstallError::DownloadFailed);
    }
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(maximum_bytes);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| WokCoreInstallError::DownloadFailed)?
    {
        let next_length = body
            .len()
            .checked_add(chunk.len())
            .ok_or(WokCoreInstallError::DownloadFailed)?;
        if next_length > maximum_bytes {
            return Err(WokCoreInstallError::DownloadFailed);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(FetchDocument::Found(body))
}

async fn download_artifact(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
    artifact: &ReleaseArtifact,
    install_directory: &Path,
) -> Result<NamedTempFile, WokCoreInstallError> {
    if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
        return Err(WokCoreInstallError::ArtifactSizeMismatch);
    }
    let url = if source.production {
        Url::parse(&artifact.url).map_err(|_| WokCoreInstallError::InvalidManifest)?
    } else {
        source
            .origin
            .join(&artifact.file)
            .map_err(|_| WokCoreInstallError::InvalidSource)?
    };
    let mut response = send_release_request(client, source, url).await?;
    if !response.status().is_success() {
        return Err(WokCoreInstallError::DownloadFailed);
    }
    if response
        .content_length()
        .is_some_and(|length| length != artifact.size)
    {
        return Err(WokCoreInstallError::ArtifactSizeMismatch);
    }

    let mut staged = Builder::new()
        .prefix(".wokcore-install-download-")
        .tempfile_in(install_directory)
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;
    let mut received = 0_u64;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| WokCoreInstallError::DownloadFailed)?
    {
        received = received
            .checked_add(
                u64::try_from(chunk.len()).map_err(|_| WokCoreInstallError::DownloadFailed)?,
            )
            .ok_or(WokCoreInstallError::DownloadFailed)?;
        if received > artifact.size {
            return Err(WokCoreInstallError::ArtifactSizeMismatch);
        }
        staged
            .write_all(&chunk)
            .map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;
        hasher.update(&chunk);
    }
    if received != artifact.size {
        return Err(WokCoreInstallError::ArtifactSizeMismatch);
    }
    staged
        .flush()
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;
    if format!("{:x}", hasher.finalize()) != artifact.sha256 {
        return Err(WokCoreInstallError::ArtifactHashMismatch);
    }
    Ok(staged)
}

async fn send_release_request(
    client: &reqwest::Client,
    source: &WokCoreInstallSource,
    url: Url,
) -> Result<Response, WokCoreInstallError> {
    validate_initial_url(source, &url)?;
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|_| WokCoreInstallError::DownloadFailed)?;
    if !response.status().is_redirection() {
        return Ok(response);
    }
    if !source.production {
        return Err(WokCoreInstallError::DownloadFailed);
    }

    let redirect = redirect_location(&url, &response)?;
    if validate_release_asset_redirect(&redirect).is_ok() {
        return send_final_redirect(client, redirect).await;
    }
    validate_latest_release_redirect(&url, &redirect)?;
    let versioned = client
        .get(redirect.clone())
        .send()
        .await
        .map_err(|_| WokCoreInstallError::DownloadFailed)?;
    if !versioned.status().is_redirection() {
        return Ok(versioned);
    }
    let release_asset = redirect_location(&redirect, &versioned)?;
    validate_release_asset_redirect(&release_asset)?;
    send_final_redirect(client, release_asset).await
}

async fn send_final_redirect(
    client: &reqwest::Client,
    url: Url,
) -> Result<Response, WokCoreInstallError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| WokCoreInstallError::DownloadFailed)?;
    if response.status().is_redirection() {
        return Err(WokCoreInstallError::DownloadFailed);
    }
    Ok(response)
}

fn redirect_location(base: &Url, response: &Response) -> Result<Url, WokCoreInstallError> {
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(WokCoreInstallError::DownloadFailed)?;
    base.join(location)
        .map_err(|_| WokCoreInstallError::DownloadFailed)
}

fn validate_initial_url(
    source: &WokCoreInstallSource,
    url: &Url,
) -> Result<(), WokCoreInstallError> {
    if !source.production {
        let relative_path = url
            .path()
            .strip_prefix(source.origin.path())
            .ok_or(WokCoreInstallError::InvalidSource)?;
        return (url.origin() == source.origin.origin()
            && url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !relative_path.is_empty()
            && !relative_path.contains('/'))
        .then_some(())
        .ok_or(WokCoreInstallError::InvalidSource);
    }

    let valid_latest = matches!(
        url.path(),
        "/hongjiadev/wokcore/releases/latest/download/wokcore-update-v1.json"
            | "/hongjiadev/wokcore/releases/latest/download/wokcore-update-v1.json.minisig"
            | "/hongjiadev/wokcore/releases/latest/download/wokcore-update-v2.json"
            | "/hongjiadev/wokcore/releases/latest/download/wokcore-update-v2.json.minisig"
    );
    (url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && (valid_latest || validate_versioned_release_url(url).is_ok()))
    .then_some(())
    .ok_or(WokCoreInstallError::InvalidSource)
}

fn validate_versioned_release_url(url: &Url) -> Result<(), WokCoreInstallError> {
    const PREFIX: &str = "/hongjiadev/wokcore/releases/download/v";
    let versioned = url
        .path()
        .strip_prefix(PREFIX)
        .ok_or(WokCoreInstallError::InvalidSource)?;
    let (version, file) = versioned
        .split_once('/')
        .ok_or(WokCoreInstallError::InvalidSource)?;
    let parsed = semver::Version::parse(version).map_err(|_| WokCoreInstallError::InvalidSource)?;
    (url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && parsed.to_string() == version
        && is_release_file(version, file))
    .then_some(())
    .ok_or(WokCoreInstallError::InvalidSource)
}

fn validate_latest_release_redirect(
    initial: &Url,
    redirect: &Url,
) -> Result<(), WokCoreInstallError> {
    const LATEST_PREFIX: &str = "/hongjiadev/wokcore/releases/latest/download/";
    let expected_file = initial
        .path()
        .strip_prefix(LATEST_PREFIX)
        .ok_or(WokCoreInstallError::DownloadFailed)?;
    validate_versioned_release_url(redirect)?;
    (redirect
        .path()
        .rsplit_once('/')
        .is_some_and(|(_, file)| file == expected_file))
    .then_some(())
    .ok_or(WokCoreInstallError::DownloadFailed)
}

fn validate_release_asset_redirect(url: &Url) -> Result<(), WokCoreInstallError> {
    (url.scheme() == "https"
        && url.host_str() == Some("release-assets.githubusercontent.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path().starts_with("/github-production-release-asset/"))
    .then_some(())
    .ok_or(WokCoreInstallError::DownloadFailed)
}

fn install_archive(
    archive: &NamedTempFile,
    artifact: &ReleaseArtifact,
    install_directory: &Path,
    install_record: &Path,
) -> Result<PathBuf, WokCoreInstallError> {
    let mut candidate = Builder::new()
        .prefix(".wokcore-install-candidate-")
        .tempfile_in(install_directory)
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;
    extract_executable(archive.as_file(), artifact, candidate.as_file_mut())?;
    make_executable(candidate.path())?;
    candidate
        .flush()
        .and_then(|_| candidate.as_file().sync_all())
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;

    let target = install_directory.join(&artifact.executable);
    if fs::symlink_metadata(&target).is_ok() {
        return Err(WokCoreInstallError::InvalidInstallState);
    }
    publish_candidate(candidate.path(), &target, install_directory)?;

    finish_record_commit(
        write_install_record(install_record, &target),
        &target,
        install_directory,
    )?;
    Ok(target)
}

fn publish_candidate(
    candidate: &Path,
    target: &Path,
    install_directory: &Path,
) -> Result<(), WokCoreInstallError> {
    publish_candidate_with_sync(candidate, target, install_directory, sync_directory)
}

fn publish_candidate_with_sync(
    candidate: &Path,
    target: &Path,
    install_directory: &Path,
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> Result<(), WokCoreInstallError> {
    fs::hard_link(candidate, target).map_err(|_| WokCoreInstallError::AtomicInstallFailed)?;
    if sync(install_directory).is_err() {
        let _ = fs::remove_file(target);
        let _ = sync(install_directory);
        return Err(WokCoreInstallError::AtomicInstallFailed);
    }
    Ok(())
}

fn finish_record_commit(
    result: Result<(), RecordCommitError>,
    target: &Path,
    install_directory: &Path,
) -> Result<(), WokCoreInstallError> {
    match result {
        Ok(()) => Ok(()),
        Err(RecordCommitError::BeforeCommit) => {
            let _ = fs::remove_file(target);
            let _ = sync_directory(install_directory);
            Err(WokCoreInstallError::InstallRecordFailed)
        }
        Err(RecordCommitError::AfterCommit) => Err(WokCoreInstallError::InstallRecordFailed),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordCommitError {
    BeforeCommit,
    AfterCommit,
}

fn write_install_record(path: &Path, executable: &Path) -> Result<(), RecordCommitError> {
    write_install_record_with_sync(path, executable, sync_directory)
}

fn write_install_record_with_sync(
    path: &Path,
    executable: &Path,
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), RecordCommitError> {
    let directory = path.parent().ok_or(RecordCommitError::BeforeCommit)?;
    let mut temporary = Builder::new()
        .prefix(".wokcore-install-record-")
        .tempfile_in(directory)
        .map_err(|_| RecordCommitError::BeforeCommit)?;
    serde_json::to_writer(
        &mut temporary,
        &InstallRecord {
            schema_version: 1,
            executable,
        },
    )
    .map_err(|_| RecordCommitError::BeforeCommit)?;
    temporary
        .write_all(b"\n")
        .and_then(|_| temporary.flush())
        .map_err(|_| RecordCommitError::BeforeCommit)?;
    secure_record_permissions(temporary.path()).map_err(|_| RecordCommitError::BeforeCommit)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| RecordCommitError::BeforeCommit)?;
    temporary
        .persist_noclobber(path)
        .map_err(|_| RecordCommitError::BeforeCommit)?;
    sync(directory).map_err(|_| RecordCommitError::AfterCommit)
}

#[derive(Serialize)]
struct InstallRecord<'a> {
    schema_version: u32,
    executable: &'a Path,
}

#[cfg(windows)]
fn extract_executable(
    archive_file: &File,
    artifact: &ReleaseArtifact,
    destination: &mut File,
) -> Result<(), WokCoreInstallError> {
    let mut file = archive_file
        .try_clone()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    file.rewind()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    let declared_entries = zip_declared_entries(&mut file)?;
    file.rewind()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| WokCoreInstallError::InvalidArchive)?;
    if archive.len() != declared_entries {
        return Err(WokCoreInstallError::InvalidArchive);
    }
    let expected = artifact.executable.as_bytes();
    let mut found = false;
    let mut documents = [false; RELEASE_DOCUMENTS.len()];
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| WokCoreInstallError::InvalidArchive)?;
        if entry.name_raw() != expected {
            validate_zip_document(&entry, &mut documents)?;
            continue;
        }
        let size = entry.size();
        if found
            || entry.is_dir()
            || size == 0
            || size > MAX_ARTIFACT_BYTES
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(WokCoreInstallError::InvalidArchive);
        }
        let copied = io::copy(
            &mut entry.take(MAX_ARTIFACT_BYTES.saturating_add(1)),
            destination,
        )
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
        if copied != size {
            return Err(WokCoreInstallError::InvalidArchive);
        }
        found = true;
    }
    found
        .then_some(())
        .ok_or(WokCoreInstallError::InvalidArchive)
}

#[cfg(windows)]
fn zip_declared_entries(file: &mut File) -> Result<usize, WokCoreInstallError> {
    const END_RECORD_SIZE: usize = 22;
    const MAX_COMMENT_SIZE: usize = u16::MAX as usize;
    const SIGNATURE: &[u8; 4] = b"PK\x05\x06";

    let length = file
        .metadata()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?
        .len();
    let tail_length = length.min((END_RECORD_SIZE + MAX_COMMENT_SIZE) as u64) as usize;
    if tail_length < END_RECORD_SIZE {
        return Err(WokCoreInstallError::InvalidArchive);
    }
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut tail)
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    for index in (0..=tail.len() - END_RECORD_SIZE).rev() {
        if &tail[index..index + SIGNATURE.len()] != SIGNATURE {
            continue;
        }
        let read_u16 =
            |offset| u16::from_le_bytes([tail[index + offset], tail[index + offset + 1]]);
        let read_u32 = |offset| {
            u32::from_le_bytes([
                tail[index + offset],
                tail[index + offset + 1],
                tail[index + offset + 2],
                tail[index + offset + 3],
            ])
        };
        let comment_length = read_u16(20) as usize;
        let entries_on_disk = read_u16(8);
        let total_entries = read_u16(10);
        let central_size = read_u32(12) as u64;
        let central_offset = read_u32(16) as u64;
        let end_offset = length - tail_length as u64 + index as u64;
        if index + END_RECORD_SIZE + comment_length != tail.len()
            || read_u16(4) != 0
            || read_u16(6) != 0
            || entries_on_disk != total_entries
            || total_entries == 0
            || total_entries == u16::MAX
            || central_offset
                .checked_add(central_size)
                .is_none_or(|central_end| central_end != end_offset)
        {
            return Err(WokCoreInstallError::InvalidArchive);
        }
        return Ok(total_entries as usize);
    }
    Err(WokCoreInstallError::InvalidArchive)
}

#[cfg(windows)]
fn validate_zip_document(
    entry: &zip::read::ZipFile<'_, File>,
    documents: &mut [bool; RELEASE_DOCUMENTS.len()],
) -> Result<(), WokCoreInstallError> {
    let name =
        std::str::from_utf8(entry.name_raw()).map_err(|_| WokCoreInstallError::InvalidArchive)?;
    let document = RELEASE_DOCUMENTS
        .iter()
        .position(|expected| *expected == name)
        .ok_or(WokCoreInstallError::InvalidArchive)?;
    if documents[document]
        || entry.is_dir()
        || entry.size() > MAX_RELEASE_DOCUMENT_BYTES
        || entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err(WokCoreInstallError::InvalidArchive);
    }
    documents[document] = true;
    Ok(())
}

#[cfg(unix)]
fn extract_executable(
    archive_file: &File,
    artifact: &ReleaseArtifact,
    destination: &mut File,
) -> Result<(), WokCoreInstallError> {
    let mut file = archive_file
        .try_clone()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    file.rewind()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut found = false;
    let mut documents = [false; RELEASE_DOCUMENTS.len()];
    let entries = archive
        .entries()
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
    for entry in entries {
        let entry = entry.map_err(|_| WokCoreInstallError::InvalidArchive)?;
        let path = entry
            .path()
            .map_err(|_| WokCoreInstallError::InvalidArchive)?;
        if path.as_os_str() != artifact.executable.as_str() {
            let name = path.to_str().ok_or(WokCoreInstallError::InvalidArchive)?;
            let document = RELEASE_DOCUMENTS
                .iter()
                .position(|expected| *expected == name)
                .ok_or(WokCoreInstallError::InvalidArchive)?;
            if documents[document]
                || !entry.header().entry_type().is_file()
                || entry.size() > MAX_RELEASE_DOCUMENT_BYTES
            {
                return Err(WokCoreInstallError::InvalidArchive);
            }
            documents[document] = true;
            continue;
        }
        let size = entry.size();
        if found || !entry.header().entry_type().is_file() || size == 0 || size > MAX_ARTIFACT_BYTES
        {
            return Err(WokCoreInstallError::InvalidArchive);
        }
        let copied = io::copy(
            &mut entry.take(MAX_ARTIFACT_BYTES.saturating_add(1)),
            destination,
        )
        .map_err(|_| WokCoreInstallError::InvalidArchive)?;
        if copied != size {
            return Err(WokCoreInstallError::InvalidArchive);
        }
        found = true;
    }
    found
        .then_some(())
        .ok_or(WokCoreInstallError::InvalidArchive)
}

fn prepare_directory(path: &Path) -> Result<(), WokCoreInstallError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if safe_directory(&metadata) => {}
        Ok(_) => return Err(WokCoreInstallError::UnsafeInstallLocation),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| WokCoreInstallError::UnsafeInstallLocation)?;
        }
        Err(_) => return Err(WokCoreInstallError::UnsafeInstallLocation),
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| WokCoreInstallError::UnsafeInstallLocation)?;
    if !safe_directory(&metadata) {
        return Err(WokCoreInstallError::UnsafeInstallLocation);
    }
    secure_directory_permissions(path)
}

fn safe_regular_file(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file()
        && !metadata.file_type().is_symlink()
        && safe_platform_metadata(metadata)
}

fn safe_directory(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir()
        && !metadata.file_type().is_symlink()
        && safe_platform_metadata(metadata)
}

#[cfg(unix)]
fn safe_platform_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    metadata.uid() == unsafe { libc::geteuid() }
}

#[cfg(windows)]
fn safe_platform_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(any(unix, windows)))]
fn safe_platform_metadata(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn secure_directory_permissions(path: &Path) -> Result<(), WokCoreInstallError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| WokCoreInstallError::UnsafeInstallLocation)
}

#[cfg(windows)]
fn secure_directory_permissions(path: &Path) -> Result<(), WokCoreInstallError> {
    use crate::system::windows_security::{PrivatePathKind, secure_private_path};

    secure_private_path(path, PrivatePathKind::Directory)
        .map_err(|_| WokCoreInstallError::UnsafeInstallLocation)
}

#[cfg(not(any(unix, windows)))]
fn secure_directory_permissions(_path: &Path) -> Result<(), WokCoreInstallError> {
    Err(WokCoreInstallError::UnsafeInstallLocation)
}

#[cfg(unix)]
fn secure_record_permissions(path: &Path) -> Result<(), WokCoreInstallError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| WokCoreInstallError::InstallRecordFailed)
}

#[cfg(windows)]
fn secure_record_permissions(path: &Path) -> Result<(), WokCoreInstallError> {
    use crate::system::windows_security::{PrivatePathKind, secure_private_path};

    secure_private_path(path, PrivatePathKind::File)
        .map_err(|_| WokCoreInstallError::InstallRecordFailed)
}

#[cfg(not(any(unix, windows)))]
fn secure_record_permissions(_path: &Path) -> Result<(), WokCoreInstallError> {
    Err(WokCoreInstallError::InstallRecordFailed)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), WokCoreInstallError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)
}

#[cfg(windows)]
fn make_executable(path: &Path) -> Result<(), WokCoreInstallError> {
    use crate::system::windows_security::{PrivatePathKind, secure_private_path};

    secure_private_path(path, PrivatePathKind::File)
        .map_err(|_| WokCoreInstallError::AtomicInstallFailed)
}

#[cfg(not(any(unix, windows)))]
fn make_executable(_path: &Path) -> Result<(), WokCoreInstallError> {
    Err(WokCoreInstallError::AtomicInstallFailed)
}

#[cfg(windows)]
fn secure_install_file_permissions(path: &Path) -> Result<(), WokCoreInstallError> {
    use crate::system::windows_security::{PrivatePathKind, secure_private_path};

    secure_private_path(path, PrivatePathKind::File)
        .map_err(|_| WokCoreInstallError::UnsafeInstallLocation)
}

#[cfg(not(windows))]
fn secure_install_file_permissions(_path: &Path) -> Result<(), WokCoreInstallError> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
}

#[cfg(not(any(unix, windows)))]
fn configure_no_follow(_options: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, Cursor, Write},
    };

    use tempfile::tempdir;
    use url::Url;

    use super::{
        RecordCommitError, ReleaseArtifact, WokCoreInstallError, extract_executable,
        finish_record_commit, publish_candidate_with_sync, validate_latest_release_redirect,
        validate_release_asset_redirect, validate_versioned_release_url,
        write_install_record_with_sync,
    };

    #[test]
    fn production_redirects_accept_only_fixed_release_locations() {
        let latest = Url::parse(
            "https://github.com/hongjiadev/wokcore/releases/latest/download/wokcore-update-v1.json",
        )
        .unwrap();
        assert!(
            validate_latest_release_redirect(
                &latest,
                &Url::parse(
                    "https://github.com/hongjiadev/wokcore/releases/download/v1.2.3/wokcore-update-v1.json"
                )
                .unwrap()
            )
            .is_ok()
        );
        let latest_v2 = Url::parse(
            "https://github.com/hongjiadev/wokcore/releases/latest/download/wokcore-update-v2.json",
        )
        .unwrap();
        assert!(
            validate_latest_release_redirect(
                &latest_v2,
                &Url::parse(
                    "https://github.com/hongjiadev/wokcore/releases/download/v1.2.3/wokcore-update-v2.json"
                )
                .unwrap()
            )
            .is_ok()
        );
        assert!(
            validate_versioned_release_url(
                &Url::parse(
                    "https://github.com/hongjiadev/wokcore/releases/download/v1.2.3/WokCore-v1.2.3-Windows-arm64-Portable.zip"
                )
                .unwrap()
            )
            .is_ok()
        );
        assert!(
            validate_versioned_release_url(
                &Url::parse(
                    "https://github.com/hongjiadev/wokcore/releases/download/v1.2.3/arbitrary.zip"
                )
                .unwrap()
            )
            .is_err()
        );
        for rejected in [
            "https://github.com/hongjiadev/other/releases/download/v1.2.3/wokcore-update-v1.json",
            "https://github.com/hongjiadev/wokcore/releases/download/not-semver/wokcore-update-v1.json",
            "https://github.com/hongjiadev/wokcore/releases/download/v1.2.3/other.json",
            "https://user@github.com/hongjiadev/wokcore/releases/download/v1.2.3/wokcore-update-v1.json",
            "https://github.com:444/hongjiadev/wokcore/releases/download/v1.2.3/wokcore-update-v1.json",
        ] {
            assert!(
                validate_latest_release_redirect(&latest, &Url::parse(rejected).unwrap()).is_err(),
                "{rejected}"
            );
        }

        assert!(
            validate_release_asset_redirect(
                &Url::parse(
                    "https://release-assets.githubusercontent.com/github-production-release-asset/123/file.zip?sp=r"
                )
                .unwrap()
            )
            .is_ok()
        );
        for rejected in [
            "https://example.com/file.zip",
            "http://release-assets.githubusercontent.com/file.zip",
            "https://user@release-assets.githubusercontent.com/file.zip",
            "https://release-assets.githubusercontent.com:444/file.zip",
            "https://release-assets.githubusercontent.com/file.zip#fragment",
        ] {
            assert!(
                validate_release_asset_redirect(&Url::parse(rejected).unwrap()).is_err(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn a_post_commit_directory_sync_failure_keeps_the_record_and_executable() {
        let fixture = tempdir().unwrap();
        let executable = fixture.path().join("wokcore");
        let record = fixture.path().join("wokcore-install.json");
        fs::write(&executable, b"executable").unwrap();

        let error = write_install_record_with_sync(&record, &executable, |_| {
            Err(io::Error::other("synthetic directory sync failure"))
        })
        .unwrap_err();

        assert_eq!(error, RecordCommitError::AfterCommit);
        assert!(record.is_file());
        assert_eq!(
            finish_record_commit(Err(error), &executable, fixture.path()),
            Err(WokCoreInstallError::InstallRecordFailed)
        );
        assert!(executable.is_file());
    }

    #[test]
    fn a_pre_commit_record_failure_removes_the_unregistered_executable() {
        let fixture = tempdir().unwrap();
        let executable = fixture.path().join("wokcore");
        fs::write(&executable, b"executable").unwrap();

        assert_eq!(
            finish_record_commit(
                Err(RecordCommitError::BeforeCommit),
                &executable,
                fixture.path()
            ),
            Err(WokCoreInstallError::InstallRecordFailed)
        );
        assert!(!executable.exists());
    }

    #[test]
    fn a_publish_sync_failure_removes_the_unregistered_executable() {
        let fixture = tempdir().unwrap();
        let candidate = fixture.path().join("candidate");
        let executable = fixture.path().join("wokcore");
        fs::write(&candidate, b"executable").unwrap();

        assert_eq!(
            publish_candidate_with_sync(&candidate, &executable, fixture.path(), |_| {
                Err(io::Error::other("synthetic directory sync failure"))
            }),
            Err(WokCoreInstallError::AtomicInstallFailed)
        );
        assert!(!executable.exists());
    }

    #[cfg(windows)]
    #[test]
    fn zip_rejects_traversal_absolute_nested_duplicate_and_symlink_entries() {
        use zip::{ZipWriter, write::SimpleFileOptions};

        fn zip(entries: &[(&str, bool)]) -> Vec<u8> {
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            for (name, symlink) in entries {
                if *symlink {
                    writer
                        .add_symlink(*name, "wokcore.exe", SimpleFileOptions::default())
                        .unwrap();
                } else {
                    writer
                        .start_file(*name, SimpleFileOptions::default())
                        .unwrap();
                    writer.write_all(b"executable").unwrap();
                }
            }
            writer.finish().unwrap().into_inner()
        }

        let mut duplicate = zip(&[("wokcore.exe", false), ("wokcore.exf", false)]);
        let original = b"wokcore.exf";
        let replacement = b"wokcore.exe";
        for index in 0..=duplicate.len() - original.len() {
            if duplicate[index..].starts_with(original) {
                duplicate[index..index + replacement.len()].copy_from_slice(replacement);
            }
        }
        for (case, archive) in [
            ("traversal", zip(&[("../wokcore.exe", false)])),
            ("absolute", zip(&[("/wokcore.exe", false)])),
            ("nested", zip(&[("nested/wokcore.exe", false)])),
            ("duplicate", duplicate),
            ("symlink", zip(&[("wokcore.exe", true)])),
        ] {
            assert_invalid_archive(&archive, case);
        }
    }

    #[cfg(unix)]
    #[test]
    fn tar_rejects_traversal_absolute_nested_duplicate_symlink_and_hardlink_entries() {
        use flate2::{Compression, write::GzEncoder};
        use tar::{Builder, EntryType, Header};

        fn tar(entries: &[(&[u8], EntryType)]) -> Vec<u8> {
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = Builder::new(encoder);
            for (name, entry_type) in entries {
                let mut header = Header::new_gnu();
                header.set_mode(0o700);
                header.set_entry_type(*entry_type);
                let data = if entry_type.is_file() {
                    b"executable".as_slice()
                } else {
                    &[]
                };
                header.set_size(data.len() as u64);
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    header.set_link_name("wokcore").unwrap();
                }
                header.as_mut_bytes()[..name.len()].copy_from_slice(name);
                header.set_cksum();
                builder.append(&header, Cursor::new(data)).unwrap();
            }
            let encoder = builder.into_inner().unwrap();
            encoder.finish().unwrap()
        }

        for (case, archive) in [
            ("traversal", tar(&[(b"../wokcore", EntryType::Regular)])),
            ("absolute", tar(&[(b"/wokcore", EntryType::Regular)])),
            ("nested", tar(&[(b"nested/wokcore", EntryType::Regular)])),
            (
                "duplicate",
                tar(&[
                    (b"wokcore", EntryType::Regular),
                    (b"wokcore", EntryType::Regular),
                ]),
            ),
            ("symlink", tar(&[(b"wokcore", EntryType::Symlink)])),
            ("hardlink", tar(&[(b"wokcore", EntryType::Link)])),
        ] {
            assert_invalid_archive(&archive, case);
        }
    }

    fn assert_invalid_archive(bytes: &[u8], case: &str) {
        let mut archive = tempfile::NamedTempFile::new().unwrap();
        archive.write_all(bytes).unwrap();
        archive.flush().unwrap();
        let mut destination = tempfile::NamedTempFile::new().unwrap();

        assert_eq!(
            extract_executable(
                archive.as_file(),
                &test_artifact(),
                destination.as_file_mut()
            ),
            Err(WokCoreInstallError::InvalidArchive),
            "{case}"
        );
    }

    fn test_artifact() -> ReleaseArtifact {
        ReleaseArtifact {
            file: format!("wokcore.{}", if cfg!(windows) { "zip" } else { "tar.gz" }),
            executable: format!("wokcore{}", std::env::consts::EXE_SUFFIX),
            size: 1,
            sha256: "0".repeat(64),
            url: "https://example.invalid/wokcore".to_owned(),
        }
    }
}
