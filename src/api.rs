use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::{
    ble::{
        BleAdapter, BleFailure, BleFailureStage, BleObservation, BtleplugAdapter,
        SOMA_MANUFACTURER_ID, decode_ba71_voltage, parse_soma_payload,
    },
    controller::ServiceAction,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path as RoutePath, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock, Semaphore, mpsc};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use url::Url;
use uuid::Uuid;

const OPENAPI_JSON: &str = include_str!("../openapi.json");
const SIMULATOR_ID: &str = "simulator-1";
const MAX_COMMANDS: usize = 1000;
const MAX_BATCHES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueConfig {
    pub max_outstanding_commands: usize,
    pub max_queued_per_device: usize,
    pub adapter_concurrency: usize,
    pub queue_wait_timeout_seconds: i64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_outstanding_commands: 1000,
            max_queued_per_device: 32,
            adapter_concurrency: 1,
            queue_wait_timeout_seconds: 120,
        }
    }
}

impl QueueConfig {
    fn validate(self) -> Result<Self, String> {
        if self.max_outstanding_commands == 0 || self.max_outstanding_commands > MAX_COMMANDS {
            return Err(format!(
                "max_outstanding_commands must be between 1 and {MAX_COMMANDS}"
            ));
        }
        if self.max_queued_per_device == 0
            || self.max_queued_per_device > self.max_outstanding_commands
        {
            return Err(
                "max_queued_per_device must be between 1 and max_outstanding_commands".into(),
            );
        }
        if self.adapter_concurrency == 0 || self.adapter_concurrency > 64 {
            return Err("adapter_concurrency must be between 1 and 64".into());
        }
        if !(1..=86_400).contains(&self.queue_wait_timeout_seconds) {
            return Err("queue_wait_timeout_seconds must be between 1 and 86400".into());
        }
        Ok(self)
    }

    fn from_environment() -> io::Result<Self> {
        let defaults = Self::default();
        let config = Self {
            max_outstanding_commands: env_positive_usize(
                "SOMARIS_COMMAND_MAX_OUTSTANDING",
                defaults.max_outstanding_commands,
            )?,
            max_queued_per_device: env_positive_usize(
                "SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE",
                defaults.max_queued_per_device,
            )?,
            adapter_concurrency: env_positive_usize(
                "SOMARIS_ADAPTER_CONCURRENCY",
                defaults.adapter_concurrency,
            )?,
            queue_wait_timeout_seconds: env_positive_usize(
                "SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS",
                defaults.queue_wait_timeout_seconds as usize,
            )? as i64,
        };
        config.validate().map_err(invalid_configuration)
    }
}

fn parse_allowed_origins(value: &str) -> Result<Vec<HeaderValue>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(str::trim)
        .map(|origin| {
            if origin.is_empty() {
                return Err("SOMARIS_ALLOWED_ORIGINS cannot contain an empty entry".into());
            }
            let url =
                Url::parse(origin).map_err(|_| format!("Invalid allowed origin: {origin}"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(format!("Allowed origin must be an HTTP origin: {origin}"));
            }
            url.origin()
                .ascii_serialization()
                .parse::<HeaderValue>()
                .map_err(|_| format!("Invalid allowed origin: {origin}"))
        })
        .collect()
}

fn env_positive_usize(name: &str, default: usize) -> io::Result<usize> {
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    let parsed = value.parse::<usize>().ok().filter(|value| *value > 0);
    parsed.ok_or_else(|| invalid_configuration(format!("{name} must be a positive integer")))
}

