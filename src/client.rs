use std::time::Duration;

use reqwest::{Client, Method};
use serde_json::{Value, json};
use url::Url;

use crate::telemetry::normalize_reason_codes;

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

#[derive(Clone)]
pub struct RemoteApiClient {
    base_url: Url,
    client: Client,
    options: ClientOptions,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PositionInput {
    Number(f64),
    String(String),
    Other(String),
}

impl From<f64> for PositionInput {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i32> for PositionInput {
    fn from(value: i32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<String> for PositionInput {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for PositionInput {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<Value> for PositionInput {
    fn from(value: Value) -> Self {
        match value {
            Value::Number(number) => number
                .as_f64()
                .map(Self::Number)
                .unwrap_or_else(|| Self::Other(number.to_string())),
            Value::String(value) => Self::String(value),
            other => Self::Other(other.to_string()),
        }
    }
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
        })
    }

    pub fn options(&self) -> ClientOptions {
        self.options
    }

    pub async fn get_device_status(&self, device_id: &str) -> Result<Value, String> {
        let url = self.device_url(device_id, &["status"])?;
        self.send_json(Method::GET, url, None, "Failed to get device status")
            .await
    }

    pub async fn set_device_position(
        &self,
        device_id: &str,
        position: impl Into<PositionInput>,
    ) -> Result<Value, String> {
        let url = self.device_url(device_id, &["position"])?;
        let position = validate_position(position.into())?;
        self.send_json(
            Method::PUT,
            url,
            Some(json!({ "position": position })),
            "Failed to set device position",
        )
        .await
    }

    pub async fn get_device_list(&self) -> Result<Value, String> {
        self.send_json(
            Method::GET,
            self.endpoint_url(&["devices"])?,
            None,
            "Failed to get device list",
        )
        .await
    }

    pub async fn get_device_diagnostics(&self, device_id: &str) -> Result<Value, String> {
        let url = self.device_url(device_id, &["diagnostics"])?;
        let value = self
            .send_json(Method::GET, url, None, "Failed to get device diagnostics")
            .await?;
        Ok(normalize_diagnostics(&value))
    }

    pub async fn get_connect_metadata(&self, device_id: &str) -> Result<Value, String> {
        let url = self.device_url(device_id, &["connect-metadata"])?;
        let value = self
            .send_json(Method::GET, url, None, "Failed to get connect metadata")
            .await?;
        Ok(normalize_diagnostics(&value))
    }

    fn device_url(&self, device_id: &str, suffix: &[&str]) -> Result<Url, String> {
        validate_device_id(device_id)?;
        let mut segments = vec!["device", device_id];
        segments.extend_from_slice(suffix);
        self.endpoint_url(&segments)
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
        let response = request
            .send()
            .await
            .map_err(|error| format!("{context}: {error}"))?;
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
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("{context}: invalid JSON response: {error}"))
    }
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

fn validate_position(value: PositionInput) -> Result<f64, String> {
    let number = match value {
        PositionInput::Number(value) => value,
        PositionInput::String(value) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| invalid_position(&value))?,
        PositionInput::Other(value) => return Err(invalid_position(&value)),
    };
    if !number.is_finite() {
        return Err(invalid_position(&format_number(number)));
    }
    Ok(number.clamp(0.0, 100.0))
}

fn invalid_position(value: &str) -> String {
    format!(
        "Failed to set device position: Invalid position value \"{value}\". Position must be a finite number."
    )
}

fn format_number(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value == f64::INFINITY {
        "Infinity".into()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        value.to_string()
    }
}

fn normalize_diagnostics(input: &Value) -> Value {
    let reasons = normalize_reason_codes(input);
    let sun = input.get("sun").unwrap_or(&Value::Null);
    let sun_times = input.get("sun_times").unwrap_or(&Value::Null);
    json!({
        "inputVoltageMv": find_integer(input, &["inputVoltageMv", "input_voltage_mv"]),
        "batteryVoltageMv": find_integer(input, &["batteryVoltageMv", "battery_voltage_mv", "battery_voltage"]),
        "bootCount": find_integer(input, &["bootCount", "boot_count"]),
        "undervoltageCount": find_integer(input, &["undervoltageCount", "undervoltage_count"]),
        "restartReasonCode": reasons.restart_reason_code,
        "restartReasonLabel": reasons.restart_reason_label,
        "motorStopReasonCode": reasons.motor_stop_reason_code,
        "motorStopReasonLabel": reasons.motor_stop_reason_label,
        "sunrise": find(input, &["sunrise", "sunrise_time"]).or_else(|| find(sun, &["sunrise", "sunrise_time"])).or_else(|| find(sun_times, &["sunrise_time", "sunrise"])),
        "sunset": find(input, &["sunset", "sunset_time"]).or_else(|| find(sun, &["sunset", "sunset_time"])).or_else(|| find(sun_times, &["sunset_time", "sunset"])),
        "connectId": find(input, &["connectId", "connect_id"]),
        "homekitCode": find(input, &["homekitCode", "homekit_code", "pairing_code"]),
        "hardwareRevision": find(input, &["hardwareRevision", "hardware_revision"]),
        "firmwareVersion": find(input, &["firmwareVersion", "firmware_version"]),
    })
}

fn find(value: &Value, names: &[&str]) -> Option<Value> {
    names
        .iter()
        .find_map(|name| value.get(*name).filter(|value| !value.is_null()).cloned())
}

fn find_integer(value: &Value, names: &[&str]) -> Value {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(parse_integer))
        .map_or(Value::Null, |number| json!(number))
}

fn parse_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Some(integer);
            }
            let number = number.as_f64()?;
            (number.is_finite()
                && number.fract() == 0.0
                && number >= i64::MIN as f64
                && number < 9_223_372_036_854_775_808.0)
                .then_some(number as i64)
        }
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}
