use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration as StdDuration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use crate::{
    ble::{
        BleAdapter, BleCommandControl, BleFailure, BleFailureStage, BleObservation,
        BtleplugAdapter, SOMA_MANUFACTURER_ID, decode_ba71_voltage, parse_soma_payload,
    },
    controller::ServiceAction,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
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
static OPENAPI_JSON_ETAG: OnceLock<String> = OnceLock::new();
static DOCS_ETAG: OnceLock<String> = OnceLock::new();
static OPENAPI_YAML: OnceLock<String> = OnceLock::new();
static OPENAPI_YAML_ETAG: OnceLock<String> = OnceLock::new();
const DOCS_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Somaris API reference</title>
  <style>
    :root { color-scheme: light dark; font: 16px/1.5 system-ui, sans-serif; }
    body { margin: 0 auto; max-width: 960px; padding: 2rem 1.25rem; }
    h1 { margin-bottom: .25rem; }
    nav { margin: 1rem 0 2rem; }
    a { color: #1683d8; }
    .operation { border-top: 1px solid #8886; padding: 1rem 0; }
    .method { display: inline-block; min-width: 4.5rem; font-weight: 700; }
    code { overflow-wrap: anywhere; }
    .description { margin: .5rem 0 0 5rem; white-space: pre-wrap; }
    #status { color: #a33; }
  </style>
</head>
<body>
  <header>
    <h1>OpenAPI reference</h1>
    <p>Somaris API operations and descriptions.</p>
    <nav><a href="/openapi.json">OpenAPI JSON</a> · <a href="/openapi.yaml">OpenAPI YAML</a></nav>
  </header>
  <p id="status" role="status"></p>
  <main id="operations"></main>
  <script>
    const methods = ["get", "post", "put", "patch", "delete"];
    fetch("/openapi.json")
      .then(response => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then(spec => {
        document.title = `${spec.info.title} reference`;
        document.querySelector("h1").textContent = `${spec.info.title} reference`;
        const root = document.querySelector("#operations");
        for (const [path, pathItem] of Object.entries(spec.paths)) {
          for (const method of methods) {
            const operation = pathItem[method];
            if (!operation) continue;
            const row = document.createElement("section");
            row.className = "operation";
            const heading = document.createElement("div");
            const verb = document.createElement("span");
            verb.className = "method";
            verb.textContent = method.toUpperCase();
            const route = document.createElement("code");
            route.textContent = path;
            heading.append(verb, route);
            row.append(heading);
            if (operation.summary || operation.description) {
              const description = document.createElement("p");
              description.className = "description";
              description.textContent = operation.summary || operation.description;
              row.append(description);
            }
            root.append(row);
          }
        }
      })
      .catch(error => {
        document.querySelector("#status").textContent = `Could not load the OpenAPI document: ${error.message}`;
      });
  </script>
</body>
</html>
"##;
const SIMULATOR_ID: &str = "simulator-1";
const SECOND_SIMULATOR_ID: &str = "simulator-2";
const SIMULATOR_NAME: &str = "Somaris Simulator 1";
const SECOND_SIMULATOR_NAME: &str = "Somaris Simulator 2";
const MAX_COMMANDS: usize = 1000;
const MAX_BATCHES: usize = 1000;
const MAX_DISCOVERED_DEVICES: usize = 1024;
const MAX_AUDIT_ENTRIES: usize = 30;
const SCAN_RESTART_DELAY: std::time::Duration = std::time::Duration::from_millis(50);
const COMMAND_EXECUTION_TIMEOUT: StdDuration = StdDuration::from_secs(45);
const COMMAND_CLEANUP_GRACE: StdDuration = StdDuration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueConfig {
    pub max_outstanding_commands: usize,
    pub max_queued_per_device: usize,
    pub adapter_concurrency: usize,
    pub adapter_scan_window_ms: u64,
    pub queue_wait_timeout_seconds: i64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_outstanding_commands: 1000,
            max_queued_per_device: 32,
            adapter_concurrency: 1,
            adapter_scan_window_ms: 5_000,
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
        if !(100..=10_000).contains(&self.adapter_scan_window_ms) {
            return Err("adapter_scan_window_ms must be between 100 and 10000".into());
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
            adapter_scan_window_ms: env_positive_u64(
                "SOMARIS_ADAPTER_SCAN_WINDOW_MS",
                defaults.adapter_scan_window_ms,
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

fn env_positive_u64(name: &str, default: u64) -> io::Result<u64> {
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    let parsed = value.parse::<u64>().ok().filter(|value| *value > 0);
    parsed.ok_or_else(|| invalid_configuration(format!("{name} must be a positive integer")))
}

fn invalid_configuration(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<Inner>>,
    device_locks: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    command_controls: Arc<Mutex<HashMap<String, BleCommandControl>>>,
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
    history_persisted_cursor: Arc<AtomicU64>,
    history_persisted_count: Arc<AtomicUsize>,
    data_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Inner {
    simulator_enabled: bool,
    simulator: Device,
    simulator_config: SimulatorConfig,
    #[serde(default)]
    simulator_two: Option<Device>,
    #[serde(default)]
    simulator_two_config: Option<SimulatorConfig>,
    enrollments: HashMap<String, Enrollment>,
    commands: HashMap<String, Command>,
    command_idempotency: HashMap<String, String>,
    batches: HashMap<String, Batch>,
    batch_idempotency: HashMap<String, String>,
    #[serde(default)]
    audit_log: VecDeque<AuditEntry>,
    history: VecDeque<HistoryObservation>,
    #[serde(skip)]
    latest_history_by_device: HashMap<String, HistoryObservation>,
    next_cursor: u64,
    scanner: ScannerCounters,
    #[serde(skip)]
    devices: HashMap<String, Device>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    fn pending_history(&self, history: &VecDeque<HistoryObservation>) -> Vec<HistoryObservation> {
        let cursor = self.history_persisted_cursor.load(Ordering::Acquire);
        let first = history.partition_point(|record| record.cursor <= cursor);
        history.iter().skip(first).cloned().collect()
    }

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
        let simulator = make_simulated_device(SIMULATOR_ID, SIMULATOR_NAME, started_at);
        let simulator_two = Some(make_simulated_device(
            SECOND_SIMULATOR_ID,
            SECOND_SIMULATOR_NAME,
            started_at,
        ));
        let mut inner = Inner {
            simulator_enabled,
            simulator,
            simulator_config: SimulatorConfig::default(),
            simulator_two,
            simulator_two_config: Some(SimulatorConfig::default()),
            enrollments: HashMap::new(),
            commands: HashMap::new(),
            command_idempotency: HashMap::new(),
            batches: HashMap::new(),
            batch_idempotency: HashMap::new(),
            audit_log: VecDeque::new(),
            history: VecDeque::new(),
            latest_history_by_device: HashMap::new(),
            next_cursor: 1,
            scanner: ScannerCounters::unavailable(),
            devices: HashMap::new(),
        };
        if let Some(path) = &history_path {
            inner.history = read_history(path).unwrap_or_default().into();
            inner.next_cursor = inner
                .history
                .back()
                .map(|record| record.cursor.saturating_add(1))
                .unwrap_or(1);
        }
        inner.latest_history_by_device = inner
            .history
            .iter()
            .map(|record| (record.device_id.clone(), record.clone()))
            .collect();
        let history_persisted_cursor = inner.history.back().map_or(0, |record| record.cursor);
        let history_persisted_count = inner.history.len();
        let queue_config = QueueConfig::default();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            device_locks: Arc::new(Mutex::new(HashMap::new())),
            command_controls: Arc::new(Mutex::new(HashMap::new())),
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
            history_persisted_cursor: Arc::new(AtomicU64::new(history_persisted_cursor)),
            history_persisted_count: Arc::new(AtomicUsize::new(history_persisted_count)),
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
        let mut file = open_private_file(path, false)?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)?;
        let mut persisted: Inner = serde_json::from_slice(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let now = Utc::now();
        initialize_simulators(&mut persisted, now);
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
                if let Some(attempt) = command
                    .attempts
                    .iter_mut()
                    .rev()
                    .find(|attempt| attempt.completed_at.is_none())
                {
                    attempt.phase = "service_recovery".into();
                    attempt.completed_at = Some(now);
                    attempt.duration_ms = Some(
                        now.signed_duration_since(attempt.started_at)
                            .num_milliseconds()
                            .max(0) as u64,
                    );
                    attempt.write_state = WriteState::PossiblySent;
                    attempt.failure_stage = Some("service_recovery".into());
                    attempt.error_code = Some("outcome_unknown".into());
                }
            }
        }
        let completed_commands: Vec<_> = persisted
            .commands
            .values()
            .filter_map(|command| {
                command.completed_at.map(|completed_at| {
                    (
                        command.id.clone(),
                        command_status_text(command.status).to_owned(),
                        completed_at,
                    )
                })
            })
            .collect();
        for (id, status, completed_at) in completed_commands {
            update_activity_status(&mut persisted, &id, &status, completed_at);
        }
        trim_audit_log(&mut persisted);
        if persisted.simulator_enabled {
            let primary_config = persisted.simulator_config;
            refresh_simulator_after_start(&mut persisted.simulator, &primary_config, now);
            let second_config = persisted.simulator_two_config;
            if let (Some(device), Some(config)) =
                (persisted.simulator_two.as_mut(), second_config.as_ref())
            {
                refresh_simulator_after_start(device, config, now);
            }
        }
        *guard = persisted;
        guard.latest_history_by_device = guard
            .history
            .iter()
            .map(|record| (record.device_id.clone(), record.clone()))
            .collect();
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
        inner.history = history.into();
        inner.latest_history_by_device = inner
            .history
            .iter()
            .map(|record| (record.device_id.clone(), record.clone()))
            .collect();
        self.history_persisted_cursor.store(
            inner.history.back().map_or(0, |record| record.cursor),
            Ordering::Release,
        );
        self.history_persisted_count
            .store(inner.history.len(), Ordering::Release);
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
        // Coalesce nearby scanner and simulator updates. A newer revision owns
        // the flush and carries every not-yet-persisted cursor.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _persist_guard = self.history_persist_lock.lock().await;
        if self.history_revision.load(Ordering::Acquire) != revision {
            return Ok(());
        }
        let persisted_cursor = self.history_persisted_cursor.load(Ordering::Acquire);
        let first_pending = history.partition_point(|record| record.cursor <= persisted_cursor);
        let pending = &history[first_pending..];
        if pending.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        for record in pending {
            serde_json::to_writer(&mut bytes, record).map_err(|_| ())?;
            bytes.push(b'\n');
        }
        let path = path.clone();
        let append_path = path.clone();
        tokio::task::spawn_blocking(move || append_and_sync(&append_path, &bytes))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let final_cursor = pending
            .last()
            .map_or(persisted_cursor, |record| record.cursor);
        let persisted_count = self
            .history_persisted_count
            .fetch_add(pending.len(), Ordering::AcqRel)
            + pending.len();
        self.history_persisted_cursor
            .store(final_cursor, Ordering::Release);

        // Keep an append-only hot path, and compact only after a bounded amount
        // of history has accumulated beyond the in-memory retention window.
        if persisted_count > 51_000 {
            let compact_path = path.clone();
            let compacted_count = tokio::task::spawn_blocking(move || {
                let mut records = read_history(&compact_path)?;
                if records.len() > 50_000 {
                    let remove = records.len() - 50_000;
                    records.drain(0..remove);
                    let mut compacted = Vec::new();
                    for record in &records {
                        serde_json::to_writer(&mut compacted, record).map_err(io::Error::other)?;
                        compacted.push(b'\n');
                    }
                    atomic_write(&compact_path, &compacted)?;
                }
                Ok::<usize, io::Error>(records.len())
            })
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
            self.history_persisted_count
                .store(compacted_count, Ordering::Release);
        }
        Ok(())
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
            self.command_controls
                .lock()
                .await
                .insert(id.clone(), BleCommandControl::new());
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
                    if !inner.simulator_enabled {
                        continue;
                    }
                    let now = Utc::now();
                    let primary_config = inner.simulator_config;
                    if !primary_config.telemetry_paused {
                        advance_simulator(&mut inner.simulator, &primary_config, now);
                        inner.simulator.last_seen_at = now;
                    }
                    let second_config = inner.simulator_two_config;
                    if let (Some(device), Some(config)) =
                        (inner.simulator_two.as_mut(), second_config.as_ref())
                        && !config.telemetry_paused
                    {
                        advance_simulator(device, config, now);
                        device.last_seen_at = now;
                    }
                    let recorded = record_history(&mut inner).unwrap_or(false);
                    if recorded {
                        let revision = state.history_revision.fetch_add(1, Ordering::AcqRel) + 1;
                        Some((state.pending_history(&inner.history), revision))
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
            let scan_permits = self
                .adapter_slots
                .clone()
                .acquire_many_owned(self.queue_config.adapter_concurrency as u32)
                .await;
            let Ok(scan_permit) = scan_permits else {
                tracing::error!("BLE adapter coordination semaphore closed; scanner stopped");
                return;
            };
            let (sender, mut receiver) = mpsc::channel(128);
            {
                let mut inner = self.inner.lock().await;
                inner.scanner.status = "scanning".into();
                inner.scanner.adapter_available = true;
                inner.scanner.scan_active = true;
                inner.scanner.last_scan_started_at = Some(Utc::now());
                inner.scanner.last_error_code = None;
            }
            let scan_window =
                std::time::Duration::from_millis(self.queue_config.adapter_scan_window_ms);
            let scan = tokio::spawn(async move { adapter.scan(sender, scan_window).await });
            while let Some(observation) = receiver.recv().await {
                self.store_ble_observation(observation).await;
            }
            let failure = match scan.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(error) => {
                    tracing::error!(%error, "BLE scan task terminated unexpectedly");
                    Some(BleFailure {
                        stage: BleFailureStage::Adapter,
                        retryable_before_write: true,
                        write_may_have_been_sent: false,
                        outcome_unknown: false,
                    })
                }
            };
            if failure.is_none() {
                let mut inner = self.inner.lock().await;
                inner.scanner.scan_active = false;
                inner.scanner.status = "available".into();
                inner.scanner.adapter_available = true;
                inner.scanner.last_error_code = None;
                drop(inner);
                drop(scan_permit);
                tokio::time::sleep(SCAN_RESTART_DELAY).await;
                continue;
            }
            if let Some(failure) = &failure {
                tracing::warn!(
                    stage = failure.stage.as_str(),
                    error_code = ble_failure_code(failure),
                    "BLE scan window failed"
                );
            }
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
                inner.scanner.adapter_available = false;
            }
            drop(scan_permit);
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
            inner.scanner.last_advertisement_at = Some(now);
            prune_registry(&mut inner, now);
            let mut rebound_device = None;
            if !inner.devices.contains_key(&observation.peripheral_id)
                && inner.enrollments.values().any(|enrollment| {
                    !enrollment.is_simulated && enrollment.logical_id == logical_id
                })
            {
                if inner.devices.values().any(|device| {
                    device.freshness == Freshness::Fresh
                        && device.logical_id.as_deref() == Some(logical_id.as_str())
                }) {
                    return;
                }
                let stale_ids: Vec<_> = inner
                    .devices
                    .iter()
                    .filter(|(_, device)| device.logical_id.as_deref() == Some(logical_id.as_str()))
                    .map(|(peripheral_id, _)| peripheral_id.clone())
                    .collect();
                rebound_device = stale_ids
                    .iter()
                    .filter_map(|peripheral_id| inner.devices.get(peripheral_id))
                    .max_by_key(|device| device.last_seen_at)
                    .cloned();
                for peripheral_id in stale_ids {
                    inner.devices.remove(&peripheral_id);
                }
            }
            if inner.devices.len() >= MAX_DISCOVERED_DEVICES
                && !inner.devices.contains_key(&observation.peripheral_id)
            {
                let Some(evicted_ids) = oldest_evictable_devices(&inner, logical_id.as_str())
                else {
                    return;
                };
                for evicted_id in evicted_ids {
                    inner.devices.remove(&evicted_id);
                }
            }
            inner.scanner.observations_stored = inner.scanner.observations_stored.saturating_add(1);
            let previous = inner
                .devices
                .get(&observation.peripheral_id)
                .cloned()
                .or(rebound_device);
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
            let mut history_changed = record_history_for_device(&mut inner, &device);
            inner.devices.insert(observation.peripheral_id, device);
            if inner.simulator_enabled {
                let simulators = simulator_snapshots(&inner);
                for simulator in &simulators {
                    history_changed |= record_history_for_device(&mut inner, simulator);
                }
            }
            prune_history(&mut inner);
            if history_changed {
                let revision = self.history_revision.fetch_add(1, Ordering::AcqRel) + 1;
                Some((self.pending_history(&inner.history), revision))
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
                        let due = battery_read_due(&device.battery, now);
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
                tracing::warn!(device = %expected_name, stage = failure.stage.as_str(), "Battery telemetry attempt failed");
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
        .route("/", get(ui_index))
        .route("/ui/style.css", get(ui_style))
        .route("/ui/components.css", get(ui_components_style))
        .route("/ui/panel.css", get(ui_panel_style))
        .route("/ui/app.mjs", get(ui_app))
        .route("/ui/components.mjs", get(ui_components))
        .route("/ui/favicon.svg", get(ui_favicon_svg))
        .route("/ui/favicon-32x32.png", get(ui_favicon_png))
        .route("/ui/apple-touch-icon.png", get(ui_apple_touch_icon))
        .route("/api/v1/health", get(health).options(preflight))
        .route("/api/v1/ready", get(ready).options(preflight))
        .route("/api/v1/diagnostics", get(diagnostics).options(preflight))
        .route("/api/v1/devices", get(list_devices).options(preflight))
        .route("/api/v1/devices/{id}", get(get_device).options(preflight))
        .route("/api/v1/history", get(get_history).options(preflight))
        .route(
            "/api/control/v1/audit-log",
            get(get_audit_log).options(preflight),
        )
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
            "/api/dev/simulators",
            get(list_simulators).options(preflight),
        )
        .route(
            "/api/dev/simulators/{id}/scenario",
            put(set_named_simulator_scenario).options(preflight),
        )
        .route(
            "/api/dev/simulators/{id}/reset",
            post(reset_named_simulator).options(preflight),
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
        .route("/docs", get(api_docs))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            allowed_origins,
            enforce_cors_contract,
        ))
}

async fn ui_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../ui/index.html"),
    )
}

async fn ui_style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../ui/style.css"),
    )
}