fn invalid_configuration(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<Inner>>,
    device_locks: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    adapter_slots: Arc<Semaphore>,
    ble_adapter: Arc<RwLock<Option<Arc<dyn BleAdapter>>>>,
    ble_enabled: bool,
    queue_config: QueueConfig,
    allowed_origins: Vec<HeaderValue>,
    started_at: DateTime<Utc>,
    started: Instant,
    history_path: Option<PathBuf>,
    history_persist_lock: Arc<Mutex<()>>,
    history_revision: Arc<AtomicU64>,
    data_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Inner {
    simulator_enabled: bool,
    simulator: Device,
    simulator_config: SimulatorConfig,
    enrollments: HashMap<String, Enrollment>,
    commands: HashMap<String, Command>,
    command_idempotency: HashMap<String, String>,
    batches: HashMap<String, Batch>,
    batch_idempotency: HashMap<String, String>,
    history: Vec<HistoryObservation>,
    next_cursor: u64,
    scanner: ScannerCounters,
    #[serde(skip)]
    devices: HashMap<String, Device>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimulatorConfig {
    scenario: SimulatorScenario,
    wireless_profile: WirelessProfile,
    battery_state: BatteryState,
    connection_delay_ms: u64,
    write_response_delay_ms: u64,
    telemetry_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ScannerCounters {
    #[serde(default)]
    status: String,
    #[serde(default)]
    adapter_available: bool,
    #[serde(default)]
    scan_active: bool,
    #[serde(default)]
    last_scan_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_advertisement_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_error_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_error_code: Option<String>,
    retry_count: u64,
    error_count: u64,
    advertisements_seen: u64,
    observations_stored: u64,
    malformed_payload_count: u64,
    unsupported_version_count: u64,
}

impl ScannerCounters {
    fn unavailable() -> Self {
        Self {
            status: "unavailable".into(),
            adapter_available: false,
            scan_active: false,
            last_scan_started_at: None,
            last_advertisement_at: None,
            last_error_at: None,
            last_error_code: Some("adapter_unavailable".into()),
            retry_count: 0,
            error_count: 0,
            advertisements_seen: 0,
            observations_stored: 0,
            malformed_payload_count: 0,
            unsupported_version_count: 0,
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::configured(true, None, None)
    }

    pub fn with_history_path(path: impl Into<PathBuf>) -> Self {
        Self::configured(true, Some(path.into()), None)
    }

    pub fn with_queue_config(mut self, config: QueueConfig) -> Result<Self, String> {
        let config = config.validate()?;
        self.adapter_slots = Arc::new(Semaphore::new(config.adapter_concurrency));
        self.queue_config = config;
        Ok(self)
    }

    pub fn with_ble_adapter(mut self, adapter: Arc<dyn BleAdapter>) -> Self {
        self.ble_adapter = Arc::new(RwLock::new(Some(adapter)));
        self.ble_enabled = true;
        self
    }

    pub fn with_allowed_origins<I, S>(mut self, origins: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values = origins
            .into_iter()
            .map(|origin| origin.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.allowed_origins = parse_allowed_origins(&values.join(","))?;
        Ok(self)
    }

    pub async fn from_data_path(path: impl Into<PathBuf>) -> io::Result<Self> {
        let state = Self::configured(true, None, Some(path.into()));
        state.load_durable_state().await?;
        state.resume_queued_commands().await;
        Ok(state)
    }

    pub async fn from_environment() -> io::Result<Self> {
        let simulator_enabled = std::env::var("SOMARIS_SIMULATOR")
            .map(|value| !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off"))
            .unwrap_or(true);
        let history_path = std::env::var_os("SOMARIS_HISTORY_PATH").map(PathBuf::from);
        let data_path = Some(
            std::env::var_os("SOMARIS_DATA_DIR")
                .map(|path| PathBuf::from(path).join("state.json"))
                .unwrap_or_else(|| PathBuf::from("data/state.json")),
        );
        let mut state = Self::configured(simulator_enabled, history_path, data_path);
        state.ble_enabled = true;
        state = state
            .with_queue_config(QueueConfig::from_environment()?)
            .map_err(invalid_configuration)?;
        let origins = std::env::var("SOMARIS_ALLOWED_ORIGINS").unwrap_or_default();
        state.allowed_origins = parse_allowed_origins(&origins).map_err(invalid_configuration)?;
        state.load_history_file().await?;
        state.load_durable_state().await?;
        state.resume_queued_commands().await;
        Ok(state)
    }

    fn configured(
        simulator_enabled: bool,
        history_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        let started_at = Utc::now();
        let simulator = make_simulated_device(started_at);
        let mut inner = Inner {
            simulator_enabled,
            simulator,
            simulator_config: SimulatorConfig::default(),
            enrollments: HashMap::new(),
            commands: HashMap::new(),
            command_idempotency: HashMap::new(),
            batches: HashMap::new(),
            batch_idempotency: HashMap::new(),
            history: Vec::new(),
            next_cursor: 1,
            scanner: ScannerCounters::unavailable(),
            devices: HashMap::new(),
        };
        if let Some(path) = &history_path {
            inner.history = read_history(path).unwrap_or_default();
            inner.next_cursor = inner
                .history
                .last()
                .map(|record| record.cursor.saturating_add(1))
                .unwrap_or(1);
        }
        let queue_config = QueueConfig::default();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            device_locks: Arc::new(Mutex::new(HashMap::new())),
            adapter_slots: Arc::new(Semaphore::new(queue_config.adapter_concurrency)),
            ble_adapter: Arc::new(RwLock::new(None)),
            ble_enabled: false,
            queue_config,
            allowed_origins: Vec::new(),
            started_at,
            started: Instant::now(),
            history_path,
            history_persist_lock: Arc::new(Mutex::new(())),
            history_revision: Arc::new(AtomicU64::new(0)),
            data_path,
        }
    }

    async fn load_durable_state(&self) -> io::Result<()> {
        let Some(path) = &self.data_path else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }
        let content = fs::read(path)?;
        let mut persisted: Inner = serde_json::from_slice(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let now = Utc::now();
        let mut guard = self.inner.lock().await;
        persisted.simulator_enabled = guard.simulator_enabled;
        persisted.scanner = ScannerCounters::unavailable();
        for command in persisted.commands.values_mut() {
            if command.status == CommandStatus::Connecting {
                command.status = CommandStatus::OutcomeUnknown;
                command.write_state = WriteState::PossiblySent;
                command.retryable = false;
                command.error_code = Some("outcome_unknown".into());
                command.failure_stage = Some("service_recovery".into());
                command.completed_at = Some(now);
            }
        }
        if persisted.simulator_enabled {
            persisted.simulator.freshness = Freshness::Fresh;
            persisted.simulator.last_seen_at = Utc::now();
            if let Some(position) = persisted.simulator.position.as_mut() {
                position.observed_at = persisted.simulator.last_seen_at;
            }
            persisted.simulator.battery = simulator_battery_telemetry(
                persisted.simulator_config.battery_state,
                persisted.simulator.last_seen_at,
            );
        }
        *guard = persisted;
        self.persist(&guard)
            .await
            .map_err(|_| io::Error::other("Could not persist recovered Somaris state."))?;
        Ok(())
    }

    async fn load_history_file(&self) -> io::Result<()> {
        let Some(path) = &self.history_path else {
            return Ok(());
        };
        let history = read_history(path)?;
        let mut inner = self.inner.lock().await;
        inner.next_cursor = history
            .last()
            .map(|record| record.cursor.saturating_add(1))
            .unwrap_or(1);
        inner.history = history;
        Ok(())
    }

    async fn persist(&self, inner: &Inner) -> Result<(), ()> {
        let Some(path) = &self.data_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(inner).map_err(|_| ())?;
        let path = path.clone();
        tokio::task::spawn_blocking(move || atomic_write(&path, &bytes))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())
    }

    async fn persist_history(
        &self,
        history: &[HistoryObservation],
        revision: u64,
    ) -> Result<(), ()> {
        let Some(path) = &self.history_path else {
            return Ok(());
        };
        let _persist_guard = self.history_persist_lock.lock().await;
        if self.history_revision.load(Ordering::Acquire) != revision {
            return Ok(());
        }
        let lines = history
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ())?
            .join("\n");
        let bytes = if lines.is_empty() {
            Vec::new()
        } else {
            format!("{lines}\n").into_bytes()
        };
        let path = path.clone();
        tokio::task::spawn_blocking(move || atomic_write(&path, &bytes))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())
    }

    async fn resume_queued_commands(&self) {
        let queued: Vec<_> = {
            let inner = self.inner.lock().await;
            inner
                .commands
                .values()
                .filter(|command| command.status == CommandStatus::Queued)
                .map(|command| command.id.clone())
                .collect()
        };
        for id in queued {
            tokio::spawn(process_command(self.clone(), id));
        }
    }

    pub fn spawn_background_tasks(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let history = {
                    let mut inner = state.inner.lock().await;
                    if !inner.simulator_enabled || inner.simulator_config.telemetry_paused {
                        continue;
                    }
                    let now = Utc::now();
                    advance_simulator(&mut inner, now);
                    inner.simulator.last_seen_at = now;
                    let recorded = record_history(&mut inner).unwrap_or(false);
                    if recorded {
                        let revision = state.history_revision.fetch_add(1, Ordering::AcqRel) + 1;
                        Some((inner.history.clone(), revision))
                    } else {
                        None
                    }
                };
                if let Some((history, revision)) = history {
                    let _ = state.persist_history(&history, revision).await;
                }
            }
        });
        if self.ble_enabled {
            let scanner_state = self.clone();
            tokio::spawn(async move { scanner_state.scanner_supervisor().await });
            let battery_state = self.clone();
            tokio::spawn(async move { battery_state.battery_poll_loop().await });
        }
    }

    async fn scanner_supervisor(&self) {
        loop {
            let mut adapter = self.ble_adapter.read().await.clone();
            if adapter.is_none() {
                match BtleplugAdapter::new().await {
                    Ok(value) => {
                        let value: Arc<dyn BleAdapter> = Arc::new(value);
                        *self.ble_adapter.write().await = Some(value.clone());
                        adapter = Some(value);
                    }
                    Err(failure) => {
                        let mut inner = self.inner.lock().await;
                        inner.scanner.status = "unavailable".into();
                        inner.scanner.adapter_available = false;
                        inner.scanner.scan_active = false;
                        inner.scanner.retry_count = inner.scanner.retry_count.saturating_add(1);
                        inner.scanner.error_count = inner.scanner.error_count.saturating_add(1);
                        inner.scanner.last_error_at = Some(Utc::now());
                        inner.scanner.last_error_code = Some(ble_failure_code(&failure).into());
                        drop(inner);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                }
            }

            let Some(adapter) = adapter else { continue };
            let (sender, mut receiver) = mpsc::channel(128);
            {
                let mut inner = self.inner.lock().await;
                inner.scanner.status = "scanning".into();
                inner.scanner.adapter_available = true;
                inner.scanner.scan_active = true;
                inner.scanner.last_scan_started_at = Some(Utc::now());
                inner.scanner.last_error_code = None;
            }
            let scan = tokio::spawn(async move { adapter.scan(sender).await });
            while let Some(observation) = receiver.recv().await {
                self.store_ble_observation(observation).await;
            }
            let failure = match scan.await {
                Ok(Err(failure)) => Some(failure),
                _ => None,
            };
            let adapter_failed = failure
                .as_ref()
                .is_some_and(|failure| failure.stage == BleFailureStage::Adapter);
            {
                let mut inner = self.inner.lock().await;
                inner.scanner.scan_active = false;
                inner.scanner.status = "unavailable".into();
                inner.scanner.retry_count = inner.scanner.retry_count.saturating_add(1);
                inner.scanner.error_count = inner.scanner.error_count.saturating_add(1);
                inner.scanner.last_error_at = Some(Utc::now());
                inner.scanner.last_error_code = failure
                    .as_ref()
                    .map(|failure| ble_failure_code(failure).to_owned())
                    .or_else(|| Some("scan_ended".into()));
                if adapter_failed {
                    inner.scanner.adapter_available = false;
                }
            }
            if adapter_failed {
                *self.ble_adapter.write().await = None;
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn store_ble_observation(&self, observation: BleObservation) {
        let now = Utc::now();
        let parsed = match parse_soma_payload(&observation.manufacturer_payload) {
            Ok(parsed) => parsed,
            Err(error) => {
                let mut inner = self.inner.lock().await;
                inner.scanner.advertisements_seen =
                    inner.scanner.advertisements_seen.saturating_add(1);
                inner.scanner.last_advertisement_at = Some(now);
                match error {
                    crate::ble::BleDecodeError::UnsupportedVersion(_) => {
                        inner.scanner.unsupported_version_count =
                            inner.scanner.unsupported_version_count.saturating_add(1);
                    }
                    _ => {
                        inner.scanner.malformed_payload_count =
                            inner.scanner.malformed_payload_count.saturating_add(1);
                    }
                }
                return;
            }
        };
        let mut raw_manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
        raw_manufacturer_data.extend_from_slice(&observation.manufacturer_payload);
        let logical_id = logical_id_for_name(&parsed.advertised_name, "soma");
        let persisted_history = {
            let mut inner = self.inner.lock().await;
            inner.scanner.advertisements_seen = inner.scanner.advertisements_seen.saturating_add(1);
            inner.scanner.observations_stored = inner.scanner.observations_stored.saturating_add(1);
            inner.scanner.last_advertisement_at = Some(now);
            let previous = inner.devices.get(&observation.peripheral_id).cloned();
            let id = previous
                .as_ref()
                .map(|device| device.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let battery =
                previous
                    .map(|device| device.battery)
                    .unwrap_or_else(|| BatteryTelemetry {
                        status: BatteryStatus::NotObserved,
                        voltage_mv: None,
                        percent_estimate: None,
                        observed_at: None,
                        last_attempt_at: None,
                        error_code: None,
                        failure_stage: None,
                        source: None,
                    });
            let device = Device {
                id,
                logical_id: Some(logical_id),
                observed_address: observation.observed_address,
                adapter_device_id: observation.peripheral_id.clone(),
                device_name: observation.device_name,
                advertised_name: parsed.advertised_name,
                rssi_dbm: observation.rssi_dbm,
                last_seen_at: now,
                freshness: Freshness::Fresh,
                source: Source::BleAdvertisement,
                is_simulated: false,
                manufacturer_data_hex: hex::encode(raw_manufacturer_data),
                manufacturer_payload_hex: hex::encode(observation.manufacturer_payload),
                position_status: PositionStatus::Observed,
                position: Some(position_value(
                    parsed.current_closed_percent,
                    parsed.target_closed_percent,
                    now,
                    Source::BleAdvertisement,
                )),
                movement_status: movement_status(
                    parsed.current_closed_percent,
                    parsed.target_closed_percent,
                ),
                battery,
            };
            inner.devices.insert(observation.peripheral_id, device);
            prune_registry(&mut inner, now);
            if record_history(&mut inner).unwrap_or(false) {
                let revision = self.history_revision.fetch_add(1, Ordering::AcqRel) + 1;
                Some((inner.history.clone(), revision))
            } else {
                None
            }
        };
        if let Some((history, revision)) = persisted_history {
            let _ = self.persist_history(&history, revision).await;
        }
    }

    async fn battery_poll_loop(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let adapter = self.ble_adapter.read().await.clone();
            let Some(adapter) = adapter else { continue };
            let now = Utc::now();
            let candidates = {
                let mut inner = self.inner.lock().await;
                prune_registry(&mut inner, now);
                inner
                    .enrollments
                    .values()
                    .filter(|enrollment| !enrollment.is_simulated)
                    .filter_map(|enrollment| {
                        let matches: Vec<_> = inner
                            .devices
                            .values()
                            .filter(|device| {
                                device.logical_id.as_deref() == Some(enrollment.logical_id.as_str())
                                    && device.advertised_name == enrollment.advertised_name
                                    && device.freshness == Freshness::Fresh
                            })
                            .collect();
                        if matches.len() != 1 {
                            return None;
                        }
                        let device = matches[0];
                        let due = device.battery.last_attempt_at.is_none_or(|attempt| {
                            now.signed_duration_since(attempt) >= ChronoDuration::hours(1)
                        });
                        due.then(|| {
                            (
                                enrollment.id.clone(),
                                enrollment.logical_id.clone(),
                                enrollment.advertised_name.clone(),
                                device.adapter_device_id.clone(),
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            };
            for (managed_id, logical_id, expected_name, peripheral_id) in candidates {
                let lock = {
                    let mut locks = self.device_locks.lock().await;
                    locks
                        .entry(managed_id.clone())
                        .or_insert_with(|| Arc::new(Semaphore::new(1)))
                        .clone()
                };
                let Ok(_device_permit) = lock.acquire_owned().await else {
                    continue;
                };
                let Ok(_adapter_permit) = self.adapter_slots.clone().acquire_owned().await else {
                    continue;
                };
                self.record_battery_attempt(
                    &logical_id,
                    &peripheral_id,
                    expected_name,
                    adapter.clone(),
                )
                .await;
            }
        }
    }

    async fn record_battery_attempt(
        &self,
        logical_id: &str,
        peripheral_id: &str,
        expected_name: String,
        adapter: Arc<dyn BleAdapter>,
    ) {
        let attempted_at = Utc::now();
        let response = adapter.read_battery(peripheral_id, &expected_name).await;
        let mut inner = self.inner.lock().await;
        let Some(device) = inner.devices.get_mut(peripheral_id) else {
            return;
        };
        if device.logical_id.as_deref() != Some(logical_id) {
            return;
        }
        device.battery.last_attempt_at = Some(attempted_at);
        device.battery.source = Some(BatterySource::GattBa71);
        match response {
            Ok(bytes) => match decode_ba71_voltage(&bytes) {
                Ok(reading) => {
                    device.battery.status = BatteryStatus::Observed;
                    device.battery.voltage_mv = Some(reading.voltage_mv);
                    device.battery.percent_estimate = Some(reading.percent_estimate);
                    device.battery.observed_at = Some(attempted_at);
                    device.battery.error_code = None;
                    device.battery.failure_stage = None;
                }
                Err(_) => {
                    device.battery.status = BatteryStatus::InvalidValue;
                    device.battery.error_code = Some("invalid_battery_value".into());
                    device.battery.failure_stage = Some("battery_read".into());
                }
            },
            Err(failure) => {
                device.battery.status = if failure.stage == BleFailureStage::BatteryDiscovery {
                    BatteryStatus::Unsupported
                } else {
                    BatteryStatus::Unavailable
                };
                device.battery.error_code = (device.battery.status == BatteryStatus::Unsupported)
                    .then(|| "ba71_unsupported".into())
                    .or_else(|| Some(ble_failure_code(&failure).into()));
                device.battery.failure_stage = Some(failure.stage.as_str().into());
            }
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        Self {
            scenario: SimulatorScenario::Normal,
            wireless_profile: WirelessProfile::HallwayObserved,
            battery_state: BatteryState::Unavailable,
            connection_delay_ms: 80,
            write_response_delay_ms: 72,
            telemetry_paused: false,
        }
    }
}

pub fn router(state: AppState) -> Router {
    let allowed_origins = state.allowed_origins.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed_origins.clone()))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::CONTENT_TYPE,
            header::IF_NONE_MATCH,
            header::HeaderName::from_static("idempotency-key"),
        ])
        .expose_headers([header::ETAG]);
    Router::new()
        .route("/api/v1/health", get(health).options(preflight))
        .route("/api/v1/ready", get(ready).options(preflight))
        .route("/api/v1/diagnostics", get(diagnostics).options(preflight))
        .route("/api/v1/devices", get(list_devices).options(preflight))
        .route("/api/v1/devices/{id}", get(get_device).options(preflight))
        .route("/api/v1/history", get(get_history).options(preflight))
        .route("/api/dev/simulator", get(get_simulator).options(preflight))
        .route(
            "/api/dev/simulator/scenario",
            put(set_simulator_scenario).options(preflight),
        )
        .route(
            "/api/dev/simulator/reset",
            post(reset_simulator).options(preflight),
        )
        .route(
            "/api/control/v1/devices",
            get(list_managed_devices)
                .post(enroll_device)
                .options(preflight),
        )
        .route(
            "/api/control/v1/devices/{id}",
            delete(remove_enrollment).options(preflight),
        )
        .route(
            "/api/control/v1/devices/{id}/commands",
            post(queue_command).options(preflight),
        )
        .route(
            "/api/control/v1/devices/{id}/toggle",
            post(toggle_device).options(preflight),
        )
        .route(
            "/api/control/v1/batches",
            post(queue_batch).options(preflight),
        )
        .route(
            "/api/control/v1/batches/{id}",
            get(get_batch).options(preflight),
        )
        .route("/api/control/v1/queue", get(get_queue).options(preflight))
        .route(
            "/api/control/v1/commands/{id}",
            get(get_command).options(preflight),
        )
        .route("/openapi.json", get(openapi_json))
        .route("/openapi.yaml", get(openapi_yaml))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            allowed_origins,
            enforce_cors_contract,
        ))
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn enforce_cors_contract(
    State(allowed_origins): State<Vec<HeaderValue>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let is_preflight = request.method() == axum::http::Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    if is_preflight {
        let origin = request.headers().get(header::ORIGIN);
        let allowed = origin.is_some_and(|origin| allowed_origins.contains(origin));
        let requested_method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<axum::http::Method>().ok());
        let method_allowed = requested_method.is_some_and(|method| {
            matches!(
                method,
                axum::http::Method::GET
                    | axum::http::Method::POST
                    | axum::http::Method::PUT
                    | axum::http::Method::DELETE
                    | axum::http::Method::OPTIONS
            )
        });
        if !allowed || !method_allowed {
            let mut response = error_response(
                StatusCode::FORBIDDEN,
                "cors_origin_denied",
                "The request origin or method is not allowed.",
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
                .headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Origin"));
            return response;
        }
    }

    let mut response = next.run(request).await;
    if is_preflight && response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::NO_CONTENT;
    }
    response
}

async fn openapi_json() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        OPENAPI_JSON,
    )
}

async fn openapi_yaml() -> Response {
    let value: Value = match serde_json::from_str(OPENAPI_JSON) {
        Ok(value) => value,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match serde_yaml::to_string(&value) {
        Ok(yaml) => (
            [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")],
            yaml,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn health(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().await;
    let scanner_status = inner.scanner.status.clone();
    let status = if inner.scanner.scan_active && inner.scanner.adapter_available {
        "healthy"
    } else {
        "degraded"
    };
    let data = HealthData {
        status: status.to_owned(),
        scanner_status,
        generated_at: Utc::now(),
    };
    Json(envelope(data, None)).into_response()
}

async fn ready(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().await;
    let ready = inner.scanner.scan_active && inner.scanner.adapter_available;
    let data = HealthData {
        status: if ready { "healthy" } else { "degraded" }.to_owned(),
        scanner_status: inner.scanner.status.clone(),
        generated_at: Utc::now(),
    };
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(envelope(data, None))).into_response()
}

async fn diagnostics(State(state): State<AppState>) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let retained = inner.devices.len() + usize::from(inner.simulator_enabled);
    let stale = inner
        .devices
        .values()
        .filter(|device| device.freshness == Freshness::Stale)
        .count();
    let data = Diagnostics {
        service_started_at: state.started_at,
        uptime_seconds: state.started.elapsed().as_secs_f64(),
        history_enabled: state.history_path.is_some(),
        scanner: ScannerDiagnostics {
            status: inner.scanner.status.clone(),
            adapter_available: inner.scanner.adapter_available,
            scan_active: inner.scanner.scan_active,
            last_scan_started_at: inner.scanner.last_scan_started_at,
            last_advertisement_at: inner.scanner.last_advertisement_at,
            last_error_at: inner.scanner.last_error_at,
            last_error_code: inner.scanner.last_error_code.clone(),
            retry_count: inner.scanner.retry_count,
            error_count: inner.scanner.error_count,
            soma_advertisements_seen: inner.scanner.advertisements_seen,
            observations_stored: inner.scanner.observations_stored,
            malformed_payload_count: inner.scanner.malformed_payload_count,
            unsupported_version_count: inner.scanner.unsupported_version_count,
        },
        registry: RegistryDiagnostics {
            retained_device_count: retained as u64,
            stale_device_count: stale as u64,
        },
    };
    Json(envelope(data, None)).into_response()
}

async fn list_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let mut devices: Vec<_> = inner.devices.values().cloned().collect();
    if inner.simulator_enabled {
        devices.push(device_snapshot(&inner));
    }
    devices.sort_by(|left, right| right.last_seen_at.cmp(&left.last_seen_at));
    conditional_json(
        &headers,
        devices.clone(),
        envelope(devices.clone(), Some(devices.len() as u64)),
    )
}

async fn get_device(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let inner = state.inner.lock().await;
    let Some(device) = inner
        .simulator_enabled
        .then(|| device_snapshot(&inner))
        .filter(|device| device.id == id)
        .or_else(|| {
            inner
                .devices
                .values()
                .find(|device| device.id == id)
                .cloned()
        })
    else {
        return error_response(
            StatusCode::NOT_FOUND,
            "device_not_found",
            "Device was not found.",
        );
    };
    conditional_json(&headers, device.clone(), envelope(device.clone(), None))
}

async fn get_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let Some(_path) = &state.history_path else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "history_disabled",
            "Persistent history is not configured.",
        );
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_query",
                "History query is invalid.",
            );
        }
    };
    if query.limit == 0 || query.limit > 1000 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be from 1 to 1000.",
        );
    }
    let since = match query.since.as_deref() {
        Some(value) => match DateTime::parse_from_rfc3339(value) {
            Ok(value) => Some(value.with_timezone(&Utc)),
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_since",
                    "since must be a date-time.",
                );
            }
        },
        None => None,
    };
    let inner = state.inner.lock().await;
    let latest_cursor = inner
        .history
        .last()
        .map(|record| record.cursor)
        .unwrap_or(0);
    let records: Vec<_> = inner
        .history
        .iter()
        .filter(|record| record.cursor > query.after.unwrap_or(0))
        .filter(|record| since.is_none_or(|since| record.observed_at >= since))
        .filter(|record| {
            query
                .device_id
                .as_deref()
                .is_none_or(|id| id == record.device_id || record.logical_id.as_deref() == Some(id))
        })
        .take(query.limit)
        .cloned()
        .collect();
    let next_cursor = records
        .last()
        .map(|record| record.cursor)
        .unwrap_or(query.after.unwrap_or(0));
    let has_more = inner
        .history
        .iter()
        .any(|record| record.cursor > next_cursor);
    let data = HistoryData {
        data: records.clone(),
        meta: HistoryMeta {
            generated_at: Utc::now(),
            count: records.len() as u64,
            next_cursor,
            latest_cursor,
            has_more,
        },
    };
    conditional_json(&headers, data.clone(), data)
}

