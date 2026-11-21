//! Transport-neutral contracts for future in-process integrations.
//!
//! Events are bounded notifications that state changed. They are not a durable
//! log; consumers must refresh authoritative state after subscribing or lagging.

use std::{collections::HashMap, time::Duration};
use std::{
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeviceCapabilities {
    pub open: bool,
    pub close: bool,
    pub stop: bool,
    pub set_position: bool,
    pub set_motor_speed: bool,
    pub calibrate_endpoints: bool,
    pub position_feedback: bool,
}

impl DeviceCapabilities {
    /// Current production controller supports endpoint actions, stop, position
    /// targeting, and motor-reported position.
    pub const WAZOMBI: Self = Self {
        open: true,
        close: true,
        stop: true,
        set_position: true,
        set_motor_speed: true,
        calibrate_endpoints: true,
        position_feedback: true,
    };

    /// Simulators support the same command and observation contract as hardware.
    pub const SIMULATOR: Self = Self::WAZOMBI;

    /// A LAN cover that supports the standard movement primitives and position
    /// feedback, but has no SOMA-specific settings or calibration operations.
    pub const LAN_COVER: Self = Self {
        open: true,
        close: true,
        stop: true,
        set_position: true,
        set_motor_speed: false,
        calibrate_endpoints: false,
        position_feedback: true,
    };
}

/// Vendor-neutral movement commands shared by in-process device drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlindCommand {
    Open,
    Close,
    Stop,
    SetPosition { closed_percent: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlindMotion {
    Opening,
    Closing,
    AtTarget,
    Stopped,
    Unknown,
}

/// State reported by a device driver, normalized to Somaris's percent-closed
/// convention (0 is open, 100 is closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlindObservation {
    pub current_closed_percent: Option<u8>,
    pub target_closed_percent: Option<u8>,
    pub motion: BlindMotion,
    pub position_control: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverFailureKind {
    /// The driver knows the request was not sent to the device.
    NotSent,
    /// The device or transport returned an explicit rejection.
    Rejected,
    /// The request may have reached the device but no reliable response arrived.
    OutcomeUnknown,
    /// The response could not be interpreted as the standard device contract.
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}")]
pub struct DriverFailure {
    pub kind: DriverFailureKind,
    pub code: String,
}

impl DriverFailure {
    pub fn new(kind: DriverFailureKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
        }
    }
}

/// A device provider maps the shared blind contract to one vendor protocol.
/// Implementations are registered in-process and receive only their own
/// persisted configuration; HTTP routes and the command queue stay provider
/// agnostic.
#[async_trait]
pub trait BlindDriver: Send + Sync {
    fn id(&self) -> &'static str;
    fn validate_config(&self, config: &Value) -> Result<(), DriverFailure>;
    async fn observe(&self, config: &Value) -> Result<BlindObservation, DriverFailure>;
    async fn execute(&self, config: &Value, command: BlindCommand) -> Result<(), DriverFailure>;
}

#[derive(Clone, Default)]
pub struct DriverRegistry {
    drivers: HashMap<String, Arc<dyn BlindDriver>>,
}

impl DriverRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry.register(Arc::new(ShellyCoverDriver::new()));
        registry
    }

    pub fn register(&mut self, driver: Arc<dyn BlindDriver>) {
        self.drivers.insert(driver.id().to_owned(), driver);
    }

    pub fn get(&self, driver_id: &str) -> Option<Arc<dyn BlindDriver>> {
        self.drivers.get(driver_id).cloned()
    }

    pub fn driver_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.drivers.keys().cloned().collect();
        ids.sort();
        ids
    }
}

const SHELLY_COVER_DRIVER_ID: &str = "shelly_cover";

#[derive(Clone)]
pub struct ShellyCoverDriver {
    client: Client,
    allowed_origins: HashMap<String, Url>,
}

struct ShellyCoverConfig {
    base_url: String,
    component_id: u32,
}

impl ShellyCoverDriver {
    pub fn new() -> Self {
        let configured_origins =
            std::env::var("SOMARIS_LAN_DRIVER_ALLOWED_ORIGINS").unwrap_or_default();
        let allowed_origins = match parse_shelly_allowed_origins(&configured_origins) {
            Ok(origins) => origins,
            Err(error) => {
                tracing::warn!(%error, "Invalid LAN driver allowlist; Shelly enrollment is disabled");
                HashMap::new()
            }
        };
        Self::with_canonical_allowed_origins(allowed_origins)
    }

    /// Build a Shelly provider restricted to the explicitly configured HTTP
    /// origins. Each origin must use an IP literal and contain no path or auth.
    pub fn with_allowed_origins<I, S>(origins: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let configured = origins
            .into_iter()
            .map(|origin| origin.as_ref().to_owned())
            .collect::<Vec<_>>()
            .join(",");
        let allowed_origins = parse_shelly_allowed_origins(&configured)?;
        Ok(Self::with_canonical_allowed_origins(allowed_origins))
    }

