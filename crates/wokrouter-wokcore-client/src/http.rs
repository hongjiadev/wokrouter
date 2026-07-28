use std::time::Duration;

use futures::StreamExt;
use reqwest::{
    Client, Response, StatusCode,
    header::{CONTENT_LENGTH, HOST},
    redirect::Policy,
};
use serde::{Deserialize, de::DeserializeOwned};

use crate::{ClientError, discovery::ValidatedDiscovery};

const MAX_PUBLIC_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct WokCoreHttp {
    client: Client,
}

#[derive(Deserialize)]
pub(crate) struct HealthWire {
    pub status: String,
    pub instance_id: String,
}

#[derive(Deserialize)]
pub(crate) struct CapabilitiesWire {
    pub wokcore_version: String,
    pub management_api_major: u32,
    pub minimum_management_api_major: u32,
    pub maximum_management_api_major: u32,
    pub provider_protocols: Vec<String>,
    pub capabilities: Vec<String>,
    pub instance_id: String,
}

#[derive(Clone, Copy)]
pub(crate) enum HttpError {
    Transport,
    InvalidResponse,
}

impl WokCoreHttp {
    pub(crate) fn new() -> Result<Self, ClientError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(4)
            .build()
            .map_err(|_| ClientError::Initialization)?;
        Ok(Self { client })
    }

    pub(crate) async fn health(
        &self,
        discovery: &ValidatedDiscovery,
    ) -> Result<HealthWire, HttpError> {
        self.get_json(discovery, "/wokcore/v1/health").await
    }

    pub(crate) async fn capabilities(
        &self,
        discovery: &ValidatedDiscovery,
    ) -> Result<CapabilitiesWire, HttpError> {
        self.get_json(discovery, "/wokcore/v1/capabilities").await
    }

    async fn get_json<T>(&self, discovery: &ValidatedDiscovery, path: &str) -> Result<T, HttpError>
    where
        T: DeserializeOwned,
    {
        let url = discovery
            .base_url
            .join(path)
            .map_err(|_| HttpError::InvalidResponse)?;
        let response = self
            .client
            .get(url)
            .header(HOST, discovery.authority.as_str())
            .send()
            .await
            .map_err(classify_transport)?;
        read_json(response).await
    }
}

fn classify_transport(error: reqwest::Error) -> HttpError {
    if error.is_connect() || error.is_timeout() {
        HttpError::Transport
    } else {
        HttpError::InvalidResponse
    }
}

async fn read_json<T>(response: Response) -> Result<T, HttpError>
where
    T: DeserializeOwned,
{
    if response.status() != StatusCode::OK {
        return Err(HttpError::InvalidResponse);
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_PUBLIC_RESPONSE_BYTES)
    {
        return Err(HttpError::InvalidResponse);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| HttpError::InvalidResponse)?;
        if body.len().saturating_add(chunk.len()) > MAX_PUBLIC_RESPONSE_BYTES {
            return Err(HttpError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| HttpError::InvalidResponse)
}