async fn get_simulator(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().await;
    if !inner.simulator_enabled {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_disabled",
            "The simulator is disabled.",
        );
    }
    Json(envelope(simulator_status(&inner), None)).into_response()
}

async fn set_simulator_scenario(
    State(state): State<AppState>,
    payload: Result<Json<SimulatorScenarioRequest>, JsonRejection>,
) -> Response {
    let request = match parse_json(payload) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    if request.scenario.is_none()
        && request.wireless_profile.is_none()
        && request.battery_state.is_none()
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "empty_scenario",
            "At least one simulator setting is required.",
        );
    }
    let mut inner = state.inner.lock().await;
    if !inner.simulator_enabled {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_disabled",
            "The simulator is disabled.",
        );
    }
    if let Some(scenario) = request.scenario {
        inner.simulator_config.scenario = scenario;
        if scenario == SimulatorScenario::Normal {
            inner.simulator_config.telemetry_paused = false;
        } else if scenario == SimulatorScenario::TelemetryPaused {
            inner.simulator_config.telemetry_paused = true;
        }
    }
    if let Some(profile) = request.wireless_profile {
        inner.simulator_config.wireless_profile = profile;
        (
            inner.simulator_config.connection_delay_ms,
            inner.simulator_config.write_response_delay_ms,
        ) = match profile {
            WirelessProfile::Standard => (0, 0),
            WirelessProfile::HallwayObserved => (80, 72),
            WirelessProfile::HallwaySlowLink => (702, 72),
        };
        inner.simulator.rssi_dbm = simulator_rssi(profile, Utc::now());
    }
    if let Some(battery_state) = request.battery_state {
        inner.simulator_config.battery_state = battery_state;
        inner.simulator.battery = simulator_battery_telemetry(battery_state, Utc::now());
    }
    if state.persist(&inner).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "Simulator settings may have changed; refresh simulator status.",
        );
    }
    Json(envelope(simulator_status(&inner), None)).into_response()
}