    fn with_canonical_allowed_origins(allowed_origins: HashMap<String, Url>) -> Self {
        // Reqwest's default HTTPS certificate validation remains enabled.
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .build()
            .expect("static Shelly HTTP client configuration should be valid");
        Self {
            client,
            allowed_origins,
        }
    }

    fn parse_config(&self, config: &Value) -> Result<(ShellyCoverConfig, Url), DriverFailure> {
        let invalid_config =
            || DriverFailure::new(DriverFailureKind::InvalidResponse, "invalid_driver_config");
        let fields = config.as_object().ok_or_else(invalid_config)?;
        if fields.len() != 2 {
            return Err(invalid_config());
        }
        let base_url = fields
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(invalid_config)?;
        let component_id = fields
            .get("component_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(invalid_config)?;
        let config = ShellyCoverConfig {
            base_url: base_url.to_owned(),
            component_id,
        };
        let url = Url::parse(&config.base_url).map_err(|_| {
            DriverFailure::new(DriverFailureKind::InvalidResponse, "invalid_driver_config")
        })?;
        let origin = canonical_shelly_origin(&url).map_err(|_| {
            DriverFailure::new(DriverFailureKind::InvalidResponse, "invalid_driver_config")
        })?;
        let Some(allowed_origin) = self.allowed_origins.get(&origin) else {
            return Err(DriverFailure::new(
                DriverFailureKind::InvalidResponse,
                "destination_not_allowed",
            ));
        };
        // Use the trusted URL parsed from service configuration for the request.
        // The caller's URL only selects an allowlisted origin and never reaches
        // the HTTP client.
        Ok((config, allowed_origin.clone()))
    }

    async fn rpc(
        &self,
        mut base_url: Url,
        method: &str,
        params: Value,
        is_command: bool,
    ) -> Result<Value, DriverFailure> {
        base_url.set_path(&format!("/rpc/{method}"));
        let response = self
            .client
            .post(base_url)
            .json(&params)
            .send()
            .await
            .map_err(|error| {
                let kind = if error.is_connect() || error.is_builder() {
                    DriverFailureKind::NotSent
                } else if is_command {
                    DriverFailureKind::OutcomeUnknown
                } else {
                    DriverFailureKind::NotSent
                };
                DriverFailure::new(kind, "rpc_transport_error")
            })?;
        if response.status() != StatusCode::OK {
            return Err(DriverFailure::new(
                if is_command {
                    if response.status().is_server_error() {
                        DriverFailureKind::OutcomeUnknown
                    } else {
                        DriverFailureKind::Rejected
                    }
                } else {
                    DriverFailureKind::NotSent
                },
                "rpc_http_error",
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            DriverFailure::new(
                if is_command {
                    DriverFailureKind::OutcomeUnknown
                } else {
                    DriverFailureKind::InvalidResponse
                },
                "rpc_invalid_response",
            )
        })?;
        if value.get("error").is_some() {
            return Err(DriverFailure::new(
                DriverFailureKind::Rejected,
                "rpc_device_rejected",
            ));
        }
        let result = value.get("result").cloned().unwrap_or(value);
        Ok(result)
    }

    fn component_params(config: &ShellyCoverConfig) -> Value {
        serde_json::json!({"id": config.component_id})
    }

    fn parse_observation(
        config: &ShellyCoverConfig,
        status: &Value,
    ) -> Result<BlindObservation, DriverFailure> {
        if status
            .get("id")
            .and_then(Value::as_u64)
            .is_some_and(|id| id != u64::from(config.component_id))
        {
            return Err(DriverFailure::new(
                DriverFailureKind::InvalidResponse,
                "rpc_component_mismatch",
            ));
        }
        let position = |key: &str| -> Result<Option<u8>, DriverFailure> {
            match status.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => {
                    let percent = value
                        .as_f64()
                        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                        .ok_or_else(|| {
                            DriverFailure::new(
                                DriverFailureKind::InvalidResponse,
                                "rpc_invalid_position",
                            )
                        })?;
                    Ok(Some((100.0 - percent).round() as u8))
                }
            }
        };
        let motion = match status.get("state").and_then(Value::as_str) {
            Some("opening") => BlindMotion::Opening,
            Some("closing") => BlindMotion::Closing,
            Some("open" | "closed") => BlindMotion::AtTarget,
            Some("stopped") => BlindMotion::Stopped,
            Some("calibrating") | None => BlindMotion::Unknown,
            Some(_) => {
                return Err(DriverFailure::new(
                    DriverFailureKind::InvalidResponse,
                    "rpc_invalid_state",
                ));
            }
        };
        Ok(BlindObservation {
            current_closed_percent: position("current_pos")?,
            target_closed_percent: position("target_pos")?,
            motion,
            position_control: status
                .get("pos_control")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

impl Default for ShellyCoverDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BlindDriver for ShellyCoverDriver {
    fn id(&self) -> &'static str {
        SHELLY_COVER_DRIVER_ID
    }

    fn validate_config(&self, config: &Value) -> Result<(), DriverFailure> {
        self.parse_config(config).map(|_| ())
    }

    async fn observe(&self, config: &Value) -> Result<BlindObservation, DriverFailure> {
        let (config, base_url) = self.parse_config(config)?;
        let status = self
            .rpc(
                base_url,
                "Cover.GetStatus",
                Self::component_params(&config),
                false,
            )
            .await?;
        Self::parse_observation(&config, &status)
    }

    async fn execute(&self, config: &Value, command: BlindCommand) -> Result<(), DriverFailure> {
        let (config, base_url) = self.parse_config(config)?;
        let (method, params) = match command {
            BlindCommand::Open => ("Cover.Open", Self::component_params(&config)),
            BlindCommand::Close => ("Cover.Close", Self::component_params(&config)),
            BlindCommand::Stop => ("Cover.Stop", Self::component_params(&config)),
            BlindCommand::SetPosition { closed_percent } if closed_percent <= 100 => (
                "Cover.GoToPosition",
                serde_json::json!({
                    "id": config.component_id,
                    "pos": 100 - closed_percent,
                }),
            ),
            BlindCommand::SetPosition { .. } => {
                return Err(DriverFailure::new(
                    DriverFailureKind::InvalidResponse,
                    "invalid_position",
                ));
            }
        };
        self.rpc(base_url, method, params, true).await?;
        Ok(())
    }
}

fn parse_shelly_allowed_origins(value: &str) -> Result<HashMap<String, Url>, String> {
    if value.trim().is_empty() {
        return Ok(HashMap::new());
    }
    value
        .split(',')
        .map(str::trim)
        .try_fold(HashMap::new(), |mut allowed, origin| {
            let url = Url::parse(origin)
                .map_err(|_| format!("Invalid LAN driver allowed origin: {origin}"))?;
            let canonical = canonical_shelly_origin(&url)?;
            allowed.insert(canonical, url);
            Ok(allowed)
        })
}

fn canonical_shelly_origin(url: &Url) -> Result<String, String> {
    let host_is_ip = url
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some();
    if !matches!(url.scheme(), "http" | "https")
        || !host_is_ip
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("LAN driver origins must be bare HTTP(S) IP origins".into());
    }
    Ok(url.origin().ascii_serialization())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    /// Monotonically increasing within this process. Use a fresh snapshot after
    /// a restart or a broadcast lag error.
    pub revision: u64,
}

#[derive(Clone)]
pub struct StateChangePublisher {
    sender: broadcast::Sender<StateChange>,
    revision: Arc<AtomicU64>,
}

impl Default for StateChangePublisher {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(128);
        Self {
            sender,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl StateChangePublisher {
    pub fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.sender.subscribe()
    }

    /// Nonblocking publication: a slow subscriber may lag and must resync.
    pub fn publish(&self) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.sender.send(StateChange { revision });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelly_driver_requires_an_exact_ip_origin_allowlist() {
        let driver = ShellyCoverDriver::with_allowed_origins(["http://192.168.1.20:8123"]).unwrap();
        assert!(
            driver
                .validate_config(&serde_json::json!({
                    "base_url": "http://192.168.1.20:8123",
                    "component_id": 0
                }))
                .is_ok()
        );

        let denied = driver
            .validate_config(&serde_json::json!({
                "base_url": "http://192.168.1.21:8123",
                "component_id": 0
            }))
            .unwrap_err();
        assert_eq!(denied.code, "destination_not_allowed");

        assert!(
            driver
                .validate_config(&serde_json::json!({
                    "base_url": "http://localhost:8123",
                    "component_id": 0
                }))
                .is_err()
        );
        assert!(ShellyCoverDriver::with_allowed_origins(["http://localhost:8123"]).is_err());
    }

    #[test]
    fn shelly_config_accepts_only_its_two_typed_fields() {
        let driver = ShellyCoverDriver::with_allowed_origins(["http://192.168.1.20"]).unwrap();
        for config in [
            serde_json::json!({"base_url": "http://192.168.1.20"}),
            serde_json::json!({"base_url": "http://192.168.1.20", "component_id": -1}),
            serde_json::json!({"base_url": "http://192.168.1.20", "component_id": 1.5}),
            serde_json::json!({"base_url": "http://192.168.1.20", "component_id": "0"}),
            serde_json::json!({"base_url": "http://192.168.1.20", "component_id": 0, "extra": true}),
            serde_json::json!({"base_url": false, "component_id": 0}),
        ] {
            assert!(
                driver.validate_config(&config).is_err(),
                "accepted {config}"
            );
        }
    }

    #[tokio::test]
    async fn subscribers_receive_revisions_and_lag_is_reported() {
        let publisher = StateChangePublisher::default();
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();
        publisher.publish();
        assert_eq!(first.recv().await.unwrap().revision, 1);
        assert_eq!(second.recv().await.unwrap().revision, 1);

        for _ in 0..140 {
            publisher.publish();
        }
        assert!(matches!(
            first.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
    }
}
