use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::{Client, Method};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use url::Url;

const CONDITIONAL_CACHE_LIMIT: usize = 128;
type ConditionalCache = Arc<Mutex<HashMap<String, (String, Value)>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOptions {
    pub timeout: Duration,
    /// Validates HTTPS certificates when enabled. Defaults to `true`.
    ///
    /// Disabling this accepts invalid certificates and permits man-in-the-middle
    /// attacks. Use only for controlled development or test environments.
    pub reject_unauthorized: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            reject_unauthorized: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAction {
    Open,
    Close,
    Stop,
}

#[derive(Clone)]
pub struct RemoteApiClient {
    base_url: Url,
    client: Client,
    options: ClientOptions,
    conditional_cache: ConditionalCache,
}

impl RemoteApiClient {
    pub fn new(base_url: &str) -> Result<Self, String> {
        Self::with_options(base_url, ClientOptions::default())
    }

    pub fn with_options(base_url: &str, options: ClientOptions) -> Result<Self, String> {
        let mut base_url =
            Url::parse(base_url).map_err(|error| format!("Invalid base URL: {error}"))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err("Base URL must use HTTP or HTTPS.".into());
        }
        if base_url.cannot_be_a_base() {
            return Err("Base URL must support path segments.".into());
        }
        if base_url.scheme() == "https" && !options.reject_unauthorized {
            tracing::warn!("HTTPS certificate validation is disabled for the Somaris API client.");
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let mut builder = Client::builder().timeout(options.timeout);
        if !options.reject_unauthorized {
            builder = builder.danger_accept_invalid_certs(true);
        }
        let client = builder
            .build()
            .map_err(|error| format!("Failed to create API client: {error}"))?;
        Ok(Self {
            base_url,
            client,
            options,
            conditional_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn options(&self) -> ClientOptions {
        self.options
    }

    pub async fn get_device_status(&self, device_id: &str) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.device_url("api/v1/devices", device_id)?,
            None,
            "Failed to get device status",
        )
        .await
    }

    pub async fn get_device_list(&self) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.endpoint_url(&["api", "v1", "devices"])?,
            None,
            "Failed to get device list",
        )
        .await
    }

    pub async fn get_diagnostics(&self) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.endpoint_url(&["api", "v1", "diagnostics"])?,
            None,
            "Failed to get diagnostics",
        )
        .await
    }

    pub async fn get_managed_devices(&self) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.endpoint_url(&["api", "control", "v1", "devices"])?,
            None,
            "Failed to get managed devices",
        )
        .await
    }

    pub async fn enroll_device(
        &self,
        observed_device_id: &str,
        label: &str,
    ) -> Result<Value, String> {
        validate_device_id(observed_device_id)?;
        self.send_json(
            Method::POST,
            self.endpoint_url(&["api", "control", "v1", "devices"])?,
            Some(json!({
                "observed_device_id": observed_device_id,
                "label": label,
            })),
            "Failed to enroll device",
        )
        .await
    }

    pub async fn remove_enrollment(&self, managed_device_id: &str) -> Result<Value, String> {
        self.send_json(
            Method::DELETE,
            self.device_url("api/control/v1/devices", managed_device_id)?,
            None,
            "Failed to remove enrollment",
        )
        .await
    }

    pub async fn submit_command(
        &self,
        managed_device_id: &str,
        action: RemoteAction,
        idempotency_key: &str,
    ) -> Result<Value, String> {
        self.send_command(
            managed_device_id,
            "commands",
            idempotency_key,
            Some(json!({ "action": action })),
            "Failed to submit command",
        )
        .await
    }

    pub async fn submit_toggle(
        &self,
        managed_device_id: &str,
        idempotency_key: &str,
    ) -> Result<Value, String> {
        self.send_command(
            managed_device_id,
            "toggle",
            idempotency_key,
            None,
            "Failed to submit toggle",
        )
        .await
    }

    pub async fn get_command(&self, command_id: &str) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.device_url("api/control/v1/commands", command_id)?,
            None,
            "Failed to get command",
        )
        .await
    }

    async fn send_command(
        &self,
        managed_device_id: &str,
        endpoint: &str,
        idempotency_key: &str,
        body: Option<Value>,
        context: &str,
    ) -> Result<Value, String> {
        validate_device_id(managed_device_id)?;
        validate_idempotency_key(idempotency_key)?;
        let url = self.device_url("api/control/v1/devices", managed_device_id)?;
        let url = append_segment(url, endpoint)?;
        let mut request = self
            .client
            .post(url)
            .header("Idempotency-Key", idempotency_key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.send_request(request, context).await
    }

    fn device_url(&self, prefix: &str, device_id: &str) -> Result<Url, String> {
        validate_device_id(device_id)?;
        let mut url = self.endpoint_url(&prefix.split('/').collect::<Vec<_>>())?;
        url = append_segment(url, device_id)?;
        Ok(url)
    }

    fn endpoint_url(&self, segments: &[&str]) -> Result<Url, String> {
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| "Base URL does not support path segments.".to_owned())?;
            path.pop_if_empty();
            path.extend(segments.iter().copied());
        }
        Ok(url)
    }

    async fn send_json(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
        context: &str,
    ) -> Result<Value, String> {
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.send_request(request, context).await
    }

    async fn send_request(
        &self,
        request: reqwest::RequestBuilder,
        context: &str,
    ) -> Result<Value, String> {
        let mut request = request
            .build()
            .map_err(|error| format!("{context}: {error}"))?;
        let is_get = request.method() == Method::GET;
        let cache_key = request.url().as_str().to_owned();
        if is_get
            && let Some(etag) = self
                .conditional_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|(etag, _)| etag.clone())
        {
            request.headers_mut().insert(
                reqwest::header::IF_NONE_MATCH,
                etag.parse()
                    .map_err(|error| format!("{context}: {error}"))?,
            );
        }
        let response = self
            .client
            .execute(request)
            .await
            .map_err(|error| format!("{context}: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return self
                .conditional_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("{context}: received 304 without a cached response"));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let value = parse_response(response, context).await?;
        if is_get {
            let mut cache = self.conditional_cache.lock().await;
            if let Some(etag) = etag {
                if cache.len() >= CONDITIONAL_CACHE_LIMIT && !cache.contains_key(&cache_key) {
                    cache.clear();
                }
                cache.insert(cache_key, (etag, value.clone()));
            } else {
                cache.remove(&cache_key);
            }
        }
        Ok(value)
    }
}

async fn parse_response(response: reqwest::Response, context: &str) -> Result<Value, String> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("{context}: {error}"))?;
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&bytes);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            format!("{context}: HTTP {status}")
        } else {
            format!("{context}: HTTP {status}: {detail}")
        });
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{context}: invalid JSON response: {error}"))
}

fn append_segment(mut url: Url, segment: &str) -> Result<Url, String> {
    url.path_segments_mut()
        .map_err(|_| "Base URL does not support path segments.".to_owned())?
        .push(segment);
    Ok(url)
}

pub fn validate_device_id(device_id: &str) -> Result<(), String> {
    let trimmed = device_id.trim();
    if trimmed.is_empty()
        || trimmed != device_id
        || device_id.contains("..")
        || device_id.contains(['/', '\\', '?', '#'])
        || device_id.chars().any(char::is_control)
    {
        return Err("Invalid deviceId: must be a non-empty path-safe identifier.".into());
    }
    Ok(())
}

fn validate_idempotency_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() || key != key.trim() || key.chars().count() > 128 {
        return Err(
            "Invalid Idempotency-Key: must contain 1 to 128 non-whitespace characters.".into(),
        );
    }
    Ok(())
}