async fn reset_simulator(
    State(state): State<AppState>,
    payload: Result<Json<SimulatorResetRequest>, JsonRejection>,
) -> Response {
    let request = match parse_json(payload) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let mut inner = state.inner.lock().await;
    if !inner.simulator_enabled {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_disabled",
            "The simulator is disabled.",
        );
    }
    let position = request.position_closed_percent.unwrap_or(100);
    if position > 100 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_position",
            "position_closed_percent must be from 0 to 100.",
        );
    }
    inner.simulator_config.scenario = SimulatorScenario::Normal;
    inner.simulator_config.telemetry_paused = false;
    let now = Utc::now();
    inner.simulator.position = Some(position_value(position, position, now, Source::Simulator));
    inner.simulator.movement_status = MovementStatus::AtTarget;
    inner.simulator.last_seen_at = now;
    let history_changed = match record_history(&mut inner) {
        Ok(changed) => changed,
        Err(record) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "history_write_failed",
                &record,
            );
        }
    };
    let history_revision = if history_changed {
        state.history_revision.fetch_add(1, Ordering::AcqRel) + 1
    } else {
        state.history_revision.load(Ordering::Acquire)
    };
    if state
        .persist_history(&inner.history, history_revision)
        .await
        .is_err()
    {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "history_write_failed",
            "Could not persist the simulator history sample.",
        );
    }
    if state.persist(&inner).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "Simulator state may have changed; refresh simulator status.",
        );
    }
    Json(envelope(simulator_status(&inner), None)).into_response()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Freshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    BleAdvertisement,
    Simulator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PositionStatus {
    Observed,
    Unsupported,
    InvalidPayload,
    NotObserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MovementStatus {
    Closing,
    Opening,
    AtTarget,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PositionConfidence {
    ProjectConfirmed,
    Simulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BatteryStatus {
    Observed,
    Unsupported,
    InvalidValue,
    Unavailable,
    NotObserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BatterySource {
    GattBa71,
    Simulator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Device {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_address: Option<String>,
    #[serde(default, skip_serializing)]
    adapter_device_id: String,
    device_name: Option<String>,
    advertised_name: String,
    rssi_dbm: i16,
    last_seen_at: DateTime<Utc>,
    freshness: Freshness,
    source: Source,
    is_simulated: bool,
    manufacturer_data_hex: String,
    manufacturer_payload_hex: String,
    position_status: PositionStatus,
    position: Option<Position>,
    movement_status: MovementStatus,
    battery: BatteryTelemetry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Position {
    current_closed_percent: u8,
    target_closed_percent: u8,
    observed_at: DateTime<Utc>,
    source: Source,
    confidence: PositionConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatteryTelemetry {
    status: BatteryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    voltage_mv: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    percent_estimate: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_attempt_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<BatterySource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    generated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope<T> {
    data: T,
    meta: Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorEnvelope {
    error: ApiError,
    meta: Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthData {
    status: String,
    scanner_status: String,
    generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Diagnostics {
    service_started_at: DateTime<Utc>,
    uptime_seconds: f64,
    history_enabled: bool,
    scanner: ScannerDiagnostics,
    registry: RegistryDiagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScannerDiagnostics {
    status: String,
    adapter_available: bool,
    scan_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scan_started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_advertisement_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error_code: Option<String>,
    retry_count: u64,
    error_count: u64,
    soma_advertisements_seen: u64,
    observations_stored: u64,
    malformed_payload_count: u64,
    unsupported_version_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryDiagnostics {
    retained_device_count: u64,
    stale_device_count: u64,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    after: Option<u64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
    since: Option<String>,
    device_id: Option<String>,
}

fn default_history_limit() -> usize {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryObservation {
    cursor: u64,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_id: Option<String>,
    observed_at: DateTime<Utc>,
    rssi_dbm: i16,
    position_status: PositionStatus,
    position: Option<HistoryPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryPosition {
    current_closed_percent: u8,
    target_closed_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryMeta {
    generated_at: DateTime<Utc>,
    count: u64,
    next_cursor: u64,
    latest_cursor: u64,
    has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryData {
    data: Vec<HistoryObservation>,
    meta: HistoryMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SimulatorScenario {
    Normal,
    PreWriteFailure,
    OutcomeUnknown,
    GattWriteError,
    TelemetryPaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WirelessProfile {
    Standard,
    HallwayObserved,
    HallwaySlowLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BatteryState {
    Unsupported,
    Unavailable,
    ObservedSample,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulatorScenarioRequest {
    scenario: Option<SimulatorScenario>,
    wireless_profile: Option<WirelessProfile>,
    battery_state: Option<BatteryState>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulatorResetRequest {
    position_closed_percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimulatorStatus {
    scenario: SimulatorScenario,
    wireless_profile: WirelessProfile,
    battery_state: BatteryState,
    connection_delay_ms: u64,
    write_response_delay_ms: u64,
    telemetry_paused: bool,
    current_closed_percent: u8,
    target_closed_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Enrollment {
    id: String,
    label: String,
    advertised_name: String,
    logical_id: String,
    is_simulated: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedDevice {
    enrollment: Enrollment,
    availability: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<ControlPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ControlPosition {
    current_closed_percent: u8,
    target_closed_percent: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRequest {
    observed_device_id: String,
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Open,
    Close,
    Stop,
}

impl Action {
    fn target_closed_percent(self, current: u8) -> u8 {
        match self {
            Self::Open => 0,
            Self::Close => 100,
            Self::Stop => current,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandRequest {
    action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CommandStatus {
    Queued,
    Connecting,
    ConnectionFailed,
    WriteAcknowledged,
    WriteFailed,
    OutcomeUnknown,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WriteState {
    NotSent,
    PossiblySent,
    Acknowledged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Command {
    id: String,
    managed_device_id: String,
    action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_basis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_position: Option<ControlPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_observed_at: Option<DateTime<Utc>>,
    status: CommandStatus,
    accepted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    attempt: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    write_state: WriteState,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchCommandRequest {
    action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchTarget {
    managed_device_id: String,
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<Command>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchSkipped {
    managed_device_id: String,
    label: String,
    availability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Batch {
    id: String,
    action: Action,
    status: String,
    accepted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<DateTime<Utc>>,
    targets: Vec<BatchTarget>,
    skipped: Vec<BatchSkipped>,
    target_count: u64,
    skipped_count: u64,
    acknowledged_count: u64,
    failed_count: u64,
    pending_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueueDiagnostics {
    pending_commands: u64,
    running_commands: u64,
    outstanding_commands: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    oldest_queued_at: Option<DateTime<Utc>>,
    oldest_queued_age_seconds: f64,
    max_outstanding_commands: u64,
    max_queued_per_device: u64,
    adapter_concurrency: u64,
}

fn envelope<T>(data: T, count: Option<u64>) -> Envelope<T> {
    Envelope {
        data,
        meta: Meta {
            generated_at: Utc::now(),
            count,
        },
    }
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: ApiError {
                code: code.to_owned(),
                message: message.to_owned(),
            },
            meta: Meta {
                generated_at: Utc::now(),
                count: None,
            },
        }),
    )
        .into_response()
}

fn parse_json<T: DeserializeOwned>(
    payload: Result<Json<T>, JsonRejection>,
) -> Result<T, Box<Response>> {
    match payload {
        Ok(Json(value)) => Ok(value),
        Err(error) => {
            let status = if error.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE {
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            } else {
                StatusCode::BAD_REQUEST
            };
            Err(Box::new(error_response(
                status,
                "invalid_request_body",
                "Request body must be valid JSON for this operation.",
            )))
        }
    }
}

fn conditional_json<T: Serialize, R: Serialize>(
    headers: &HeaderMap,
    etag_value: T,
    response_value: R,
) -> Response {
    let serialized = serde_json::to_vec(&etag_value).unwrap_or_default();
    let etag = format!("\"{}\"", hex::encode(Sha256::digest(serialized)));
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == etag || candidate.trim() == "*")
        });
    if matches {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, etag)
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let mut response = Json(response_value).into_response();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn read_history(path: &Path) -> io::Result<Vec<HistoryObservation>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .collect()
}

fn make_simulated_device(now: DateTime<Utc>) -> Device {
    let name = "Hallway Blinds0".to_owned();
    let payload = simulated_manufacturer_payload(100, 100);
    let mut manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
    manufacturer_data.extend_from_slice(&payload);
    Device {
        id: SIMULATOR_ID.to_owned(),
        logical_id: Some(logical_id_for_name(&name, "simulator")),
        observed_address: Some("simulator://blind-1".to_owned()),
        adapter_device_id: "simulator://blind-1".to_owned(),
        device_name: Some("S".to_owned()),
        advertised_name: name,
        rssi_dbm: -84,
        last_seen_at: now,
        freshness: Freshness::Fresh,
        source: Source::Simulator,
        is_simulated: true,
        manufacturer_data_hex: hex::encode(manufacturer_data),
        manufacturer_payload_hex: hex::encode(payload),
        position_status: PositionStatus::Observed,
        position: Some(position_value(100, 100, now, Source::Simulator)),
        movement_status: MovementStatus::AtTarget,
        battery: simulator_battery_telemetry(BatteryState::Unavailable, now),
    }
}

fn position_value(
    current_closed_percent: u8,
    target_closed_percent: u8,
    observed_at: DateTime<Utc>,
    source: Source,
) -> Position {
    Position {
        current_closed_percent,
        target_closed_percent,
        observed_at,
        source,
        confidence: if source == Source::Simulator {
            PositionConfidence::Simulated
        } else {
            PositionConfidence::ProjectConfirmed
        },
    }
}

fn simulator_status(inner: &Inner) -> SimulatorStatus {
    let position = inner.simulator.position.as_ref();
    SimulatorStatus {
        scenario: inner.simulator_config.scenario,
        wireless_profile: inner.simulator_config.wireless_profile,
        battery_state: inner.simulator_config.battery_state,
        connection_delay_ms: inner.simulator_config.connection_delay_ms,
        write_response_delay_ms: inner.simulator_config.write_response_delay_ms,
        telemetry_paused: inner.simulator_config.telemetry_paused,
        current_closed_percent: position
            .map(|value| value.current_closed_percent)
            .unwrap_or(100),
        target_closed_percent: position
            .map(|value| value.target_closed_percent)
            .unwrap_or(100),
    }
}

fn record_history(inner: &mut Inner) -> Result<bool, String> {
    let mut devices: Vec<_> = inner.devices.values().cloned().collect();
    if inner.simulator_enabled {
        devices.push(device_snapshot(inner));
    }
    let mut changed = false;
    for device in devices {
        let current_position = device.position.as_ref().map(|position| {
            (
                position.current_closed_percent,
                position.target_closed_percent,
            )
        });
        let latest = inner
            .history
            .iter()
            .rev()
            .find(|record| record.device_id == device.id);
        let same_position = latest.is_some_and(|last| {
            last.position.as_ref().map(|position| {
                (
                    position.current_closed_percent,
                    position.target_closed_percent,
                )
            }) == current_position
        });
        if same_position
            && latest.is_some_and(|last| {
                device.last_seen_at.signed_duration_since(last.observed_at)
                    < ChronoDuration::seconds(5)
            })
        {
            continue;
        }
        inner.history.push(HistoryObservation {
            cursor: inner.next_cursor,
            device_id: device.id,
            logical_id: device.logical_id,
            observed_at: device.last_seen_at,
            rssi_dbm: device.rssi_dbm,
            position_status: device.position_status,
            position: device.position.map(|position| HistoryPosition {
                current_closed_percent: position.current_closed_percent,
                target_closed_percent: position.target_closed_percent,
            }),
        });
        inner.next_cursor = inner.next_cursor.saturating_add(1);
        changed = true;
    }
    if inner.history.len() > 50_000 {
        let remove = inner.history.len() - 50_000;
        inner.history.drain(0..remove);
    }
    Ok(changed)
}

fn advance_simulator(inner: &mut Inner, now: DateTime<Utc>) {
    let Some((current, target)) = inner.simulator.position.as_mut().map(|position| {
        if position.current_closed_percent < position.target_closed_percent {
            position.current_closed_percent = position
                .current_closed_percent
                .saturating_add(1)
                .min(position.target_closed_percent);
        } else if position.current_closed_percent > position.target_closed_percent {
            position.current_closed_percent = position
                .current_closed_percent
                .saturating_sub(1)
                .max(position.target_closed_percent);
        }
        position.observed_at = now;
        (
            position.current_closed_percent,
            position.target_closed_percent,
        )
    }) else {
        return;
    };
    inner.simulator.movement_status = movement_status(current, target);
    inner.simulator.rssi_dbm = simulator_rssi(inner.simulator_config.wireless_profile, now);
}

fn simulator_rssi(profile: WirelessProfile, now: DateTime<Utc>) -> i16 {
    const HALLWAY_RSSI_SAMPLES: [i16; 26] = [
        -82, -85, -87, -71, -83, -67, -67, -90, -85, -85, -86, -85, -88, -85, -87, -86, -83,
        -83, -83, -82, -84, -84, -84, -85, -84, -82,
    ];
    match profile {
        WirelessProfile::Standard => -60,
        WirelessProfile::HallwayObserved | WirelessProfile::HallwaySlowLink => {
            let index = now.timestamp().rem_euclid(HALLWAY_RSSI_SAMPLES.len() as i64) as usize;
            HALLWAY_RSSI_SAMPLES[index]
        }
    }
}

fn simulated_manufacturer_payload(
    current_closed_percent: u8,
    target_closed_percent: u8,
) -> Vec<u8> {
    // Byte 1 is retained as the opaque 0x64 value seen in the live advertisement.
    let mut payload = vec![0x05, 0x64, current_closed_percent, target_closed_percent];
    payload.extend_from_slice(b"Hallway Blinds0");
    payload
}

fn simulator_battery_telemetry(state: BatteryState, now: DateTime<Utc>) -> BatteryTelemetry {
    match state {
        BatteryState::Unsupported => BatteryTelemetry {
            status: BatteryStatus::Unsupported,
            voltage_mv: None,
            percent_estimate: None,
            observed_at: None,
            last_attempt_at: None,
            error_code: None,
            failure_stage: None,
            source: Some(BatterySource::Simulator),
        },
        BatteryState::Unavailable => BatteryTelemetry {
            status: BatteryStatus::Unavailable,
            voltage_mv: Some(4210),
            percent_estimate: Some(100),
            observed_at: Some(now - ChronoDuration::minutes(40)),
            last_attempt_at: Some(now - ChronoDuration::minutes(10)),
            error_code: Some("device_disconnected_during_service_resolution".into()),
            failure_stage: Some("service_resolution".into()),
            source: Some(BatterySource::Simulator),
        },
        BatteryState::ObservedSample => BatteryTelemetry {
            status: BatteryStatus::Observed,
            voltage_mv: Some(4230),
            percent_estimate: Some(100),
            observed_at: Some(now),
            last_attempt_at: Some(now),
            error_code: None,
            failure_stage: None,
            source: Some(BatterySource::Simulator),
        },
    }
}

fn prune_registry(inner: &mut Inner, now: DateTime<Utc>) {
    inner.devices.retain(|_, device| {
        let age = now.signed_duration_since(device.last_seen_at);
        if age > ChronoDuration::minutes(10) {
            false
        } else {
            device.freshness = if age > ChronoDuration::seconds(60) {
                Freshness::Stale
            } else {
                Freshness::Fresh
            };
            true
        }
    });
}

fn logical_id_for_name(name: &str, namespace: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    format!("{namespace}-{}", hex::encode(digest))
}

fn ble_failure_code(failure: &BleFailure) -> &'static str {
    match failure.stage {
        BleFailureStage::Adapter => "adapter_unavailable",
        BleFailureStage::DeviceConnection => "device_connection_failed",
        BleFailureStage::ServiceResolution => "service_resolution_failed",
        BleFailureStage::GattDiscovery => "gatt_discovery_failed",
        BleFailureStage::IdentityVerification => "identity_verification_failed",
        BleFailureStage::GattWrite => {
            if failure.outcome_unknown {
                "outcome_unknown"
            } else {
                "gatt_write_error"
            }
        }
        BleFailureStage::BatteryDiscovery => "ba71_unsupported",
        BleFailureStage::BatteryRead => "battery_read_failed",
    }
}

fn device_snapshot(inner: &Inner) -> Device {
    let mut device = inner.simulator.clone();
    let (current, target) = device
        .position
        .as_ref()
        .map(|position| {
            (
                position.current_closed_percent,
                position.target_closed_percent,
            )
        })
        .unwrap_or((100, 100));
    let payload = simulated_manufacturer_payload(current, target);
    let mut manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
    manufacturer_data.extend_from_slice(&payload);
    device.manufacturer_data_hex = hex::encode(manufacturer_data);
    device.manufacturer_payload_hex = hex::encode(payload);
    device
}

async fn list_managed_devices(State(state): State<AppState>) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let mut devices: Vec<_> = inner
        .enrollments
        .values()
        .cloned()
        .map(|enrollment| managed_device(&inner, enrollment))
        .collect();
    devices.sort_by(|left, right| left.enrollment.label.cmp(&right.enrollment.label));
    Json(envelope(devices.clone(), Some(devices.len() as u64))).into_response()
}

async fn enroll_device(
    State(state): State<AppState>,
    payload: Result<Json<EnrollmentRequest>, JsonRejection>,
) -> Response {
    let request = match parse_json(payload) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let label = request.label.trim();
    if label.is_empty() || label.chars().count() > 64 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_label",
            "label must contain 1 to 64 characters.",
        );
    }
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let observed = if inner.simulator_enabled && request.observed_device_id == inner.simulator.id {
        Some(device_snapshot(&inner))
    } else {
        inner
            .devices
            .values()
            .find(|device| device.id == request.observed_device_id)
            .cloned()
    };
    let Some(observed) = observed else {
        return error_response(
            StatusCode::NOT_FOUND,
            "device_not_found",
            "The observed device was not found.",
        );
    };
    if observed.freshness != Freshness::Fresh {
        return error_response(
            StatusCode::CONFLICT,
            "device_not_ready",
            "A fresh device observation is required for enrollment.",
        );
    }
    let Some(logical_id) = observed.logical_id.clone() else {
        return error_response(
            StatusCode::CONFLICT,
            "identity_unavailable",
            "The observed device has no stable logical identity.",
        );
    };
    if inner
        .enrollments
        .values()
        .any(|entry| entry.logical_id == logical_id && entry.is_simulated == observed.is_simulated)
    {
        return error_response(
            StatusCode::CONFLICT,
            "duplicate_device",
            "This device is already enrolled.",
        );
    }
    if !observed.is_simulated
        && inner
            .devices
            .values()
            .filter(|device| {
                !device.is_simulated
                    && device.logical_id.as_deref() == Some(logical_id.as_str())
                    && device.freshness == Freshness::Fresh
            })
            .count()
            != 1
    {
        return error_response(
            StatusCode::CONFLICT,
            "ambiguous_device",
            "The advertised name matches more than one fresh device.",
        );
    }
    let enrollment = Enrollment {
        id: Uuid::new_v4().to_string(),
        label: label.to_owned(),
        advertised_name: observed.advertised_name,
        logical_id,
        is_simulated: observed.is_simulated,
        created_at: Utc::now(),
    };
    inner
        .enrollments
        .insert(enrollment.id.clone(), enrollment.clone());
    if state.persist(&inner).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "Enrollment may have been saved; refresh the managed-device list.",
        );
    }
    (StatusCode::CREATED, Json(envelope(enrollment, None))).into_response()
}

async fn remove_enrollment(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Response {
    let mut inner = state.inner.lock().await;
    if !inner.enrollments.contains_key(&id) {
        return error_response(
            StatusCode::NOT_FOUND,
            "enrollment_not_found",
            "Managed-device enrollment was not found.",
        );
    }
    if inner.commands.values().any(|command| {
        command.managed_device_id == id
            && matches!(
                command.status,
                CommandStatus::Queued | CommandStatus::Connecting
            )
    }) {
        return error_response(
            StatusCode::CONFLICT,
            "commands_pending",
            "The enrollment has queued or running commands.",
        );
    }
    inner.enrollments.remove(&id);
    if state.persist(&inner).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "Enrollment removal may have completed; refresh the managed-device list.",
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn queue_command(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
    payload: Result<Json<CommandRequest>, JsonRejection>,
) -> Response {
    let request = match parse_json(payload) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "idempotency_key_required",
            "Idempotency-Key must contain 1 to 128 characters.",
        );
    };
    match submit_command(&state, &id, request.action, key, None).await {
        Ok((command, is_new)) => {
            if is_new {
                tokio::spawn(process_command(state.clone(), command.id.clone()));
            }
            (StatusCode::ACCEPTED, Json(envelope(command, None))).into_response()
        }
        Err(error) => error.into_response(),
    }
}

async fn toggle_device(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let Some(key) = idempotency_key(&headers) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "idempotency_key_required",
            "Idempotency-Key must contain 1 to 128 characters.",
        );
    };
    let idempotency_id = format!("command:{id}:{key}");
    {
        let inner = state.inner.lock().await;
        if let Some(command_id) = inner.command_idempotency.get(&idempotency_id)
            && let Some(command) = inner.commands.get(command_id)
        {
            return (StatusCode::ACCEPTED, Json(envelope(command.clone(), None))).into_response();
        }
    }
    let selected = {
        let inner = state.inner.lock().await;
        resolve_toggle(&inner, &id)
    };
    let (action, basis, position, observed_at) = match selected {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    match submit_command(
        &state,
        &id,
        action,
        key,
        Some((basis, position, observed_at)),
    )
    .await
    {
        Ok((command, is_new)) => {
            if is_new {
                tokio::spawn(process_command(state.clone(), command.id.clone()));
            }
            (StatusCode::ACCEPTED, Json(envelope(command, None))).into_response()
        }
        Err(error) => error.into_response(),
    }
}

async fn queue_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<BatchCommandRequest>, JsonRejection>,
) -> Response {
    let request = match parse_json(payload) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "idempotency_key_required",
            "Idempotency-Key must contain 1 to 128 characters.",
        );
    };
    let batch = {
        let mut inner = state.inner.lock().await;
        let key_id = format!("batch:{key}");
        if let Some(existing_id) = inner.batch_idempotency.get(&key_id)
            && let Some(existing) = inner.batches.get(existing_id)
        {
            if existing.action == request.action {
                return (StatusCode::ACCEPTED, Json(envelope(existing.clone(), None)))
                    .into_response();
            }
            return error_response(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "Idempotency-Key was already used with a different batch action.",
            );
        }
        prune_journal(&mut inner);
        prune_batch_journal(&mut inner);
        let mut enrollments: Vec<_> = inner.enrollments.values().cloned().collect();
        enrollments.sort_by(|a, b| a.label.cmp(&b.label));
        let ready_count = enrollments
            .iter()
            .filter(|enrollment| {
                managed_device(&inner, (*enrollment).clone()).availability == "ready"
            })
            .count();
        if inner.commands.len().saturating_add(ready_count) > MAX_COMMANDS
            || inner.batches.len() >= MAX_BATCHES
        {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "batch_capacity_reached",
                "The command or batch journal is at capacity.",
            );
        }
        let mut targets = Vec::new();
        let mut skipped = Vec::new();
        for enrollment in enrollments {
            let managed = managed_device(&inner, enrollment.clone());
            if managed.availability == "ready" {
                let child_key = format!("batch:{key}:{}", enrollment.id);
                let command = make_command(
                    &enrollment.id,
                    request.action,
                    state.queue_config.queue_wait_timeout_seconds,
                );
                let command_id = command.id.clone();
                targets.push(BatchTarget {
                    managed_device_id: enrollment.id.clone(),
                    label: enrollment.label.clone(),
                    status: "pending".into(),
                    command_id: Some(command_id.clone()),
                    command: Some(command.clone()),
                    error_code: None,
                });
                inner.commands.insert(command_id.clone(), command);
                inner
                    .command_idempotency
                    .insert(format!("command:{}:{child_key}", enrollment.id), command_id);
            } else {
                skipped.push(BatchSkipped {
                    managed_device_id: enrollment.id,
                    label: enrollment.label,
                    availability: managed.availability,
                });
            }
        }
        if targets.is_empty() {
            return error_response(
                StatusCode::CONFLICT,
                "no_available_blinds",
                "No enrolled device is ready for control.",
            );
        }
        let now = Utc::now();
        let batch = Batch {
            id: Uuid::new_v4().to_string(),
            action: request.action,
            status: "queued".into(),
            accepted_at: now,
            started_at: None,
            completed_at: None,
            target_count: targets.len() as u64,
            skipped_count: skipped.len() as u64,
            acknowledged_count: 0,
            failed_count: 0,
            pending_count: targets.len() as u64,
            targets,
            skipped,
        };
        inner.batch_idempotency.insert(key_id, batch.id.clone());
        inner.batches.insert(batch.id.clone(), batch.clone());
        if state.persist(&inner).await.is_err() {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "persistence_outcome_unknown",
                "Batch may have been accepted; refresh the batch list through its returned resource.",
            );
        }
        batch
    };
    tokio::spawn(process_batch(state, batch.id.clone()));
    (StatusCode::ACCEPTED, Json(envelope(batch, None))).into_response()
}

async fn get_batch(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Response {
    let inner = state.inner.lock().await;
    match inner.batches.get(&id) {
        Some(batch) => Json(envelope(batch.clone(), None)).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            "batch_not_found",
            "Batch was not found.",
        ),
    }
}

async fn get_command(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Response {
    let inner = state.inner.lock().await;
    match inner.commands.get(&id) {
        Some(command) => Json(envelope(command.clone(), None)).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            "command_not_found",
            "Command was not found.",
        ),
    }
}

async fn get_queue(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().await;
    let pending: Vec<_> = inner
        .commands
        .values()
        .filter(|command| command.status == CommandStatus::Queued)
        .collect();
    let running = inner
        .commands
        .values()
        .filter(|command| command.status == CommandStatus::Connecting)
        .count();
    let oldest = pending.iter().map(|command| command.accepted_at).min();
    let data = QueueDiagnostics {
        pending_commands: pending.len() as u64,
        running_commands: running as u64,
        outstanding_commands: (pending.len() + running) as u64,
        oldest_queued_at: oldest,
        oldest_queued_age_seconds: oldest
            .map(|value| (Utc::now() - value).num_milliseconds().max(0) as f64 / 1000.0)
            .unwrap_or(0.0),
        max_outstanding_commands: state.queue_config.max_outstanding_commands as u64,
        max_queued_per_device: state.queue_config.max_queued_per_device as u64,
        adapter_concurrency: state.queue_config.adapter_concurrency as u64,
    };
    Json(envelope(data, None)).into_response()
}

#[derive(Debug)]
struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiFailure {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        error_response(self.status, self.code, &self.message)
    }
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    let key = headers.get("idempotency-key")?.to_str().ok()?.trim();
    (!key.is_empty() && key.chars().count() <= 128).then(|| key.to_owned())
}

fn make_command(
    managed_device_id: &str,
    action: Action,
    queue_wait_timeout_seconds: i64,
) -> Command {
    let now = Utc::now();
    Command {
        id: Uuid::new_v4().to_string(),
        managed_device_id: managed_device_id.to_owned(),
        action,
        toggle_basis: None,
        toggle_position: None,
        toggle_observed_at: None,
        status: CommandStatus::Queued,
        accepted_at: now,
        started_at: None,
        completed_at: None,
        expires_at: now + ChronoDuration::seconds(queue_wait_timeout_seconds),
        attempt: 0,
        error_code: None,
        failure_stage: None,
        write_state: WriteState::NotSent,
        retryable: true,
        duration_ms: None,
    }
}

fn prune_journal(inner: &mut Inner) {
    let cutoff = Utc::now() - ChronoDuration::hours(24);
    let mut terminal: Vec<_> = inner
        .commands
        .values()
        .filter_map(|command| {
            command
                .completed_at
                .filter(|_| {
                    !matches!(
                        command.status,
                        CommandStatus::Queued | CommandStatus::Connecting
                    )
                })
                .map(|completed_at| (completed_at, command.id.clone()))
        })
        .collect();
    terminal.sort_by_key(|(completed_at, _)| *completed_at);
    let mut remove: HashSet<String> = terminal
        .iter()
        .filter(|(completed_at, _)| *completed_at <= cutoff)
        .map(|(_, id)| id.clone())
        .collect();
    let mut remaining = inner.commands.len().saturating_sub(remove.len());
    for (_, id) in terminal {
        if remaining < MAX_COMMANDS {
            break;
        }
        if remove.insert(id) {
            remaining = remaining.saturating_sub(1);
        }
    }
    for id in &remove {
        inner.commands.remove(id);
    }
    let retained: HashSet<_> = inner.commands.keys().cloned().collect();
    inner
        .command_idempotency
        .retain(|_, command_id| retained.contains(command_id));
}

fn prune_batch_journal(inner: &mut Inner) {
    let cutoff = Utc::now() - ChronoDuration::hours(24);
    let mut terminal: Vec<_> = inner
        .batches
        .values()
        .filter_map(|batch| {
            batch
                .completed_at
                .filter(|_| !matches!(batch.status.as_str(), "queued" | "running"))
                .map(|completed_at| (completed_at, batch.id.clone()))
        })
        .collect();
    terminal.sort_by_key(|(completed_at, _)| *completed_at);
    let mut remove: HashSet<String> = terminal
        .iter()
        .filter(|(completed_at, _)| *completed_at <= cutoff)
        .map(|(_, id)| id.clone())
        .collect();
    let mut remaining = inner.batches.len().saturating_sub(remove.len());
    for (_, id) in terminal {
        if remaining < MAX_BATCHES {
            break;
        }
        if remove.insert(id) {
            remaining = remaining.saturating_sub(1);
        }
    }
    for id in &remove {
        inner.batches.remove(id);
    }
    let retained: HashSet<_> = inner.batches.keys().cloned().collect();
    inner
        .batch_idempotency
        .retain(|_, batch_id| retained.contains(batch_id));
}

async fn submit_command(
    state: &AppState,
    managed_device_id: &str,
    action: Action,
    key: String,
    toggle: Option<(String, ControlPosition, DateTime<Utc>)>,
) -> Result<(Command, bool), ApiFailure> {
    let mut inner = state.inner.lock().await;
    let key_id = format!("command:{managed_device_id}:{key}");
    if let Some(command_id) = inner.command_idempotency.get(&key_id)
        && let Some(command) = inner.commands.get(command_id)
    {
        if command.action == action {
            return Ok((command.clone(), false));
        }
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "Idempotency-Key was already used with a different action.",
        ));
    }
    prune_journal(&mut inner);
    if inner.commands.len() >= MAX_COMMANDS {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "command_journal_full",
            "The command journal is at capacity.",
        ));
    }
    let Some(enrollment) = inner.enrollments.get(managed_device_id).cloned() else {
        return Err(ApiFailure::new(
            StatusCode::NOT_FOUND,
            "enrollment_not_found",
            "Managed-device enrollment was not found.",
        ));
    };
    if enrollment.is_simulated && !inner.simulator_enabled {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "device_unavailable",
            "The simulated device is unavailable.",
        ));
    }
    let managed = managed_device(&inner, enrollment);
    if managed.availability != "ready" {
        let (code, message) = if managed.availability == "ambiguous" {
            (
                "ambiguous_device",
                "The enrolled name matches more than one fresh device.",
            )
        } else {
            (
                "device_not_ready",
                "A unique fresh device observation is required for control.",
            )
        };
        return Err(ApiFailure::new(StatusCode::CONFLICT, code, message));
    }
    let outstanding: Vec<_> = inner
        .commands
        .values()
        .filter(|command| {
            matches!(
                command.status,
                CommandStatus::Queued | CommandStatus::Connecting
            )
        })
        .collect();
    if outstanding.len() >= state.queue_config.max_outstanding_commands {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "queue_capacity_reached",
            "The command queue is at capacity.",
        ));
    }
    let per_device = outstanding
        .iter()
        .filter(|command| command.managed_device_id == managed_device_id)
        .count();
    if per_device >= state.queue_config.max_queued_per_device {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "device_queue_full",
            "This device already has the maximum number of outstanding commands.",
        ));
    }
    if action == Action::Stop {
        for command in inner.commands.values_mut().filter(|command| {
            command.managed_device_id == managed_device_id
                && command.status == CommandStatus::Queued
                && matches!(command.action, Action::Open | Action::Close)
        }) {
            command.status = CommandStatus::Cancelled;
            command.completed_at = Some(Utc::now());
            command.error_code = Some("preempted_by_stop".into());
            command.retryable = false;
        }
    }
    let mut command = make_command(
        managed_device_id,
        action,
        state.queue_config.queue_wait_timeout_seconds,
    );
    if let Some((basis, position, observed_at)) = toggle {
        command.toggle_basis = Some(basis);
        command.toggle_position = Some(position);
        command.toggle_observed_at = Some(observed_at);
    }
    inner.command_idempotency.insert(key_id, command.id.clone());
    inner.commands.insert(command.id.clone(), command.clone());
    if state.persist(&inner).await.is_err() {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "The command may have been accepted; query its resource before retrying.",
        ));
    }
    Ok((command, true))
}

fn resolve_toggle(
    inner: &Inner,
    managed_device_id: &str,
) -> Result<(Action, String, ControlPosition, DateTime<Utc>), ApiFailure> {
    let Some(enrollment) = inner.enrollments.get(managed_device_id) else {
        return Err(ApiFailure::new(
            StatusCode::NOT_FOUND,
            "enrollment_not_found",
            "Managed-device enrollment was not found.",
        ));
    };
    if enrollment.is_simulated && !inner.simulator_enabled {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "device_unavailable",
            "A unique fresh observation is not available.",
        ));
    }
    let device = if enrollment.is_simulated {
        inner
            .simulator
            .logical_id
            .as_deref()
            .filter(|logical_id| *logical_id == enrollment.logical_id)
            .map(|_| &inner.simulator)
    } else {
        let matching: Vec<_> = inner
            .devices
            .values()
            .filter(|device| {
                !device.is_simulated
                    && device.logical_id.as_deref() == Some(enrollment.logical_id.as_str())
                    && device.advertised_name == enrollment.advertised_name
                    && device.freshness == Freshness::Fresh
            })
            .collect();
        if matching.len() > 1 {
            return Err(ApiFailure::new(
                StatusCode::CONFLICT,
                "ambiguous_device",
                "The enrolled name matches more than one fresh device.",
            ));
        }
        matching.first().copied()
    };
    let Some(device) = device.filter(|device| {
        device.freshness == Freshness::Fresh
            && Utc::now().signed_duration_since(device.last_seen_at) <= ChronoDuration::seconds(60)
    }) else {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "device_not_ready",
            "A unique fresh observation is required to toggle this blind.",
        ));
    };
    let Some(position) = device.position.as_ref() else {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "position_unavailable",
            "A position observation is required to select a toggle action.",
        ));
    };
    let position_snapshot = ControlPosition {
        current_closed_percent: position.current_closed_percent,
        target_closed_percent: position.target_closed_percent,
    };
    let observed_at = position.observed_at;
    let mut latest_intent: Option<&Command> = None;
    for command in inner
        .commands
        .values()
        .filter(|command| command.managed_device_id == managed_device_id)
        .filter(|command| {
            matches!(
                command.status,
                CommandStatus::Queued
                    | CommandStatus::Connecting
                    | CommandStatus::WriteAcknowledged
                    | CommandStatus::WriteFailed
                    | CommandStatus::OutcomeUnknown
            )
        })
    {
        if latest_intent.is_none_or(|current| current.accepted_at < command.accepted_at) {
            latest_intent = Some(command);
        }
    }
    if let Some(command) = latest_intent {
        if command.action == Action::Stop
            && matches!(
                command.status,
                CommandStatus::Queued | CommandStatus::Connecting
            )
        {
            return Err(ApiFailure::new(
                StatusCode::CONFLICT,
                "toggle_pending_stop",
                "A stop command is pending.",
            ));
        }
        if matches!(
            command.status,
            CommandStatus::OutcomeUnknown | CommandStatus::WriteFailed
        ) && command.write_state == WriteState::PossiblySent
            && command.accepted_at >= observed_at
        {
            return Err(ApiFailure::new(
                StatusCode::CONFLICT,
                "toggle_outcome_unknown",
                "Check for a newer position observation before toggling again.",
            ));
        }
        if matches!(
            command.status,
            CommandStatus::Queued | CommandStatus::Connecting
        ) && matches!(command.action, Action::Open | Action::Close)
        {
            let next = opposite(command.action);
            return Ok((
                next,
                "command_intent".into(),
                position_snapshot,
                observed_at,
            ));
        }
        if command.status == CommandStatus::WriteAcknowledged
            && position.current_closed_percent != position.target_closed_percent
        {
            let next = opposite(command.action);
            return Ok((
                next,
                "command_intent".into(),
                position_snapshot,
                observed_at,
            ));
        }
    }
    let action = if position.target_closed_percent < 50 {
        Action::Close
    } else {
        Action::Open
    };
    Ok((
        action,
        "observed_target".into(),
        position_snapshot,
        observed_at,
    ))
}

fn opposite(action: Action) -> Action {
    match action {
        Action::Open => Action::Close,
        Action::Close => Action::Open,
        Action::Stop => Action::Stop,
    }
}

async fn process_command(state: AppState, command_id: String) {
    let managed_device_id = {
        let inner = state.inner.lock().await;
        let Some(command) = inner.commands.get(&command_id) else {
            return;
        };
        command.managed_device_id.clone()
    };
    let lock = {
        let mut locks = state.device_locks.lock().await;
        locks
            .entry(managed_device_id.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    };
    let Ok(_device_permit) = lock.acquire_owned().await else {
        return;
    };
    let Ok(_adapter_permit) = state.adapter_slots.clone().acquire_owned().await else {
        return;
    };
    let (action, scenario, connection_delay_ms, write_delay_ms, is_simulated) = {
        let mut inner = state.inner.lock().await;
        let is_simulated = inner
            .enrollments
            .get(&managed_device_id)
            .is_some_and(|enrollment| enrollment.is_simulated);
        let Some(command) = inner.commands.get_mut(&command_id) else {
            return;
        };
        if command.status != CommandStatus::Queued {
            return;
        }
        if Utc::now() >= command.expires_at {
            command.status = CommandStatus::Expired;
            command.completed_at = Some(Utc::now());
            command.retryable = true;
            let _ = state.persist(&inner).await;
            return;
        }
        command.status = CommandStatus::Connecting;
        command.started_at = Some(Utc::now());
        command.attempt = 1;
        let action = command.action;
        let scenario = inner.simulator_config.scenario;
        let connection_delay_ms = inner.simulator_config.connection_delay_ms;
        let write_delay_ms = inner.simulator_config.write_response_delay_ms;
        let _ = state.persist(&inner).await;
        (
            action,
            scenario,
            connection_delay_ms,
            write_delay_ms,
            is_simulated,
        )
    };
    if !is_simulated {
        process_physical_command(&state, &command_id, action).await;
        return;
    }
    tokio::time::sleep(std::time::Duration::from_millis(
        connection_delay_ms.saturating_add(write_delay_ms),
    ))
    .await;

    let mut inner = state.inner.lock().await;
    let now = Utc::now();
    if matches!(
        scenario,
        SimulatorScenario::PreWriteFailure
            | SimulatorScenario::OutcomeUnknown
            | SimulatorScenario::GattWriteError
    ) {
        inner.simulator_config.scenario = SimulatorScenario::Normal;
    }
    if scenario == SimulatorScenario::TelemetryPaused {
        inner.simulator_config.telemetry_paused = true;
    }
    let update_simulator_telemetry = matches!(
        scenario,
        SimulatorScenario::Normal | SimulatorScenario::OutcomeUnknown
    );
    let simulator_position = if matches!(
        scenario,
        SimulatorScenario::Normal
            | SimulatorScenario::OutcomeUnknown
            | SimulatorScenario::TelemetryPaused
    ) && inner.simulator_enabled
    {
        inner.simulator.position.as_mut().map(|position| {
            let target = action.target_closed_percent(position.current_closed_percent);
            position.target_closed_percent = target;
            if update_simulator_telemetry {
                position.observed_at = now;
            }
            (position.current_closed_percent, target)
        })
    } else {
        None
    };
    if let Some((current, target)) = simulator_position {
        inner.simulator.movement_status = movement_status(current, target);
        if update_simulator_telemetry {
            inner.simulator.last_seen_at = now;
        }
    }
    if let Some(command) = inner.commands.get_mut(&command_id) {
        command.duration_ms = Some(
            now.signed_duration_since(command.started_at.unwrap_or(command.accepted_at))
                .num_milliseconds()
                .max(0) as u64,
        );
        command.completed_at = Some(now);
        match scenario {
            SimulatorScenario::PreWriteFailure => {
                command.status = CommandStatus::ConnectionFailed;
                command.error_code = Some("connection_failed".into());
                command.failure_stage = Some("device_connection".into());
                command.write_state = WriteState::NotSent;
                command.retryable = true;
            }
            SimulatorScenario::OutcomeUnknown => {
                command.status = CommandStatus::OutcomeUnknown;
                command.error_code = Some("outcome_unknown".into());
                command.failure_stage = Some("gatt_write".into());
                command.write_state = WriteState::PossiblySent;
                command.retryable = false;
            }
            SimulatorScenario::GattWriteError => {
                command.status = CommandStatus::WriteFailed;
                command.error_code = Some("gatt_write_error".into());
                command.failure_stage = Some("gatt_write".into());
                command.write_state = WriteState::PossiblySent;
                command.retryable = false;
            }
            SimulatorScenario::Normal | SimulatorScenario::TelemetryPaused => {
                command.status = CommandStatus::WriteAcknowledged;
                command.write_state = WriteState::Acknowledged;
                command.retryable = false;
            }
        }
    }
    if inner.simulator_enabled
        && matches!(
            scenario,
            SimulatorScenario::Normal | SimulatorScenario::OutcomeUnknown
        )
    {
        if record_history(&mut inner).unwrap_or(false) {
            state.history_revision.fetch_add(1, Ordering::AcqRel);
        }
        let history_revision = state.history_revision.load(Ordering::Acquire);
        if state
            .persist_history(&inner.history, history_revision)
            .await
            .is_err()
        {
            tracing::warn!("Could not persist simulator telemetry history.");
        }
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Command result could not be persisted.");
    }
}

fn physical_target(
    inner: &Inner,
    managed_device_id: &str,
) -> Result<(String, String), &'static str> {
    let Some(enrollment) = inner.enrollments.get(managed_device_id) else {
        return Err("enrollment_not_found");
    };
    if enrollment.is_simulated {
        return Err("device_not_ready");
    }
    let now = Utc::now();
    let matches: Vec<_> = inner
        .devices
        .values()
        .filter(|device| {
            !device.is_simulated
                && device.logical_id.as_deref() == Some(enrollment.logical_id.as_str())
                && device.advertised_name == enrollment.advertised_name
                && device.freshness == Freshness::Fresh
                && now.signed_duration_since(device.last_seen_at) <= ChronoDuration::seconds(60)
        })
        .collect();
    match matches.as_slice() {
        [] => Err("device_not_ready"),
        [device] => Ok((
            device.adapter_device_id.clone(),
            enrollment.advertised_name.clone(),
        )),
        _ => Err("ambiguous_device"),
    }
}

async fn perform_physical_action(
    adapter: &dyn BleAdapter,
    peripheral_id: &str,
    expected_name: &str,
    action: ServiceAction,
) -> Result<(), BleFailure> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(45),
        adapter.write_action(peripheral_id, expected_name, action),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(BleFailure {
            stage: BleFailureStage::GattWrite,
            retryable_before_write: false,
            write_may_have_been_sent: true,
            outcome_unknown: true,
        }),
    }
}