async fn ui_components_style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../ui/components.css"),
    )
}

async fn ui_panel_style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../ui/panel.css"),
    )
}

async fn ui_app() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../ui/app.mjs"),
    )
}

async fn ui_components() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../ui/components.mjs"),
    )
}

async fn ui_favicon_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        include_str!("../ui/favicon.svg"),
    )
}

async fn ui_favicon_png() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        Bytes::from_static(include_bytes!("../ui/favicon-32x32.png")),
    )
}

async fn ui_apple_touch_icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        Bytes::from_static(include_bytes!("../ui/apple-touch-icon.png")),
    )
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn enforce_cors_contract(
    State(allowed_origins): State<Vec<HeaderValue>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let request_path = request.uri().path().to_owned();
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
    if request_path.starts_with("/api/") && !response.headers().contains_key(header::CACHE_CONTROL)
    {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

async fn openapi_json(headers: HeaderMap) -> Response {
    static_response(
        &headers,
        OPENAPI_JSON.as_bytes(),
        "application/json; charset=utf-8",
        OPENAPI_JSON_ETAG.get_or_init(|| strong_etag(OPENAPI_JSON.as_bytes())),
    )
}

async fn api_docs(headers: HeaderMap) -> Response {
    static_response(
        &headers,
        DOCS_HTML.as_bytes(),
        "text/html; charset=utf-8",
        DOCS_ETAG.get_or_init(|| strong_etag(DOCS_HTML.as_bytes())),
    )
}

async fn openapi_yaml(headers: HeaderMap) -> Response {
    let yaml = OPENAPI_YAML.get_or_init(|| {
        serde_json::from_str::<Value>(OPENAPI_JSON)
            .ok()
            .and_then(|value| serde_yaml::to_string(&value).ok())
            .unwrap_or_default()
    });
    if yaml.is_empty() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    static_response(
        &headers,
        yaml.as_bytes(),
        "application/yaml; charset=utf-8",
        OPENAPI_YAML_ETAG.get_or_init(|| strong_etag(yaml.as_bytes())),
    )
}

fn strong_etag(bytes: &[u8]) -> String {
    format!("\"{}\"", hex::encode(Sha256::digest(bytes)))
}

fn static_response(
    headers: &HeaderMap,
    bytes: &'static [u8],
    content_type: &'static str,
    etag: &str,
) -> Response {
    if etag_matches(headers, etag) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, etag)
            .header(header::CACHE_CONTROL, "public, max-age=3600")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(Bytes::from_static(bytes)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn health(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().await;
    let scanner_status = inner.scanner.status.clone();
    let status = if inner.scanner.adapter_available {
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
    let ready = inner.scanner.adapter_available;
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

async fn diagnostics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let retained = inner.devices.len()
        + if inner.simulator_enabled {
            simulator_count(&inner)
        } else {
            0
        };
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
    let validator = DiagnosticsValidator {
        service_started_at: data.service_started_at,
        history_enabled: data.history_enabled,
        scanner: &data.scanner,
        registry: &data.registry,
    };
    conditional_json(&headers, validator, envelope(&data, None))
}

async fn list_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let mut devices: Vec<_> = inner.devices.values().cloned().collect();
    if inner.simulator_enabled {
        devices.extend(simulator_snapshots(&inner));
    }
    devices.sort_by_key(|device| std::cmp::Reverse(device.last_seen_at));
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
        .then(|| simulator_snapshots(&inner))
        .into_iter()
        .flatten()
        .find(|device| device.id == id)
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
        .back()
        .map(|record| record.cursor)
        .unwrap_or(0);
    let after = query.after.unwrap_or(0);
    let first = inner
        .history
        .partition_point(|record| record.cursor <= after);
    let records: Vec<_> = inner
        .history
        .iter()
        .skip(first)
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
    let more_start = inner
        .history
        .partition_point(|record| record.cursor <= next_cursor);
    let has_more = inner.history.iter().skip(more_start).any(|record| {
        since.is_none_or(|since| record.observed_at >= since)
            && query
                .device_id
                .as_deref()
                .is_none_or(|id| id == record.device_id || record.logical_id.as_deref() == Some(id))
    });
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
    let validator = HistoryValidator {
        records: &data.data,
        count: data.meta.count,
        next_cursor: data.meta.next_cursor,
        latest_cursor: data.meta.latest_cursor,
        has_more: data.meta.has_more,
    };
    conditional_json(&headers, validator, &data)
}

async fn get_audit_log(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let inner = state.inner.lock().await;
    let entries: Vec<_> = inner
        .audit_log
        .iter()
        .rev()
        .take(MAX_AUDIT_ENTRIES)
        .cloned()
        .collect();
    conditional_json(
        &headers,
        entries.clone(),
        envelope(entries.clone(), Some(entries.len() as u64)),
    )
}

async fn get_simulator(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let inner = state.inner.lock().await;
    if !inner.simulator_enabled {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_disabled",
            "The simulator is disabled.",
        );
    }
    let data = simulator_by_id(&inner, SIMULATOR_ID)
        .map(|(device, config)| simulator_status(device, config))
        .expect("the primary simulator is configured");
    conditional_json(&headers, data.clone(), envelope(data, None))
}

async fn list_simulators(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let inner = state.inner.lock().await;
    if !inner.simulator_enabled {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_disabled",
            "The simulator is disabled.",
        );
    }
    let simulators = simulator_statuses(&inner);
    conditional_json(
        &headers,
        simulators.clone(),
        envelope(simulators.clone(), Some(simulators.len() as u64)),
    )
}

async fn set_simulator_scenario(
    State(state): State<AppState>,
    payload: Result<Json<SimulatorScenarioRequest>, JsonRejection>,
) -> Response {
    set_simulator_scenario_for(state, SIMULATOR_ID, payload).await
}

async fn set_named_simulator_scenario(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<SimulatorScenarioRequest>, JsonRejection>,
) -> Response {
    set_simulator_scenario_for(state, &id, payload).await
}

async fn set_simulator_scenario_for(
    state: AppState,
    id: &str,
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
    let Some((device, config)) = simulator_by_id_mut(&mut inner, id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_not_found",
            "The simulator was not found.",
        );
    };
    if let Some(scenario) = request.scenario {
        config.scenario = scenario;
        if scenario == SimulatorScenario::Normal {
            config.telemetry_paused = false;
        } else if scenario == SimulatorScenario::TelemetryPaused {
            config.telemetry_paused = true;
        }
    }
    if let Some(profile) = request.wireless_profile {
        config.wireless_profile = profile;
        (config.connection_delay_ms, config.write_response_delay_ms) = match profile {
            WirelessProfile::Standard => (0, 0),
            WirelessProfile::HallwayObserved => (80, 72),
            WirelessProfile::HallwaySlowLink => (702, 72),
        };
        device.rssi_dbm = simulator_rssi(profile, Utc::now());
    }
    if let Some(battery_state) = request.battery_state {
        config.battery_state = battery_state;
        device.battery = simulator_battery_telemetry(battery_state, Utc::now());
    }
    let subject = device.advertised_name.clone();
    let detail = [
        request
            .scenario
            .map(|value| format!("Scenario: {}", serialized_variant(value))),
        request
            .wireless_profile
            .map(|value| format!("Wireless profile: {}", serialized_variant(value))),
        request
            .battery_state
            .map(|value| format!("Battery: {}", serialized_variant(value))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");
    record_activity(
        &mut inner,
        None,
        ActivityKind::SimulatorSettingsChanged,
        Some(subject),
        None,
        None,
        Some(detail),
    );
    if state.persist(&inner).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "Simulator settings may have changed; refresh simulator status.",
        );
    }
    let data = simulator_by_id(&inner, id)
        .map(|(device, config)| simulator_status(device, config))
        .expect("the simulator remains configured after updating it");
    Json(envelope(data, None)).into_response()
}

