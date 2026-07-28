use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use reqwest::{
    Body, Client, Method, RequestBuilder, Response, StatusCode,
    header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, HeaderValue},
    redirect::Policy,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

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

#[derive(Clone, Copy)]
pub(crate) enum ProtectedHttpError {
    Transport,
    Unauthorized,
    Forbidden,
    Conflict,
    InvalidRequest,
    InvalidResponse,
}

#[derive(Clone, Copy)]
pub(crate) struct ProtectedJsonOptions {
    pub(crate) request_timeout: Duration,
    pub(crate) max_response_bytes: usize,
    pub(crate) expected_status: StatusCode,
}

pub(crate) struct SecretJsonBody(Zeroizing<Vec<u8>>);

impl SecretJsonBody {
    pub(crate) fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for SecretJsonBody {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
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

    pub(crate) async fn protected_json<T>(
        &self,
        discovery: &ValidatedDiscovery,
        method: Method,
        path: &str,
        token: &SecretString,
        request_timeout: Duration,
    ) -> Result<T, ProtectedHttpError>
    where
        T: DeserializeOwned,
    {
        let request =
            self.protected_request(discovery, method, path, &[], token, request_timeout)?;
        send_protected_json(request, StatusCode::OK, MAX_PUBLIC_RESPONSE_BYTES).await
    }

    pub(crate) async fn protected_json_query<T>(
        &self,
        discovery: &ValidatedDiscovery,
        path: &str,
        query: &[(String, String)],
        token: &SecretString,
        request_timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<T, ProtectedHttpError>
    where
        T: DeserializeOwned,
    {
        let request =
            self.protected_request(discovery, Method::GET, path, query, token, request_timeout)?;
        send_protected_json(request, StatusCode::OK, max_response_bytes).await
    }

    pub(crate) async fn protected_json_no_body<T>(
        &self,
        discovery: &ValidatedDiscovery,
        method: Method,
        path: &str,
        token: &SecretString,
        options: ProtectedJsonOptions,
    ) -> Result<T, ProtectedHttpError>
    where
        T: DeserializeOwned,
    {
        let request =
            self.protected_request(discovery, method, path, &[], token, options.request_timeout)?;
        send_protected_json(request, options.expected_status, options.max_response_bytes).await
    }

    pub(crate) async fn protected_json_body<T, B>(
        &self,
        discovery: &ValidatedDiscovery,
        method: Method,
        path: &str,
        token: &SecretString,
        options: ProtectedJsonOptions,
        body: &B,
    ) -> Result<T, ProtectedHttpError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let request =
            self.protected_request(discovery, method, path, &[], token, options.request_timeout)?;
        send_protected_json(
            request.json(body),
            options.expected_status,
            options.max_response_bytes,
        )
        .await
    }

    pub(crate) async fn protected_secret_json<T>(
        &self,
        discovery: &ValidatedDiscovery,
        method: Method,
        path: &str,
        token: &SecretString,
        options: ProtectedJsonOptions,
        body: Zeroizing<Vec<u8>>,
    ) -> Result<T, ProtectedHttpError>
    where
        T: DeserializeOwned,
    {
        let request =
            self.protected_request(discovery, method, path, &[], token, options.request_timeout)?;
        let body = Body::from(Bytes::from_owner(SecretJsonBody::new(body)));
        send_protected_json(
            request.header(CONTENT_TYPE, "application/json").body(body),
            options.expected_status,
            options.max_response_bytes,
        )
        .await
    }

    pub(crate) async fn protected_bytes_query(
        &self,
        discovery: &ValidatedDiscovery,
        path: &str,
        query: &[(String, String)],
        token: &SecretString,
        request_timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, ProtectedHttpError> {
        let request =
            self.protected_request(discovery, Method::GET, path, query, token, request_timeout)?;
        let response = send_protected(request, StatusCode::OK).await?;
        read_bounded(response, max_response_bytes)
            .await
            .map(Zeroizing::new)
    }

    fn protected_request(
        &self,
        discovery: &ValidatedDiscovery,
        method: Method,
        path: &str,
        query: &[(String, String)],
        token: &SecretString,
        request_timeout: Duration,
    ) -> Result<RequestBuilder, ProtectedHttpError> {
        let mut url = discovery
            .base_url
            .join(path)
            .map_err(|_| ProtectedHttpError::InvalidResponse)?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(name, value)| (name, value)));
        }
        let mut authorization = Zeroizing::new(Vec::with_capacity(
            "Bearer ".len() + token.expose_secret().len(),
        ));
        authorization.extend_from_slice(b"Bearer ");
        authorization.extend_from_slice(token.expose_secret().as_bytes());
        let mut authorization = HeaderValue::from_bytes(&authorization)
            .map_err(|_| ProtectedHttpError::InvalidResponse)?;
        authorization.set_sensitive(true);

        Ok(self
            .client
            .request(method, url)
            .header(HOST, discovery.authority.as_str())
            .header(AUTHORIZATION, authorization)
            .timeout(request_timeout))
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

fn classify_protected_transport(error: reqwest::Error) -> ProtectedHttpError {
    if error.is_connect() || error.is_timeout() {
        ProtectedHttpError::Transport
    } else {
        ProtectedHttpError::InvalidResponse
    }
}

async fn read_json<T>(response: Response) -> Result<T, HttpError>
where
    T: DeserializeOwned,
{
    if response.status() != StatusCode::OK {
        return Err(HttpError::InvalidResponse);
    }
    let body = read_bounded_public(response, MAX_PUBLIC_RESPONSE_BYTES).await?;
    serde_json::from_slice(&body).map_err(|_| HttpError::InvalidResponse)
}

async fn send_protected_json<T>(
    request: RequestBuilder,
    expected_status: StatusCode,
    max_response_bytes: usize,
) -> Result<T, ProtectedHttpError>
where
    T: DeserializeOwned,
{
    let response = send_protected(request, expected_status).await?;
    let body = read_bounded(response, max_response_bytes).await?;
    serde_json::from_slice(&body).map_err(|_| ProtectedHttpError::InvalidResponse)
}

async fn send_protected(
    request: RequestBuilder,
    expected_status: StatusCode,
) -> Result<Response, ProtectedHttpError> {
    let response = request.send().await.map_err(classify_protected_transport)?;
    match response.status() {
        StatusCode::UNAUTHORIZED => Err(ProtectedHttpError::Unauthorized),
        StatusCode::FORBIDDEN => Err(ProtectedHttpError::Forbidden),
        StatusCode::CONFLICT => Err(ProtectedHttpError::Conflict),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            Err(ProtectedHttpError::InvalidRequest)
        }
        status if status == expected_status => Ok(response),
        _ => Err(ProtectedHttpError::InvalidResponse),
    }
}

async fn read_bounded(
    response: Response,
    max_response_bytes: usize,
) -> Result<Vec<u8>, ProtectedHttpError> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max_response_bytes)
    {
        return Err(ProtectedHttpError::InvalidResponse);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProtectedHttpError::InvalidResponse)?;
        if body.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(ProtectedHttpError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_bounded_public(
    response: Response,
    max_response_bytes: usize,
) -> Result<Vec<u8>, HttpError> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max_response_bytes)
    {
        return Err(HttpError::InvalidResponse);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| HttpError::InvalidResponse)?;
        if body.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(HttpError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