async fn process_physical_command(state: &AppState, command_id: &str, action: Action) {
    let (managed_device_id, started_at) = {
        let inner = state.inner.lock().await;
        let Some(command) = inner.commands.get(command_id) else {
            return;
        };
        (
            command.managed_device_id.clone(),
            command.started_at.unwrap_or(command.accepted_at),
        )
    };
    let Some(adapter) = state.ble_adapter.read().await.clone() else {
        complete_physical_failure(
            state,
            command_id,
            BleFailure {
                stage: BleFailureStage::Adapter,
                retryable_before_write: true,
                write_may_have_been_sent: false,
                outcome_unknown: false,
            },
            "adapter_unavailable",
            started_at,
        )
        .await;
        return;
    };

    let target = {
        let inner = state.inner.lock().await;
        physical_target(&inner, &managed_device_id)
    };
    let (mut peripheral_id, expected_name) = match target {
        Ok(target) => target,
        Err(code) => {
            complete_physical_failure(
                state,
                command_id,
                BleFailure {
                    stage: BleFailureStage::IdentityVerification,
                    retryable_before_write: true,
                    write_may_have_been_sent: false,
                    outcome_unknown: false,
                },
                code,
                started_at,
            )
            .await;
            return;
        }
    };
    let service_action = match action {
        Action::Open => ServiceAction::Open,
        Action::Close => ServiceAction::Close,
        Action::Stop => ServiceAction::Stop,
    };
    let mut result = perform_physical_action(
        adapter.as_ref(),
        &peripheral_id,
        &expected_name,
        service_action,
    )
    .await;
    let mut attempt = 1;
    if result.as_ref().is_err_and(|failure| {
        failure.retryable_before_write
            && matches!(
                failure.stage,
                BleFailureStage::DeviceConnection | BleFailureStage::ServiceResolution
            )
    }) {
        let retry_target = {
            let inner = state.inner.lock().await;
            physical_target(&inner, &managed_device_id)
        };
        match retry_target {
            Ok((retry_peripheral_id, _)) => {
                peripheral_id = retry_peripheral_id;
                attempt = 2;
                if let Some(command) = state.inner.lock().await.commands.get_mut(command_id) {
                    command.attempt = attempt;
                }
                let persist_result = {
                    let inner = state.inner.lock().await;
                    state.persist(&inner).await
                };
                let _ = persist_result;
                result = perform_physical_action(
                    adapter.as_ref(),
                    &peripheral_id,
                    &expected_name,
                    service_action,
                )
                .await;
            }
            Err(code) => {
                complete_physical_failure(
                    state,
                    command_id,
                    BleFailure {
                        stage: BleFailureStage::IdentityVerification,
                        retryable_before_write: true,
                        write_may_have_been_sent: false,
                        outcome_unknown: false,
                    },
                    code,
                    started_at,
                )
                .await;
                return;
            }
        }
    }

    let mut inner = state.inner.lock().await;
    let Some(command) = inner.commands.get_mut(command_id) else {
        return;
    };
    let now = Utc::now();
    command.attempt = attempt;
    command.duration_ms = Some(
        now.signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64,
    );
    command.completed_at = Some(now);
    match result {
        Ok(()) => {
            command.status = CommandStatus::WriteAcknowledged;
            command.write_state = WriteState::Acknowledged;
            command.retryable = false;
            command.error_code = None;
            command.failure_stage = None;
        }
        Err(failure) => {
            command.status = if failure.write_may_have_been_sent {
                if failure.outcome_unknown {
                    CommandStatus::OutcomeUnknown
                } else {
                    CommandStatus::WriteFailed
                }
            } else {
                CommandStatus::ConnectionFailed
            };
            command.write_state = if failure.write_may_have_been_sent {
                WriteState::PossiblySent
            } else {
                WriteState::NotSent
            };
            command.retryable = failure.retryable_before_write && !failure.write_may_have_been_sent;
            command.error_code = Some(ble_failure_code(&failure).into());
            command.failure_stage = Some(failure.stage.as_str().into());
        }
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Physical command result could not be persisted.");
    }
}