async fn reset_simulator(
    State(state): State<AppState>,
    payload: Result<Json<SimulatorResetRequest>, JsonRejection>,
) -> Response {
    reset_simulator_for(state, SIMULATOR_ID, payload).await
}

async fn reset_named_simulator(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<SimulatorResetRequest>, JsonRejection>,
) -> Response {
    reset_simulator_for(state, &id, payload).await
}

async fn reset_simulator_for(
    state: AppState,
    id: &str,
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
    let Some((device, config)) = simulator_by_id_mut(&mut inner, id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "simulator_not_found",
            "The simulator was not found.",
        );
    };
    let position = request.position_closed_percent.unwrap_or(100);
    if position > 100 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_position",
            "position_closed_percent must be from 0 to 100.",
        );
    }
    config.scenario = SimulatorScenario::Normal;
    config.telemetry_paused = false;
    let now = Utc::now();
    device.position = Some(position_value(position, position, now, Source::Simulator));
    device.movement_status = MovementStatus::AtTarget;
    device.last_seen_at = now;
    let subject = device.advertised_name.clone();
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
    record_activity(
        &mut inner,
        None,
        ActivityKind::SimulatorReset,
        Some(subject),
        None,
        None,
        Some(format!("Reset to {position}% closed")),
    );
    let history_revision = if history_changed {
        state.history_revision.fetch_add(1, Ordering::AcqRel) + 1
    } else {
        state.history_revision.load(Ordering::Acquire)
    };
    let pending_history = state.pending_history(&inner.history);
    if state
        .persist_history(&pending_history, history_revision)
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
    let data = simulator_by_id(&inner, id)
        .map(|(device, config)| simulator_status(device, config))
        .expect("the simulator remains configured after resetting it");
    Json(envelope(data, None)).into_response()
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

fn battery_read_due(battery: &BatteryTelemetry, now: DateTime<Utc>) -> bool {
    let interval = if battery.status == BatteryStatus::Unavailable {
        ChronoDuration::minutes(5)
    } else {
        ChronoDuration::hours(1)
    };
    battery
        .last_attempt_at
        .is_none_or(|attempt| now.signed_duration_since(attempt) >= interval)
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

#[derive(Serialize)]
struct DiagnosticsValidator<'a> {
    service_started_at: DateTime<Utc>,
    history_enabled: bool,
    scanner: &'a ScannerDiagnostics,
    registry: &'a RegistryDiagnostics,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActivityKind {
    EnrollmentAdded,
    EnrollmentRemoved,
    Command,
    Batch,
    SimulatorSettingsChanged,
    SimulatorReset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuditEntry {
    id: String,
    occurred_at: DateTime<Utc>,
    kind: ActivityKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<DateTime<Utc>>,
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

#[derive(Serialize)]
struct HistoryValidator<'a> {
    records: &'a [HistoryObservation],
    count: u64,
    next_cursor: u64,
    latest_cursor: u64,
    has_more: bool,
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
    id: String,
    advertised_name: String,
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
    gatt_readiness: Option<GattReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<ControlPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GattReadiness {
    /// The most recent command outcome; historical evidence, not a live connection probe.
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_command_accepted_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_attempt_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_write_acknowledged_at: Option<DateTime<Utc>>,
    attempt_count: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
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
    #[serde(default)]
    attempts: Vec<CommandAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    write_state: WriteState,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandAttempt {
    number: u8,
    phase: String,
    started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    write_state: WriteState,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
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
    adapter_scan_window_ms: u64,
    command_execution_timeout_seconds: u64,
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
    // The response envelope includes a fresh generated_at timestamp, while the
    // validator covers only semantic data. It is therefore a weak validator.
    let opaque_etag = format!("\"{}\"", hex::encode(Sha256::digest(serialized)));
    let etag = format!("W/{opaque_etag}");
    if etag_matches(headers, &etag) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, etag)
            .header(header::CACHE_CONTROL, "private, no-cache")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let mut response = Json(response_value).into_response();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    response
}

fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    let opaque_etag = etag.strip_prefix("W/").unwrap_or(etag);
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|candidate| {
                let candidate = candidate.trim();
                candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == opaque_etag
            })
        })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", Uuid::new_v4()));
    let temporary = PathBuf::from(temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;

    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn append_and_sync(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let existed = path.exists();
    {
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let mut file = options.open(path)?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_data()?;
    }
    if !existed {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn open_private_file(path: &Path, write: bool) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(write);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "persistent state paths must point to regular files",
        ));
    }
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn read_history(path: &Path) -> io::Result<Vec<HistoryObservation>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = open_private_file(path, true)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    if !content.is_empty() && !content.ends_with('\n') {
        let valid_len = content.rfind('\n').map_or(0, |index| index + 1);
        content.truncate(valid_len);
        file.set_len(valid_len as u64)?;
    }
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .collect()
}

