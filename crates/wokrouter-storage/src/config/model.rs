#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub ui: UiConfig,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VersionedConfig {
    pub revision: u64,
    #[serde(flatten)]
    pub config: AppConfig,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ServerConfig {
    pub host: std::net::IpAddr,
    pub port: u16,
    pub allow_insecure_private_lan: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UiConfig {
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                port: 10101,
                allow_insecure_private_lan: false,
            },
            ui: UiConfig {
                locale_override: None,
                timezone_override: None,
            },
        }
    }
}