async fn complete_physical_failure(
    state: &AppState,
    command_id: &str,
    failure: BleFailure,
    error_code: &str,
    started_at: DateTime<Utc>,
) {
    let mut inner = state.inner.lock().await;
    let Some(command) = inner.commands.get_mut(command_id) else {
        return;
    };
    let now = Utc::now();
    command.status = CommandStatus::ConnectionFailed;
    command.completed_at = Some(now);
    command.duration_ms = Some(
        now.signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64,
    );
    command.error_code = Some(error_code.to_owned());
    command.failure_stage = Some(failure.stage.as_str().into());
    command.write_state = WriteState::NotSent;
    command.retryable = failure.retryable_before_write;
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Physical command failure could not be persisted.");
    }
}

async fn process_batch(state: AppState, batch_id: String) {
    let command_ids = {
        let mut inner = state.inner.lock().await;
        let Some(batch) = inner.batches.get_mut(&batch_id) else {
            return;
        };
        batch.status = "running".into();
        batch.started_at = Some(Utc::now());
        let ids = batch
            .targets
            .iter()
            .filter_map(|target| target.command_id.clone())
            .collect::<Vec<_>>();
        let _ = state.persist(&inner).await;
        ids
    };
    for command_id in command_ids {
        process_command(state.clone(), command_id.clone()).await;
        let mut inner = state.inner.lock().await;
        let command = inner.commands.get(&command_id).cloned();
        if let Some(batch) = inner.batches.get_mut(&batch_id) {
            if let Some(target) = batch
                .targets
                .iter_mut()
                .find(|target| target.command_id.as_deref() == Some(command_id.as_str()))
                && let Some(command) = command
            {
                target.status = command_status_text(command.status).into();
                target.error_code = command.error_code.clone();
                target.command = Some(command);
            }
            batch.acknowledged_count = batch
                .targets
                .iter()
                .filter(|target| target.status == "write_acknowledged")
                .count() as u64;
            batch.failed_count = batch
                .targets
                .iter()
                .filter(|target| {
                    matches!(
                        target.status.as_str(),
                        "connection_failed"
                            | "write_failed"
                            | "outcome_unknown"
                            | "expired"
                            | "cancelled"
                    )
                })
                .count() as u64;
            batch.pending_count = batch
                .targets
                .iter()
                .filter(|target| {
                    matches!(target.status.as_str(), "pending" | "queued" | "connecting")
                })
                .count() as u64;
            if batch.pending_count == 0 {
                batch.status = if batch.acknowledged_count == batch.target_count {
                    "completed".into()
                } else if batch.acknowledged_count == 0 {
                    "failed".into()
                } else {
                    "partial".into()
                };
                batch.completed_at = Some(Utc::now());
            }
            let _ = state.persist(&inner).await;
        }
    }
}