fn make_simulated_device(id: &str, name: &str, now: DateTime<Utc>) -> Device {
    let payload = simulated_manufacturer_payload(name, 100, 100);
    let mut manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
    manufacturer_data.extend_from_slice(&payload);
    Device {
        id: id.to_owned(),
        logical_id: Some(logical_id_for_name(name, "simulator")),
        observed_address: Some(format!("simulator://{id}")),
        adapter_device_id: format!("simulator://{id}"),
        device_name: Some("S".to_owned()),
        advertised_name: name.to_owned(),
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

fn initialize_simulators(inner: &mut Inner, now: DateTime<Utc>) {
    let previous_primary_logical_id = inner.simulator.logical_id.clone();
    retarget_simulated_device(&mut inner.simulator, SIMULATOR_ID, SIMULATOR_NAME);
    if inner.simulator_two.is_none() {
        inner.simulator_two = Some(make_simulated_device(
            SECOND_SIMULATOR_ID,
            SECOND_SIMULATOR_NAME,
            now,
        ));
    } else if let Some(device) = inner.simulator_two.as_mut() {
        retarget_simulated_device(device, SECOND_SIMULATOR_ID, SECOND_SIMULATOR_NAME);
    }
    if inner.simulator_two_config.is_none() {
        inner.simulator_two_config = Some(SimulatorConfig::default());
    }
    if let Some(previous_logical_id) = previous_primary_logical_id {
        let current_logical_id = logical_id_for_name(SIMULATOR_NAME, "simulator");
        for record in inner
            .history
            .iter_mut()
            .filter(|record| record.device_id == SIMULATOR_ID)
        {
            record.logical_id = Some(current_logical_id.clone());
        }
        for enrollment in inner
            .enrollments
            .values_mut()
            .filter(|entry| entry.is_simulated)
        {
            if enrollment.logical_id == previous_logical_id {
                enrollment.advertised_name = SIMULATOR_NAME.to_owned();
                enrollment.logical_id = current_logical_id.clone();
            }
        }
    }
}

fn retarget_simulated_device(device: &mut Device, id: &str, name: &str) {
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
    let payload = simulated_manufacturer_payload(name, current, target);
    let mut manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
    manufacturer_data.extend_from_slice(&payload);
    device.id = id.to_owned();
    device.logical_id = Some(logical_id_for_name(name, "simulator"));
    device.observed_address = Some(format!("simulator://{id}"));
    device.adapter_device_id = format!("simulator://{id}");
    device.advertised_name = name.to_owned();
    device.source = Source::Simulator;
    device.is_simulated = true;
    device.manufacturer_data_hex = hex::encode(manufacturer_data);
    device.manufacturer_payload_hex = hex::encode(payload);
}

fn refresh_simulator_after_start(
    device: &mut Device,
    config: &SimulatorConfig,
    now: DateTime<Utc>,
) {
    device.freshness = Freshness::Fresh;
    device.last_seen_at = now;
    if let Some(position) = device.position.as_mut() {
        position.observed_at = now;
    }
    device.battery = simulator_battery_telemetry(config.battery_state, now);
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

fn simulator_by_id<'a>(inner: &'a Inner, id: &str) -> Option<(&'a Device, &'a SimulatorConfig)> {
    match id {
        SIMULATOR_ID => Some((&inner.simulator, &inner.simulator_config)),
        SECOND_SIMULATOR_ID => inner
            .simulator_two
            .as_ref()
            .zip(inner.simulator_two_config.as_ref()),
        _ => None,
    }
}

fn simulator_by_id_mut<'a>(
    inner: &'a mut Inner,
    id: &str,
) -> Option<(&'a mut Device, &'a mut SimulatorConfig)> {
    match id {
        SIMULATOR_ID => Some((&mut inner.simulator, &mut inner.simulator_config)),
        SECOND_SIMULATOR_ID => inner
            .simulator_two
            .as_mut()
            .zip(inner.simulator_two_config.as_mut()),
        _ => None,
    }
}

fn simulator_id_for_logical_id(inner: &Inner, logical_id: &str) -> Option<&'static str> {
    [SIMULATOR_ID, SECOND_SIMULATOR_ID].into_iter().find(|id| {
        simulator_by_id(inner, id)
            .is_some_and(|(device, _)| device.logical_id.as_deref() == Some(logical_id))
    })
}

fn simulator_count(inner: &Inner) -> usize {
    1 + usize::from(inner.simulator_two.is_some())
}

fn simulator_snapshots(inner: &Inner) -> Vec<Device> {
    [SIMULATOR_ID, SECOND_SIMULATOR_ID]
        .into_iter()
        .filter_map(|id| simulator_by_id(inner, id).map(|(device, _)| device_snapshot(device)))
        .collect()
}

fn simulator_statuses(inner: &Inner) -> Vec<SimulatorStatus> {
    [SIMULATOR_ID, SECOND_SIMULATOR_ID]
        .into_iter()
        .filter_map(|id| {
            simulator_by_id(inner, id).map(|(device, config)| simulator_status(device, config))
        })
        .collect()
}

fn simulator_status(device: &Device, config: &SimulatorConfig) -> SimulatorStatus {
    let position = device.position.as_ref();
    SimulatorStatus {
        id: device.id.clone(),
        advertised_name: device.advertised_name.clone(),
        scenario: config.scenario,
        wireless_profile: config.wireless_profile,
        battery_state: config.battery_state,
        connection_delay_ms: config.connection_delay_ms,
        write_response_delay_ms: config.write_response_delay_ms,
        telemetry_paused: config.telemetry_paused,
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
        devices.extend(simulator_snapshots(inner));
    }
    let mut changed = false;
    for device in devices {
        changed |= record_history_for_device(inner, &device);
    }
    prune_history(inner);
    Ok(changed)
}

fn record_history_for_device(inner: &mut Inner, device: &Device) -> bool {
    let current_position = device.position.as_ref().map(|position| {
        (
            position.current_closed_percent,
            position.target_closed_percent,
        )
    });
    let latest = inner.latest_history_by_device.get(&device.id);
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
            device.last_seen_at.signed_duration_since(last.observed_at) < ChronoDuration::seconds(5)
        })
    {
        return false;
    }
    let record = HistoryObservation {
        cursor: inner.next_cursor,
        device_id: device.id.clone(),
        logical_id: device.logical_id.clone(),
        observed_at: device.last_seen_at,
        rssi_dbm: device.rssi_dbm,
        position_status: device.position_status,
        position: device.position.as_ref().map(|position| HistoryPosition {
            current_closed_percent: position.current_closed_percent,
            target_closed_percent: position.target_closed_percent,
        }),
    };
    inner.history.push_back(record.clone());
    inner
        .latest_history_by_device
        .insert(record.device_id.clone(), record);
    inner.next_cursor = inner.next_cursor.saturating_add(1);
    true
}

fn prune_history(inner: &mut Inner) {
    if inner.history.len() > 50_000 {
        let remove = inner.history.len() - 50_000;
        for _ in 0..remove {
            let Some(record) = inner.history.pop_front() else {
                break;
            };
            if inner
                .latest_history_by_device
                .get(&record.device_id)
                .is_some_and(|latest| latest.cursor == record.cursor)
            {
                inner.latest_history_by_device.remove(&record.device_id);
            }
        }
    }
}

fn record_activity(
    inner: &mut Inner,
    id: Option<String>,
    kind: ActivityKind,
    subject: Option<String>,
    action: Option<Action>,
    status: Option<String>,
    detail: Option<String>,
) {
    inner.audit_log.push_back(AuditEntry {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        occurred_at: Utc::now(),
        kind,
        subject,
        action,
        status,
        detail,
        completed_at: None,
    });
    trim_audit_log(inner);
}

fn serialized_variant<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(value)) => value,
        _ => "unknown".into(),
    }
}

fn update_activity_status(inner: &mut Inner, id: &str, status: &str, completed_at: DateTime<Utc>) {
    set_activity_status(inner, id, status, Some(completed_at));
}

fn set_activity_status(
    inner: &mut Inner,
    id: &str,
    status: &str,
    completed_at: Option<DateTime<Utc>>,
) {
    if let Some(entry) = inner
        .audit_log
        .iter_mut()
        .rev()
        .find(|entry| entry.id == id)
    {
        entry.status = Some(status.to_owned());
        entry.completed_at = completed_at;
    }
}

fn trim_audit_log(inner: &mut Inner) {
    while inner.audit_log.len() > MAX_AUDIT_ENTRIES {
        inner.audit_log.pop_front();
    }
}

