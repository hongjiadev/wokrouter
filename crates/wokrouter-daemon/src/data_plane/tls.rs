use std::{
    fs::{self, File},
    io::BufReader,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use wokrouter_core::secret::SecretRef;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsConfig {
    certificate_path: PathBuf,
    private_key_path: PathBuf,
}

impl TlsConfig {
    pub fn new(certificate_path: impl Into<PathBuf>, private_key_path: impl Into<PathBuf>) -> Self {
        Self {
            certificate_path: certificate_path.into(),
            private_key_path: private_key_path.into(),
        }
    }

    pub fn certificate_path(&self) -> &Path {
        &self.certificate_path
    }

    pub fn private_key_path(&self) -> &Path {
        &self.private_key_path
    }

    pub fn validate(&self) -> Result<ValidatedTlsConfig, TlsConfigError> {
        let certificate_path = validate_file(&self.certificate_path, TlsFileKind::Certificate)?;
        let private_key_path = validate_file(&self.private_key_path, TlsFileKind::PrivateKey)?;
        if certificate_path == private_key_path {
            return Err(TlsConfigError::SameFile);
        }

        let certificate_file = File::open(&certificate_path)
            .map_err(|_| TlsConfigError::Unreadable(TlsFileKind::Certificate))?;
        let certificates = rustls_pemfile::certs(&mut BufReader::new(certificate_file))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TlsConfigError::InvalidPem(TlsFileKind::Certificate))?;
        if certificates.is_empty() {
            return Err(TlsConfigError::MissingPemItem(TlsFileKind::Certificate));
        }

        let private_key_file = File::open(&private_key_path)
            .map_err(|_| TlsConfigError::Unreadable(TlsFileKind::PrivateKey))?;
        let private_key = rustls_pemfile::private_key(&mut BufReader::new(private_key_file))
            .map_err(|_| TlsConfigError::InvalidPem(TlsFileKind::PrivateKey))?
            .ok_or(TlsConfigError::MissingPemItem(TlsFileKind::PrivateKey))?;

        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|_| TlsConfigError::InvalidCertificateOrKey)?;

        Ok(ValidatedTlsConfig {
            certificate_path,
            private_key_path,
            server_config: Arc::new(server_config),
        })
    }
}

fn validate_file(path: &Path, kind: TlsFileKind) -> Result<PathBuf, TlsConfigError> {
    let canonical = fs::canonicalize(path).map_err(|_| TlsConfigError::Unreadable(kind))?;
    let metadata = fs::metadata(&canonical).map_err(|_| TlsConfigError::Unreadable(kind))?;
    if !metadata.is_file() {
        return Err(TlsConfigError::NotAFile(kind));
    }
    Ok(canonical)
}

#[derive(Clone)]
pub struct ValidatedTlsConfig {
    certificate_path: PathBuf,
    private_key_path: PathBuf,
    server_config: Arc<rustls::ServerConfig>,
}

impl ValidatedTlsConfig {
    pub fn certificate_path(&self) -> &Path {
        &self.certificate_path
    }

    pub fn private_key_path(&self) -> &Path {
        &self.private_key_path
    }

    pub fn rustls_config(&self) -> Arc<rustls::ServerConfig> {
        Arc::clone(&self.server_config)
    }
}

impl std::fmt::Debug for ValidatedTlsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedTlsConfig")
            .field("certificate_path", &self.certificate_path)
            .field("private_key_path", &self.private_key_path)
            .field("server_config", &"[configured]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsFileKind {
    Certificate,
    PrivateKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TlsConfigError {
    #[error("TLS certificate and private key must use different files")]
    SameFile,
    #[error("TLS {0:?} file is missing or unreadable")]
    Unreadable(TlsFileKind),
    #[error("TLS {0:?} path is not a regular file")]
    NotAFile(TlsFileKind),
    #[error("TLS {0:?} file is not valid PEM")]
    InvalidPem(TlsFileKind),
    #[error("TLS {0:?} PEM file does not contain the required item")]
    MissingPemItem(TlsFileKind),
    #[error("TLS certificate or private key is not usable")]
    InvalidCertificateOrKey,
}

pub struct ListenerSecurity;

impl ListenerSecurity {
    pub fn validate(
        bind_addr: SocketAddr,
        bearer_ref: Option<&SecretRef>,
        tls_config: Option<&TlsConfig>,
        insecure_private_lan_ack: bool,
    ) -> Result<ValidatedListenerSecurity, ListenerSecurityError> {
        let ip = bind_addr.ip();
        if ip.is_loopback() {
            return Ok(ValidatedListenerSecurity {
                tls: tls_config.map(TlsConfig::validate).transpose()?,
            });
        }
        if !is_private_lan(ip) {
            return Err(ListenerSecurityError::UnsupportedAddress);
        }
        if bearer_ref.is_none() {
            return Err(ListenerSecurityError::BearerRequired);
        }

        let tls = tls_config.map(TlsConfig::validate).transpose()?;
        if tls.is_none() && !insecure_private_lan_ack {
            return Err(ListenerSecurityError::TransportDecisionRequired);
        }

        Ok(ValidatedListenerSecurity { tls })
    }
}

fn is_private_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => is_unique_local(ip),
    }
}

fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

#[derive(Clone, Debug)]
pub struct ValidatedListenerSecurity {
    tls: Option<ValidatedTlsConfig>,
}

impl ValidatedListenerSecurity {
    pub fn rustls_config(&self) -> Option<Arc<rustls::ServerConfig>> {
        self.tls.as_ref().map(ValidatedTlsConfig::rustls_config)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ListenerSecurityError {
    #[error("only loopback and private LAN listener addresses are supported")]
    UnsupportedAddress,
    #[error("private LAN listeners require a bearer token reference")]
    BearerRequired,
    #[error("private LAN listeners require TLS or an explicit insecure transport acknowledgement")]
    TransportDecisionRequired,
    #[error(transparent)]
    Tls(#[from] TlsConfigError),
}