fn managed_device(inner: &Inner, enrollment: Enrollment) -> ManagedDevice {
    let matching: Vec<&Device> = if enrollment.is_simulated {
        inner
            .simulator_enabled
            .then_some(&inner.simulator)
            .filter(|device| device.logical_id.as_deref() == Some(enrollment.logical_id.as_str()))
            .into_iter()
            .collect()
    } else {
        inner
            .devices
            .values()
            .filter(|device| {
                !device.is_simulated
                    && device.logical_id.as_deref() == Some(enrollment.logical_id.as_str())
                    && device.advertised_name == enrollment.advertised_name
            })
            .collect()
    };
    if matching.len() > 1 {
        return ManagedDevice {
            enrollment,
            availability: "ambiguous".into(),
            last_seen_at: matching.iter().map(|device| device.last_seen_at).max(),
            position: None,
        };
    }
    match matching.first().copied() {
        Some(device) => {
            let fresh = device.freshness == Freshness::Fresh
                && Utc::now().signed_duration_since(device.last_seen_at)
                    <= ChronoDuration::seconds(60);
            ManagedDevice {
                enrollment,
                availability: if fresh { "ready" } else { "stale" }.into(),
                last_seen_at: Some(device.last_seen_at),
                position: device.position.as_ref().map(|position| ControlPosition {
                    current_closed_percent: position.current_closed_percent,
                    target_closed_percent: position.target_closed_percent,
                }),
            }
        }
        None => ManagedDevice {
            enrollment,
            availability: "not_seen".into(),
            last_seen_at: None,
            position: None,
        },
    }
}

fn movement_status(current: u8, target: u8) -> MovementStatus {
    if current == target {
        MovementStatus::AtTarget
    } else if target > current {
        MovementStatus::Closing
    } else {
        MovementStatus::Opening
    }
}

fn command_status_text(status: CommandStatus) -> &'static str {
    match status {
        CommandStatus::Queued => "queued",
        CommandStatus::Connecting => "connecting",
        CommandStatus::ConnectionFailed => "connection_failed",
        CommandStatus::WriteAcknowledged => "write_acknowledged",
        CommandStatus::WriteFailed => "write_failed",
        CommandStatus::OutcomeUnknown => "outcome_unknown",
        CommandStatus::Expired => "expired",
        CommandStatus::Cancelled => "cancelled",
    }
}