fn advance_simulator(device: &mut Device, config: &SimulatorConfig, now: DateTime<Utc>) {
    let Some((current, target)) = device.position.as_mut().map(|position| {
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
    device.movement_status = movement_status(current, target);
    device.rssi_dbm = simulator_rssi(config.wireless_profile, now);
}

fn simulator_rssi(profile: WirelessProfile, now: DateTime<Utc>) -> i16 {
    const HALLWAY_RSSI_SAMPLES: [i16; 26] = [
        -82, -85, -87, -71, -83, -67, -67, -90, -85, -85, -86, -85, -88, -85, -87, -86, -83, -83,
        -83, -82, -84, -84, -84, -85, -84, -82,
    ];
    match profile {
        WirelessProfile::Standard => -60,
        WirelessProfile::HallwayObserved | WirelessProfile::HallwaySlowLink => {
            let index = now
                .timestamp()
                .rem_euclid(HALLWAY_RSSI_SAMPLES.len() as i64) as usize;
            HALLWAY_RSSI_SAMPLES[index]
        }
    }
}

fn simulated_manufacturer_payload(
    advertised_name: &str,
    current_closed_percent: u8,
    target_closed_percent: u8,
) -> Vec<u8> {
    // Byte 1 is retained as the opaque 0x64 value seen in the live advertisement.
    let mut payload = vec![0x05, 0x64, current_closed_percent, target_closed_percent];
    payload.extend_from_slice(advertised_name.as_bytes());
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

fn oldest_evictable_devices(inner: &Inner, incoming_logical_id: &str) -> Option<Vec<String>> {
    let mut fresh_device_counts = HashMap::new();
    for device in inner
        .devices
        .values()
        .filter(|device| device.freshness == Freshness::Fresh)
    {
        if let Some(logical_id) = device.logical_id.as_deref() {
            *fresh_device_counts.entry(logical_id).or_insert(0usize) += 1;
        }
    }
    let enrolled_logical_ids: HashSet<_> = inner
        .enrollments
        .values()
        .filter(|enrollment| !enrollment.is_simulated)
        .map(|enrollment| enrollment.logical_id.as_str())
        .collect();

    let safe_candidate = inner
        .devices
        .iter()
        .filter(|(_, device)| {
            let Some(logical_id) = device.logical_id.as_deref() else {
                return true;
            };
            if enrolled_logical_ids.contains(logical_id) {
                return false;
            }
            if device.freshness != Freshness::Fresh {
                return true;
            }
            logical_id != incoming_logical_id && fresh_device_counts.get(logical_id) == Some(&1)
        })
        .min_by_key(|(_, device)| device.last_seen_at)
        .map(|(peripheral_id, _)| peripheral_id.clone());
    if let Some(peripheral_id) = safe_candidate {
        return Some(vec![peripheral_id]);
    }

    let mut ambiguous_groups: HashMap<&str, Vec<&str>> = HashMap::new();
    for (peripheral_id, device) in &inner.devices {
        let Some(logical_id) = device.logical_id.as_deref() else {
            continue;
        };
        if device.freshness == Freshness::Fresh
            && logical_id != incoming_logical_id
            && !enrolled_logical_ids.contains(logical_id)
            && fresh_device_counts
                .get(logical_id)
                .is_some_and(|count| *count > 1)
        {
            ambiguous_groups
                .entry(logical_id)
                .or_default()
                .push(peripheral_id);
        }
    }
    ambiguous_groups
        .into_iter()
        .min_by_key(|(_, peripheral_ids)| {
            peripheral_ids
                .iter()
                .filter_map(|peripheral_id| {
                    inner
                        .devices
                        .get(*peripheral_id)
                        .map(|device| device.last_seen_at)
                })
                .min()
        })
        .map(|(_, peripheral_ids)| peripheral_ids.into_iter().map(str::to_owned).collect())
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
        BleFailureStage::CommandDeadline => "command_timeout",
        BleFailureStage::CommandPreempted => "preempted_by_stop",
        BleFailureStage::BatteryDiscovery => "ba71_unsupported",
        BleFailureStage::BatteryRead => "battery_read_failed",
    }
}

fn device_snapshot(simulator: &Device) -> Device {
    let mut device = simulator.clone();
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
    let payload = simulated_manufacturer_payload(&device.advertised_name, current, target);
    let mut manufacturer_data = SOMA_MANUFACTURER_ID.to_le_bytes().to_vec();
    manufacturer_data.extend_from_slice(&payload);
    device.manufacturer_data_hex = hex::encode(manufacturer_data);
    device.manufacturer_payload_hex = hex::encode(payload);
    device
}

async fn list_managed_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut inner = state.inner.lock().await;
    prune_registry(&mut inner, Utc::now());
    let mut devices: Vec<_> = inner
        .enrollments
        .values()
        .cloned()
        .map(|enrollment| managed_device(&inner, enrollment))
        .collect();
    devices.sort_by(|left, right| left.enrollment.label.cmp(&right.enrollment.label));
    conditional_json(
        &headers,
        devices.clone(),
        envelope(devices.clone(), Some(devices.len() as u64)),
    )
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
    let observed = if inner.simulator_enabled {
        simulator_snapshots(&inner)
            .into_iter()
            .find(|device| device.id == request.observed_device_id)
    } else {
        None
    }
    .or_else(|| {
        inner
            .devices
            .values()
            .find(|device| device.id == request.observed_device_id)
            .cloned()
    });
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
    record_activity(
        &mut inner,
        None,
        ActivityKind::EnrollmentAdded,
        Some(enrollment.label.clone()),
        None,
        None,
        None,
    );
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
    let enrollment = inner
        .enrollments
        .remove(&id)
        .expect("the enrollment was checked above");
    record_activity(
        &mut inner,
        None,
        ActivityKind::EnrollmentRemoved,
        Some(enrollment.label),
        None,
        None,
        None,
    );
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
            let command = command_snapshot(&state, &command.id)
                .await
                .unwrap_or(command);
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
    let existing = {
        let inner = state.inner.lock().await;
        if let Some(command_id) = inner.command_idempotency.get(&idempotency_id)
            && let Some(command) = inner.commands.get(command_id)
        {
            Some(command.clone())
        } else {
            None
        }
    };
    if let Some(command) = existing {
        let command = command_snapshot(&state, &command.id)
            .await
            .unwrap_or(command);
        return (StatusCode::ACCEPTED, Json(envelope(command, None))).into_response();
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
            let command = command_snapshot(&state, &command.id)
                .await
                .unwrap_or(command);
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
        record_activity(
            &mut inner,
            Some(batch.id.clone()),
            ActivityKind::Batch,
            Some(format!("{} ready blinds", batch.target_count)),
            Some(batch.action),
            Some(batch.status.clone()),
            (batch.skipped_count > 0)
                .then(|| format!("{} unavailable blinds skipped", batch.skipped_count)),
        );
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

async fn get_batch(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let inner = state.inner.lock().await;
    match inner.batches.get(&id) {
        Some(batch) => conditional_json(&headers, batch.clone(), envelope(batch.clone(), None)),
        None => error_response(
            StatusCode::NOT_FOUND,
            "batch_not_found",
            "Batch was not found.",
        ),
    }
}

async fn get_command(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    match command_snapshot(&state, &id).await {
        Some(command) => conditional_json(&headers, command.clone(), envelope(command, None)),
        None => error_response(
            StatusCode::NOT_FOUND,
            "command_not_found",
            "Command was not found.",
        ),
    }
}

async fn command_snapshot(state: &AppState, command_id: &str) -> Option<Command> {
    let inner = state.inner.lock().await;
    let mut command = inner.commands.get(command_id).cloned()?;
    if command.status == CommandStatus::Connecting {
        // Keep the command state stable while checking whether its live control crossed the write boundary.
        let controls = state.command_controls.lock().await;
        if controls
            .get(command_id)
            .is_some_and(BleCommandControl::write_started)
        {
            command.write_state = WriteState::PossiblySent;
            command.retryable = false;
            if let Some(attempt) = command.attempts.last_mut() {
                attempt.phase = "gatt_write".into();
                attempt.write_state = WriteState::PossiblySent;
            }
        }
    }
    Some(command)
}

async fn get_queue(State(state): State<AppState>, headers: HeaderMap) -> Response {
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
        adapter_scan_window_ms: state.queue_config.adapter_scan_window_ms,
        command_execution_timeout_seconds: COMMAND_EXECUTION_TIMEOUT.as_secs(),
    };
    conditional_json(&headers, data.clone(), envelope(data, None))
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
        attempts: Vec::new(),
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
    let subject = enrollment.label.clone();
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
    let mut preempt_in_flight = Vec::new();
    let mut cancelled_queued = Vec::new();
    if action == Action::Stop {
        let mut cancelled = Vec::new();
        for command in inner.commands.values_mut().filter(|command| {
            command.managed_device_id == managed_device_id
                && command.status == CommandStatus::Queued
                && matches!(command.action, Action::Open | Action::Close)
        }) {
            command.status = CommandStatus::Cancelled;
            command.completed_at = Some(Utc::now());
            command.error_code = Some("preempted_by_stop".into());
            command.retryable = false;
            cancelled_queued.push(command.id.clone());
            cancelled.push((
                command.id.clone(),
                command.completed_at.unwrap_or_else(Utc::now),
            ));
        }
        for (id, completed_at) in cancelled {
            update_activity_status(&mut inner, &id, "cancelled", completed_at);
        }
        preempt_in_flight.extend(
            inner
                .commands
                .values()
                .filter(|command| {
                    command.managed_device_id == managed_device_id
                        && command.status == CommandStatus::Connecting
                        && command.write_state == WriteState::NotSent
                        && matches!(command.action, Action::Open | Action::Close)
                })
                .map(|command| command.id.clone()),
        );
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
    record_activity(
        &mut inner,
        Some(command.id.clone()),
        ActivityKind::Command,
        Some(subject),
        Some(action),
        Some(command_status_text(command.status).to_owned()),
        None,
    );
    if state.persist(&inner).await.is_err() {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "persistence_outcome_unknown",
            "The command may have been accepted; query its resource before retrying.",
        ));
    }
    state
        .command_controls
        .lock()
        .await
        .insert(command.id.clone(), BleCommandControl::new());
    drop(inner);
    if !preempt_in_flight.is_empty() || !cancelled_queued.is_empty() {
        let mut controls = state.command_controls.lock().await;
        for id in cancelled_queued {
            controls.remove(&id);
        }
        for id in preempt_in_flight {
            if let Some(control) = controls.get(&id) {
                control.request_prewrite_cancel();
            }
        }
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
        simulator_id_for_logical_id(inner, &enrollment.logical_id)
            .and_then(|id| simulator_by_id(inner, id).map(|(device, _)| device))
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
    let control = state
        .command_controls
        .lock()
        .await
        .entry(command_id.clone())
        .or_default()
        .clone();
    let managed_device_id = {
        let inner = state.inner.lock().await;
        inner
            .commands
            .get(&command_id)
            .map(|command| command.managed_device_id.clone())
    };
    let Some(managed_device_id) = managed_device_id else {
        state.command_controls.lock().await.remove(&command_id);
        return;
    };
    let lock = {
        let mut locks = state.device_locks.lock().await;
        locks
            .entry(managed_device_id.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    };
    let Ok(_device_permit) = lock.acquire_owned().await else {
        state.command_controls.lock().await.remove(&command_id);
        return;
    };
    let Ok(_adapter_permit) = state.adapter_slots.clone().acquire_owned().await else {
        state.command_controls.lock().await.remove(&command_id);
        return;
    };
    let (action, simulator_id, scenario, connection_delay_ms, write_delay_ms, execution_deadline) = {
        let mut inner = state.inner.lock().await;
        let simulator_id = inner
            .enrollments
            .get(&managed_device_id)
            .filter(|enrollment| enrollment.is_simulated)
            .and_then(|enrollment| simulator_id_for_logical_id(&inner, &enrollment.logical_id));
        let config =
            simulator_id.and_then(|id| simulator_by_id(&inner, id).map(|(_, config)| *config));
        let Some(command) = inner.commands.get_mut(&command_id) else {
            drop(inner);
            state.command_controls.lock().await.remove(&command_id);
            return;
        };
        if command.status != CommandStatus::Queued {
            drop(inner);
            state.command_controls.lock().await.remove(&command_id);
            return;
        }
        if Utc::now() >= command.expires_at {
            command.status = CommandStatus::Expired;
            let completed_at = Utc::now();
            command.completed_at = Some(completed_at);
            command.retryable = true;
            update_activity_status(&mut inner, &command_id, "expired", completed_at);
            let _ = state.persist(&inner).await;
            drop(inner);
            state.command_controls.lock().await.remove(&command_id);
            return;
        }
        command.status = CommandStatus::Connecting;
        let started_at = Utc::now();
        command.started_at = Some(started_at);
        command.attempt = 1;
        command.attempts.push(CommandAttempt {
            number: 1,
            phase: "connecting".into(),
            started_at,
            completed_at: None,
            duration_ms: None,
            write_state: WriteState::NotSent,
            failure_stage: None,
            error_code: None,
        });
        let execution_deadline = tokio::time::Instant::now() + COMMAND_EXECUTION_TIMEOUT;
        let action = command.action;
        let scenario = config
            .as_ref()
            .map_or(SimulatorScenario::Normal, |config| config.scenario);
        let connection_delay_ms = config
            .as_ref()
            .map_or(0, |config| config.connection_delay_ms);
        let write_delay_ms = config
            .as_ref()
            .map_or(0, |config| config.write_response_delay_ms);
        set_activity_status(&mut inner, &command_id, "connecting", None);
        let _ = state.persist(&inner).await;
        (
            action,
            simulator_id,
            scenario,
            connection_delay_ms,
            write_delay_ms,
            execution_deadline,
        )
    };
    let Some(simulator_id) = simulator_id else {
        process_physical_command(&state, &command_id, action, control, execution_deadline).await;
        return;
    };
    let simulation_started = Instant::now();
    let delay = tokio::time::sleep(StdDuration::from_millis(
        connection_delay_ms.saturating_add(write_delay_ms),
    ));
    tokio::pin!(delay);
    let delay_result = tokio::select! {
        biased;
        _ = control.cancelled() => Err(BleFailure::command_preempted()),
        result = tokio::time::timeout_at(execution_deadline, &mut delay) => match result {
            Ok(()) => Ok(()),
            Err(_) => {
                control.request_deadline_cancel();
                Err(control.cancellation_failure())
            }
        }
    };
    if let Err(failure) = delay_result {
        let error_code = ble_failure_code(&failure);
        finish_command_attempt(
            &state,
            &command_id,
            1,
            simulation_started.elapsed(),
            Err(&failure),
        )
        .await;
        complete_physical_failure(&state, &command_id, failure, error_code).await;
        return;
    }
    if scenario != SimulatorScenario::PreWriteFailure && !control.begin_write() {
        let failure = BleFailure::command_preempted();
        finish_command_attempt(
            &state,
            &command_id,
            1,
            simulation_started.elapsed(),
            Err(&failure),
        )
        .await;
        complete_physical_failure(&state, &command_id, failure, "preempted_by_stop").await;
        return;
    }

    let mut inner = state.inner.lock().await;
    let now = Utc::now();
    let simulator_enabled = inner.simulator_enabled;
    let Some((device, config)) = simulator_by_id_mut(&mut inner, simulator_id) else {
        return;
    };
    if matches!(
        scenario,
        SimulatorScenario::PreWriteFailure
            | SimulatorScenario::OutcomeUnknown
            | SimulatorScenario::GattWriteError
    ) {
        config.scenario = SimulatorScenario::Normal;
    }
    if scenario == SimulatorScenario::TelemetryPaused {
        config.telemetry_paused = true;
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
    ) && simulator_enabled
    {
        device.position.as_mut().map(|position| {
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
        device.movement_status = movement_status(current, target);
        if update_simulator_telemetry {
            device.last_seen_at = now;
        }
    }
    if let Some(command) = inner.commands.get_mut(&command_id) {
        command.duration_ms = Some(
            now.signed_duration_since(command.started_at.unwrap_or(command.accepted_at))
                .num_milliseconds()
                .max(0) as u64,
        );
        command.completed_at = Some(now);
        if let Some(attempt) = command.attempts.last_mut() {
            attempt.completed_at = Some(now);
            attempt.duration_ms = Some(
                now.signed_duration_since(attempt.started_at)
                    .num_milliseconds()
                    .max(0) as u64,
            );
        }
        match scenario {
            SimulatorScenario::PreWriteFailure => {
                command.status = CommandStatus::ConnectionFailed;
                command.error_code = Some("connection_failed".into());
                command.failure_stage = Some("device_connection".into());
                command.write_state = WriteState::NotSent;
                command.retryable = true;
                if let Some(attempt) = command.attempts.last_mut() {
                    attempt.phase = "device_connection".into();
                    attempt.failure_stage = Some("device_connection".into());
                    attempt.error_code = Some("connection_failed".into());
                    attempt.write_state = WriteState::NotSent;
                }
            }
            SimulatorScenario::OutcomeUnknown => {
                command.status = CommandStatus::OutcomeUnknown;
                command.error_code = Some("outcome_unknown".into());
                command.failure_stage = Some("gatt_write".into());
                command.write_state = WriteState::PossiblySent;
                command.retryable = false;
                if let Some(attempt) = command.attempts.last_mut() {
                    attempt.phase = "gatt_write".into();
                    attempt.failure_stage = Some("gatt_write".into());
                    attempt.error_code = Some("outcome_unknown".into());
                    attempt.write_state = WriteState::PossiblySent;
                }
            }
            SimulatorScenario::GattWriteError => {
                command.status = CommandStatus::WriteFailed;
                command.error_code = Some("gatt_write_error".into());
                command.failure_stage = Some("gatt_write".into());
                command.write_state = WriteState::PossiblySent;
                command.retryable = false;
                if let Some(attempt) = command.attempts.last_mut() {
                    attempt.phase = "gatt_write".into();
                    attempt.failure_stage = Some("gatt_write".into());
                    attempt.error_code = Some("gatt_write_error".into());
                    attempt.write_state = WriteState::PossiblySent;
                }
            }
            SimulatorScenario::Normal | SimulatorScenario::TelemetryPaused => {
                command.status = CommandStatus::WriteAcknowledged;
                command.write_state = WriteState::Acknowledged;
                command.retryable = false;
                if let Some(attempt) = command.attempts.last_mut() {
                    attempt.phase = "gatt_write_acknowledged".into();
                    attempt.write_state = WriteState::Acknowledged;
                }
            }
        }
    }
    if let Some((status, completed_at)) = inner
        .commands
        .get(&command_id)
        .and_then(|command| command.completed_at.map(|at| (command.status, at)))
    {
        update_activity_status(
            &mut inner,
            &command_id,
            command_status_text(status),
            completed_at,
        );
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
        let pending_history = state.pending_history(&inner.history);
        if state
            .persist_history(&pending_history, history_revision)
            .await
            .is_err()
        {
            tracing::warn!("Could not persist simulator telemetry history.");
        }
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Command result could not be persisted.");
    }
    drop(inner);
    state.command_controls.lock().await.remove(&command_id);
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

fn retryable_prewrite_failure(failure: &BleFailure) -> bool {
    failure.retryable_before_write
        && !failure.write_may_have_been_sent
        && !failure.outcome_unknown
        && matches!(
            failure.stage,
            BleFailureStage::DeviceConnection | BleFailureStage::ServiceResolution
        )
}

fn retry_backoff(attempt: u8) -> std::time::Duration {
    let exponent = attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(250_u64.saturating_mul(1_u64 << exponent))
}

async fn process_physical_command(
    state: &AppState,
    command_id: &str,
    action: Action,
    control: BleCommandControl,
    deadline: tokio::time::Instant,
) {
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
    let service_action = match action {
        Action::Open => ServiceAction::Open,
        Action::Close => ServiceAction::Close,
        Action::Stop => ServiceAction::Stop,
    };
    let mut attempt = 1;
    let mut failure_code_override = None;
    let result = loop {
        if attempt > 1 {
            start_command_attempt(state, command_id, attempt).await;
        }
        let attempt_started = tokio::time::Instant::now();
        let attempt_result = if control.is_cancelled() {
            Err(control.cancellation_failure())
        } else {
            let target = {
                let inner = state.inner.lock().await;
                physical_target(&inner, &managed_device_id)
            };
            match target {
                Err(code) => {
                    failure_code_override = Some(code);
                    Err(BleFailure {
                        stage: BleFailureStage::IdentityVerification,
                        retryable_before_write: false,
                        write_may_have_been_sent: false,
                        outcome_unknown: false,
                    })
                }
                Ok((peripheral_id, expected_name)) => {
                    let Some(adapter) = state.ble_adapter.read().await.clone() else {
                        let failure = BleFailure {
                            stage: BleFailureStage::Adapter,
                            retryable_before_write: true,
                            write_may_have_been_sent: false,
                            outcome_unknown: false,
                        };
                        finish_command_attempt(
                            state,
                            command_id,
                            attempt,
                            attempt_started.elapsed(),
                            Err(&failure),
                        )
                        .await;
                        return complete_physical_failure(
                            state,
                            command_id,
                            failure,
                            "adapter_unavailable",
                        )
                        .await;
                    };
                    let write = adapter.write_action(
                        &peripheral_id,
                        &expected_name,
                        service_action,
                        &control,
                    );
                    tokio::pin!(write);
                    let completed = tokio::select! {
                        biased;
                        result = &mut write => Some(result),
                        _ = control.cancelled() => None,
                        _ = tokio::time::sleep_until(deadline) => {
                            control.request_deadline_cancel();
                            None
                        }
                    };
                    match completed {
                        Some(result) => result,
                        None => match tokio::time::timeout(COMMAND_CLEANUP_GRACE, &mut write).await
                        {
                            Ok(result) => result,
                            Err(_) => {
                                tracing::warn!(
                                    command_id,
                                    cleanup_grace_ms = COMMAND_CLEANUP_GRACE.as_millis() as u64,
                                    "BLE adapter did not finish cancellation cleanup within the grace period"
                                );
                                Err(control.cancellation_failure())
                            }
                        },
                    }
                }
            }
        };
        finish_command_attempt(
            state,
            command_id,
            attempt,
            attempt_started.elapsed(),
            attempt_result.as_ref().map(|_| ()),
        )
        .await;

        if let Err(failure) = &attempt_result
            && attempt == 1
            && retryable_prewrite_failure(failure)
        {
            let delay = retry_backoff(attempt);
            tracing::warn!(
                command_id,
                attempt,
                stage = failure.stage.as_str(),
                retry_delay_ms = delay.as_millis() as u64,
                "Retrying physical command after a confirmed pre-write BLE failure"
            );
            if tokio::time::Instant::now() + delay < deadline {
                tokio::select! {
                    biased;
                    _ = control.cancelled() => break Err(control.cancellation_failure()),
                    _ = tokio::time::sleep(delay) => {}
                }
                attempt = 2;
                continue;
            }
        }
        break attempt_result;
    };

    if let Err(failure) = &result {
        tracing::warn!(
            command_id,
            attempt,
            stage = failure.stage.as_str(),
            write_may_have_been_sent = failure.write_may_have_been_sent,
            outcome_unknown = failure.outcome_unknown,
            "Physical command failed"
        );
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
            command.status = if failure.stage == BleFailureStage::CommandPreempted {
                CommandStatus::Cancelled
            } else if failure.write_may_have_been_sent {
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
            command.error_code = Some(
                failure_code_override
                    .unwrap_or_else(|| ble_failure_code(&failure))
                    .into(),
            );
            command.failure_stage = Some(failure.stage.as_str().into());
            if failure.stage == BleFailureStage::CommandPreempted {
                command.retryable = false;
            }
        }
    }
    if let Some((status, completed_at)) = inner
        .commands
        .get(command_id)
        .and_then(|command| command.completed_at.map(|at| (command.status, at)))
    {
        update_activity_status(
            &mut inner,
            command_id,
            command_status_text(status),
            completed_at,
        );
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Physical command result could not be persisted.");
    }
    drop(inner);
    state.command_controls.lock().await.remove(command_id);
}

async fn start_command_attempt(state: &AppState, command_id: &str, number: u8) {
    let mut inner = state.inner.lock().await;
    let now = Utc::now();
    if let Some(command) = inner.commands.get_mut(command_id) {
        command.attempt = number;
        command.attempts.push(CommandAttempt {
            number,
            phase: "connecting".into(),
            started_at: now,
            completed_at: None,
            duration_ms: None,
            write_state: WriteState::NotSent,
            failure_stage: None,
            error_code: None,
        });
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, attempt = number, "Command attempt start could not be persisted.");
    }
}

async fn finish_command_attempt(
    state: &AppState,
    command_id: &str,
    number: u8,
    elapsed: StdDuration,
    result: Result<(), &BleFailure>,
) {
    let mut inner = state.inner.lock().await;
    let now = Utc::now();
    let attempt = inner.commands.get_mut(command_id).and_then(|command| {
        command
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.number == number)
    });
    if let Some(attempt) = attempt {
        attempt.completed_at = Some(now);
        attempt.duration_ms = Some(elapsed.as_millis().min(u64::MAX as u128) as u64);
        match result {
            Ok(()) => {
                attempt.phase = "gatt_write_acknowledged".into();
                attempt.write_state = WriteState::Acknowledged;
            }
            Err(failure) => {
                attempt.phase = failure.stage.as_str().into();
                attempt.failure_stage = Some(failure.stage.as_str().into());
                attempt.error_code = Some(ble_failure_code(failure).into());
                attempt.write_state = if failure.write_may_have_been_sent {
                    WriteState::PossiblySent
                } else {
                    WriteState::NotSent
                };
            }
        }
    }
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, attempt = number, "Command attempt result could not be persisted.");
    }
}

async fn complete_physical_failure(
    state: &AppState,
    command_id: &str,
    failure: BleFailure,
    error_code: &str,
) {
    let mut inner = state.inner.lock().await;
    let Some(command) = inner.commands.get_mut(command_id) else {
        return;
    };
    let now = Utc::now();
    command.status = if failure.stage == BleFailureStage::CommandPreempted {
        CommandStatus::Cancelled
    } else if failure.write_may_have_been_sent {
        if failure.outcome_unknown {
            CommandStatus::OutcomeUnknown
        } else {
            CommandStatus::WriteFailed
        }
    } else {
        CommandStatus::ConnectionFailed
    };
    command.completed_at = Some(now);
    command.duration_ms = Some(
        now.signed_duration_since(command.started_at.unwrap_or(command.accepted_at))
            .num_milliseconds()
            .max(0) as u64,
    );
    command.error_code = Some(error_code.to_owned());
    command.failure_stage = Some(failure.stage.as_str().into());
    command.write_state = if failure.write_may_have_been_sent {
        WriteState::PossiblySent
    } else {
        WriteState::NotSent
    };
    command.retryable = failure.retryable_before_write && !failure.write_may_have_been_sent;
    if failure.stage == BleFailureStage::CommandPreempted {
        command.retryable = false;
    }
    let status = command_status_text(command.status);
    update_activity_status(&mut inner, command_id, status, now);
    if state.persist(&inner).await.is_err() {
        tracing::warn!(%command_id, "Physical command failure could not be persisted.");
    }
    drop(inner);
    state.command_controls.lock().await.remove(command_id);
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
        set_activity_status(&mut inner, &batch_id, "running", None);
        let _ = state.persist(&inner).await;
        ids
    };
    for command_id in command_ids {
        process_command(state.clone(), command_id.clone()).await;
        let mut inner = state.inner.lock().await;
        let command = inner.commands.get(&command_id).cloned();
        let completed_batch = if let Some(batch) = inner.batches.get_mut(&batch_id) {
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
            (batch.pending_count == 0).then(|| {
                (
                    batch.status.clone(),
                    batch.completed_at.unwrap_or_else(Utc::now),
                )
            })
        } else {
            None
        };
        if let Some((status, completed_at)) = completed_batch {
            update_activity_status(&mut inner, &batch_id, &status, completed_at);
        }
        let _ = state.persist(&inner).await;
    }
}

fn managed_device(inner: &Inner, enrollment: Enrollment) -> ManagedDevice {
    let gatt_readiness = (!enrollment.is_simulated).then(|| gatt_readiness(inner, &enrollment.id));
    let matching: Vec<&Device> = if enrollment.is_simulated {
        inner
            .simulator_enabled
            .then(|| simulator_id_for_logical_id(inner, &enrollment.logical_id))
            .flatten()
            .and_then(|id| simulator_by_id(inner, id).map(|(device, _)| device))
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
            gatt_readiness,
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
                gatt_readiness,
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
            gatt_readiness,
            last_seen_at: None,
            position: None,
        },
    }
}

