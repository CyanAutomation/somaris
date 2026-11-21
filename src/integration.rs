//! Transport-neutral contracts for future in-process integrations.
//!
//! Events are bounded notifications that state changed. They are not a durable
//! log; consumers must refresh authoritative state after subscribing or lagging.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
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
    async fn execute(
        &self,
        config: &Value,
        command: BlindCommand,
    ) -> Result<(), DriverFailure>;
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellyCoverConfig {
    base_url: String,
    component_id: u32,
}

impl ShellyCoverDriver {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .build()
            .expect("static Shelly HTTP client configuration should be valid");
        Self { client }
    }

    fn parse_config(config: &Value) -> Result<(ShellyCoverConfig, Url), DriverFailure> {
        let config: ShellyCoverConfig = serde_json::from_value(config.clone()).map_err(|_| {
            DriverFailure::new(DriverFailureKind::InvalidResponse, "invalid_driver_config")
        })?;
        let url = Url::parse(&config.base_url).map_err(|_| {
            DriverFailure::new(DriverFailureKind::InvalidResponse, "invalid_driver_config")
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(DriverFailure::new(
                DriverFailureKind::InvalidResponse,
                "invalid_driver_config",
            ));
        }
        Ok((config, url))
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
                    let percent = value.as_f64().filter(|value| value.is_finite() && (0.0..=100.0).contains(value)).ok_or_else(|| {
                        DriverFailure::new(DriverFailureKind::InvalidResponse, "rpc_invalid_position")
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
        Self::parse_config(config).map(|_| ())
    }

    async fn observe(&self, config: &Value) -> Result<BlindObservation, DriverFailure> {
        let (config, base_url) = Self::parse_config(config)?;
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

    async fn execute(
        &self,
        config: &Value,
        command: BlindCommand,
    ) -> Result<(), DriverFailure> {
        let (config, base_url) = Self::parse_config(config)?;
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