fn gatt_readiness(inner: &Inner, managed_device_id: &str) -> GattReadiness {
    let latest = inner
        .commands
        .values()
        .filter(|command| command.managed_device_id == managed_device_id)
        .max_by_key(|command| (command.accepted_at, command.id.as_str()));
    let last_write_acknowledged_at = inner
        .commands
        .values()
        .filter(|command| {
            command.managed_device_id == managed_device_id
                && command.status == CommandStatus::WriteAcknowledged
        })
        .filter_map(|command| command.completed_at)
        .max();
    match latest {
        Some(command) => GattReadiness {
            status: command_status_text(command.status).to_owned(),
            last_command_accepted_at: Some(command.accepted_at),
            last_attempt_at: command.started_at,
            last_write_acknowledged_at,
            attempt_count: command.attempt,
            failure_stage: command.failure_stage.clone(),
            error_code: command.error_code.clone(),
        },
        None => GattReadiness {
            status: "untested".into(),
            last_command_accepted_at: None,
            last_attempt_at: None,
            last_write_acknowledged_at: None,
            attempt_count: 0,
            failure_stage: None,
            error_code: None,
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

#[cfg(test)]
mod security_hardening_tests {
    use super::*;

    #[tokio::test]
    async fn persisted_single_simulator_state_keeps_primary_and_adds_second() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let state = AppState::new();
        let old_name = "Hallway Blinds0";
        let old_logical_id = logical_id_for_name(old_name, "simulator");
        {
            let mut inner = state.inner.lock().await;
            retarget_simulated_device(&mut inner.simulator, SIMULATOR_ID, old_name);
            inner.simulator.position = Some(position_value(42, 42, Utc::now(), Source::Simulator));
            inner.simulator_config.scenario = SimulatorScenario::TelemetryPaused;
            inner.simulator_config.telemetry_paused = true;
            let old_snapshot = device_snapshot(&inner.simulator);
            assert!(record_history_for_device(&mut inner, &old_snapshot));
            inner.enrollments.insert(
                "existing-simulator-enrollment".into(),
                Enrollment {
                    id: "existing-simulator-enrollment".into(),
                    label: "My simulator".into(),
                    advertised_name: old_name.into(),
                    logical_id: old_logical_id,
                    is_simulated: true,
                    created_at: Utc::now(),
                },
            );
            let mut persisted = serde_json::to_value(&*inner).unwrap();
            let object = persisted.as_object_mut().unwrap();
            object.remove("simulator_two");
            object.remove("simulator_two_config");
            std::fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();
        }

        let restored = AppState::from_data_path(&path).await.unwrap();
        let inner = restored.inner.lock().await;
        assert_eq!(inner.simulator.advertised_name, SIMULATOR_NAME);
        assert_eq!(
            inner
                .simulator
                .position
                .as_ref()
                .unwrap()
                .current_closed_percent,
            42
        );
        assert_eq!(
            inner.simulator_config.scenario,
            SimulatorScenario::TelemetryPaused
        );
        assert_eq!(
            inner.history[0].logical_id.as_deref(),
            Some(logical_id_for_name(SIMULATOR_NAME, "simulator").as_str())
        );
        let second = inner.simulator_two.as_ref().unwrap();
        assert_eq!(second.id, SECOND_SIMULATOR_ID);
        assert_eq!(second.advertised_name, SECOND_SIMULATOR_NAME);
        assert_eq!(
            second.position.as_ref().unwrap().current_closed_percent,
            100
        );
        let enrollment = inner
            .enrollments
            .get("existing-simulator-enrollment")
            .unwrap()
            .clone();
        assert_eq!(enrollment.advertised_name, SIMULATOR_NAME);
        assert_eq!(
            enrollment.logical_id,
            logical_id_for_name(SIMULATOR_NAME, "simulator")
        );
        assert_eq!(managed_device(&inner, enrollment).availability, "ready");
    }

    #[test]
    fn battery_retry_schedule_distinguishes_unavailable_from_success() {
        let now = Utc::now();
        let mut battery = simulator_battery_telemetry(BatteryState::Unavailable, now);
        battery.last_attempt_at = None;
        assert!(battery_read_due(&battery, now));
        battery.last_attempt_at = Some(now);
        assert!(!battery_read_due(
            &battery,
            now + ChronoDuration::minutes(4)
        ));
        assert!(battery_read_due(&battery, now + ChronoDuration::minutes(5)));
        battery.status = BatteryStatus::Observed;
        assert!(!battery_read_due(
            &battery,
            now + ChronoDuration::minutes(59)
        ));
        assert!(battery_read_due(&battery, now + ChronoDuration::hours(1)));
        battery.status = BatteryStatus::Unsupported;
        assert!(!battery_read_due(
            &battery,
            now + ChronoDuration::minutes(5)
        ));
    }

    fn observation(peripheral_id: &str, advertised_name: &str) -> BleObservation {
        BleObservation {
            peripheral_id: peripheral_id.to_owned(),
            observed_address: None,
            device_name: Some("S".into()),
            rssi_dbm: -40,
            manufacturer_payload: simulated_manufacturer_payload(advertised_name, 50, 50),
        }
    }

    #[test]
    fn simulator_advertisements_follow_the_device_name_and_position_contract() {
        let payload = simulated_manufacturer_payload("Study Blinds1", 23, 57);
        let decoded = parse_soma_payload(&payload).unwrap();

        assert_eq!(decoded.advertised_name, "Study Blinds1");
        assert_eq!(decoded.current_closed_percent, 23);
        assert_eq!(decoded.target_closed_percent, 57);
    }

    async fn fill_registry(state: &AppState) {
        let mut inner = state.inner.lock().await;
        let now = Utc::now();
        for index in 0..MAX_DISCOVERED_DEVICES {
            let key = format!("peripheral-{index}");
            let mut device = make_simulated_device(SIMULATOR_ID, SIMULATOR_NAME, now);
            device.id = format!("device-{index}");
            device.logical_id = Some(format!("soma-{index}"));
            device.adapter_device_id = key.clone();
            device.advertised_name = format!("Blind {index}");
            device.last_seen_at = now - ChronoDuration::milliseconds(index as i64);
            inner.devices.insert(key, device);
        }
    }

    #[tokio::test]
    async fn ble_registry_replaces_the_oldest_unambiguous_device_at_capacity() {
        let state = AppState::new();
        fill_registry(&state).await;

        state
            .store_ble_observation(observation("peripheral-overflow", "Overflow blind"))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), MAX_DISCOVERED_DEVICES);
        assert!(inner.devices.contains_key("peripheral-overflow"));
        assert!(!inner.devices.contains_key("peripheral-1023"));
    }

    #[tokio::test]
    async fn ble_registry_preserves_stale_enrolled_devices_at_capacity() {
        let state = AppState::new();
        fill_registry(&state).await;
        {
            let mut inner = state.inner.lock().await;
            let stale_enrolled = inner.devices.get_mut("peripheral-1023").unwrap();
            stale_enrolled.last_seen_at = Utc::now() - ChronoDuration::seconds(61);
            stale_enrolled.freshness = Freshness::Stale;
            inner.enrollments.insert(
                "enrollment-1023".into(),
                Enrollment {
                    id: "enrollment-1023".into(),
                    label: "Blind 1023".into(),
                    advertised_name: "Blind 1023".into(),
                    logical_id: "soma-1023".into(),
                    is_simulated: false,
                    created_at: Utc::now(),
                },
            );
        }

        state
            .store_ble_observation(observation("peripheral-overflow", "Overflow blind"))
            .await;

        let inner = state.inner.lock().await;
        assert!(inner.devices.contains_key("peripheral-1023"));
        assert!(inner.devices.contains_key("peripheral-overflow"));
        assert!(!inner.devices.contains_key("peripheral-1022"));
    }

    #[tokio::test]
    async fn ble_registry_rejects_a_new_peripheral_claiming_a_retained_enrolled_identity() {
        let state = AppState::new();
        fill_registry(&state).await;
        let logical_id = logical_id_for_name("Enrolled blind", "soma");
        {
            let mut inner = state.inner.lock().await;
            let existing = inner.devices.get_mut("peripheral-0").unwrap();
            existing.logical_id = Some(logical_id.clone());
            existing.advertised_name = "Enrolled blind".into();
            inner.enrollments.insert(
                "enrollment-0".into(),
                Enrollment {
                    id: "enrollment-0".into(),
                    label: "Enrolled blind".into(),
                    advertised_name: "Enrolled blind".into(),
                    logical_id,
                    is_simulated: false,
                    created_at: Utc::now(),
                },
            );
        }

        state
            .store_ble_observation(observation("peripheral-impersonator", "Enrolled blind"))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), MAX_DISCOVERED_DEVICES);
        assert!(inner.devices.contains_key("peripheral-0"));
        assert!(!inner.devices.contains_key("peripheral-impersonator"));
    }

    #[tokio::test]
    async fn ble_registry_rebinds_a_stale_enrolled_identity_to_a_new_peripheral_id() {
        let state = AppState::new();
        fill_registry(&state).await;
        let logical_id = logical_id_for_name("Enrolled blind", "soma");
        {
            let mut inner = state.inner.lock().await;
            let existing = inner.devices.get_mut("peripheral-0").unwrap();
            existing.logical_id = Some(logical_id.clone());
            existing.advertised_name = "Enrolled blind".into();
            existing.last_seen_at = Utc::now() - ChronoDuration::seconds(61);
            existing.freshness = Freshness::Stale;
            existing.is_simulated = false;
            existing.source = Source::BleAdvertisement;
            inner.enrollments.insert(
                "enrollment-0".into(),
                Enrollment {
                    id: "enrollment-0".into(),
                    label: "Enrolled blind".into(),
                    advertised_name: "Enrolled blind".into(),
                    logical_id,
                    is_simulated: false,
                    created_at: Utc::now(),
                },
            );
        }

        state
            .store_ble_observation(observation("peripheral-rotated", "Enrolled blind"))
            .await;

        let inner = state.inner.lock().await;
        assert!(!inner.devices.contains_key("peripheral-0"));
        assert!(inner.devices.contains_key("peripheral-rotated"));
        assert_eq!(
            physical_target(&inner, "enrollment-0").unwrap().0,
            "peripheral-rotated"
        );
        assert_eq!(
            managed_device(&inner, inner.enrollments["enrollment-0"].clone()).availability,
            "ready"
        );
    }

    #[tokio::test]
    async fn ble_registry_rebinds_stale_enrolled_device_when_every_slot_is_enrolled() {
        let state = AppState::new();
        fill_registry(&state).await;
        let target_name = "Blind 1023";
        let target_logical_id = logical_id_for_name(target_name, "soma");
        {
            let mut inner = state.inner.lock().await;
            for index in 0..MAX_DISCOVERED_DEVICES {
                let key = format!("peripheral-{index}");
                let device = &inner.devices[&key];
                let enrollment = Enrollment {
                    id: format!("enrollment-{index}"),
                    label: device.advertised_name.clone(),
                    advertised_name: device.advertised_name.clone(),
                    logical_id: device.logical_id.clone().unwrap(),
                    is_simulated: false,
                    created_at: Utc::now(),
                };
                inner.enrollments.insert(enrollment.id.clone(), enrollment);
            }
            let target = inner.devices.get_mut("peripheral-1023").unwrap();
            target.logical_id = Some(target_logical_id.clone());
            target.advertised_name = target_name.into();
            target.last_seen_at = Utc::now() - ChronoDuration::seconds(61);
            target.freshness = Freshness::Stale;
            target.is_simulated = false;
            target.source = Source::BleAdvertisement;
            let target_enrollment = inner.enrollments.get_mut("enrollment-1023").unwrap();
            target_enrollment.logical_id = target_logical_id;
            target_enrollment.advertised_name = target_name.into();
        }

        state
            .store_ble_observation(observation("peripheral-rotated", target_name))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), MAX_DISCOVERED_DEVICES);
        assert!(!inner.devices.contains_key("peripheral-1023"));
        assert!(inner.devices.contains_key("peripheral-rotated"));
        assert_eq!(
            physical_target(&inner, "enrollment-1023").unwrap().0,
            "peripheral-rotated"
        );
    }

    #[tokio::test]
    async fn ble_registry_still_refreshes_known_devices_at_capacity() {
        let state = AppState::new();
        fill_registry(&state).await;

        state
            .store_ble_observation(observation("peripheral-0", "Updated blind"))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), MAX_DISCOVERED_DEVICES);
        assert_eq!(
            inner.devices["peripheral-0"].advertised_name,
            "Updated blind"
        );
        assert_eq!(inner.history.len(), 3);
    }

    #[tokio::test]
    async fn ble_registry_keeps_existing_candidates_when_a_new_device_shares_their_name() {
        let state = AppState::new();
        fill_registry(&state).await;
        let logical_id = logical_id_for_name("Shared name", "soma");
        {
            let mut inner = state.inner.lock().await;
            let existing = inner.devices.get_mut("peripheral-0").unwrap();
            existing.logical_id = Some(logical_id.clone());
            existing.advertised_name = "Shared name".into();
        }

        state
            .store_ble_observation(observation("peripheral-shared", "Shared name"))
            .await;

        let inner = state.inner.lock().await;
        assert!(inner.devices.contains_key("peripheral-0"));
        assert!(inner.devices.contains_key("peripheral-shared"));
        assert_eq!(
            inner
                .devices
                .values()
                .filter(|device| device.logical_id.as_deref() == Some(logical_id.as_str()))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn ble_registry_drops_an_unenrolled_ambiguous_group_as_a_unit() {
        let state = AppState::new();
        fill_registry(&state).await;
        {
            let mut inner = state.inner.lock().await;
            for device in inner.devices.values_mut() {
                device.logical_id = Some("soma-ambiguous".into());
                device.advertised_name = "Ambiguous".into();
            }
        }

        state
            .store_ble_observation(observation("peripheral-overflow", "Overflow blind"))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), 1);
        assert!(inner.devices.contains_key("peripheral-overflow"));
        assert!(
            inner
                .devices
                .values()
                .all(|device| device.logical_id.as_deref() != Some("soma-ambiguous"))
        );
    }

    #[tokio::test]
    async fn ble_registry_preserves_enrolled_ambiguous_groups_at_capacity() {
        let state = AppState::new();
        fill_registry(&state).await;
        {
            let mut inner = state.inner.lock().await;
            for device in inner.devices.values_mut() {
                device.logical_id = Some("soma-enrolled-ambiguous".into());
                device.advertised_name = "Enrolled ambiguous".into();
            }
            inner.enrollments.insert(
                "enrollment-ambiguous".into(),
                Enrollment {
                    id: "enrollment-ambiguous".into(),
                    label: "Enrolled ambiguous".into(),
                    advertised_name: "Enrolled ambiguous".into(),
                    logical_id: "soma-enrolled-ambiguous".into(),
                    is_simulated: false,
                    created_at: Utc::now(),
                },
            );
        }

        state
            .store_ble_observation(observation("peripheral-overflow", "Overflow blind"))
            .await;

        let inner = state.inner.lock().await;
        assert_eq!(inner.devices.len(), MAX_DISCOVERED_DEVICES);
        assert!(!inner.devices.contains_key("peripheral-overflow"));
        assert_eq!(
            inner
                .devices
                .values()
                .filter(|device| {
                    device.logical_id.as_deref() == Some("soma-enrolled-ambiguous")
                })
                .count(),
            MAX_DISCOVERED_DEVICES
        );
    }

    #[tokio::test]
    async fn history_pruning_preserves_latest_index_for_retained_device_records() {
        let state = AppState::new();
        let mut inner = state.inner.lock().await;
        let now = Utc::now();
        for cursor in 1..=50_001 {
            let device_id = if cursor == 50_001 {
                "device-1".to_string()
            } else {
                format!("device-{cursor}")
            };
            let record = HistoryObservation {
                cursor,
                device_id,
                logical_id: Some(format!("soma-{cursor}")),
                observed_at: now - ChronoDuration::seconds(cursor as i64),
                rssi_dbm: -40,
                position_status: PositionStatus::Observed,
                position: None,
            };
            inner
                .latest_history_by_device
                .insert(record.device_id.clone(), record.clone());
            inner.history.push_back(record);
        }

        prune_history(&mut inner);

        assert_eq!(inner.history.len(), 50_000);
        assert_eq!(inner.history.front().unwrap().cursor, 2);
        assert_eq!(
            inner
                .latest_history_by_device
                .get("device-1")
                .map(|record| record.cursor),
            Some(50_001)
        );
        assert!(
            inner
                .history
                .iter()
                .any(|record| record.device_id == "device-1" && record.cursor == 50_001)
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_state_writes_use_private_permissions_and_replace_destination_symlinks() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"keep me private").unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();

        atomic_write(&path, b"state").unwrap();

        assert_eq!(fs::read(&sentinel).unwrap(), b"keep me private");
        assert_eq!(fs::read(&path).unwrap(), b"state");
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
        assert_eq!(
            fs::metadata(&path).unwrap().mode() & 0o777,
            0o600,
            "state file must only be readable and writable by its owner"
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_append_tightens_permissions_of_existing_files() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.jsonl");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        append_and_sync(&path, b"new\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"old\nnew\n");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn history_read_tightens_permissions_of_existing_files() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.jsonl");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(read_history(&path).unwrap().is_empty());

        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn private_file_open_does_not_change_directory_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(open_private_file(&path, false).is_err());

        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn durable_state_load_does_not_follow_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let sentinel = directory.path().join("sentinel");
        let state = AppState::new();
        let contents = serde_json::to_vec(&*state.inner.lock().await).unwrap();
        fs::write(&sentinel, &contents).unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();
        let mut state = state;
        state.data_path = Some(path.clone());

        assert!(state.load_durable_state().await.is_err());

        assert_eq!(fs::read(&sentinel).unwrap(), contents);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[tokio::test]
    async fn recovered_connecting_command_closes_its_attempt_as_outcome_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let state = AppState::configured(true, None, Some(path.clone()));
        let started_at = Utc::now() - ChronoDuration::seconds(5);
        let mut command = make_command("managed-physical", Action::Open, 120);
        command.status = CommandStatus::Connecting;
        command.started_at = Some(started_at);
        command.attempt = 1;
        command.attempts.push(CommandAttempt {
            number: 1,
            phase: "gatt_write".into(),
            started_at,
            completed_at: None,
            duration_ms: None,
            write_state: WriteState::NotSent,
            failure_stage: None,
            error_code: None,
        });
        let command_id = command.id.clone();
        {
            let mut inner = state.inner.lock().await;
            inner.commands.insert(command_id.clone(), command);
            state.persist(&inner).await.unwrap();
        }

        let recovered = AppState::from_data_path(&path).await.unwrap();
        let inner = recovered.inner.lock().await;
        let command = inner.commands.get(&command_id).unwrap();
        assert_eq!(command.status, CommandStatus::OutcomeUnknown);
        assert_eq!(command.write_state, WriteState::PossiblySent);
        assert_eq!(command.attempts[0].phase, "service_recovery");
        assert_eq!(command.attempts[0].write_state, WriteState::PossiblySent);
        assert_eq!(
            command.attempts[0].error_code.as_deref(),
            Some("outcome_unknown")
        );
        assert!(command.attempts[0].duration_ms.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn history_append_does_not_follow_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.jsonl");
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"keep me private\n").unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();

        assert!(append_and_sync(&path, b"attacker data\n").is_err());

        assert_eq!(fs::read(&sentinel).unwrap(), b"keep me private\n");
    }

    #[cfg(unix)]
    #[test]
    fn history_repair_does_not_follow_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.jsonl");
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"preserve this incomplete journal").unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();

        assert!(read_history(&path).is_err());

        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"preserve this incomplete journal"
        );
    }
}
