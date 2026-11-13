use std::{
    fmt::Display,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use btleplug::{
    api::{
        Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _,
        ScanFilter, WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, watch};
use uuid::Uuid;

use crate::controller::{
    ServiceAction, encode_motor_speed_readback_query, encode_service_action,
    firmware_supports_motor_speed, parse_motor_speed_readback, service_action_target,
};

use super::{BA71_CHARACTERISTIC_UUID, SOMA_MANUFACTURER_ID, parse_soma_payload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleObservation {
    /// Host adapter identifier used to look this peripheral up for GATT work.
    /// This is not assumed to be a Bluetooth MAC address.
    pub peripheral_id: String,
    /// MAC address when the host platform exposes a usable value.
    pub observed_address: Option<String>,
    pub device_name: Option<String>,
    pub rssi_dbm: i16,
    pub manufacturer_payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleFailureStage {
    Adapter,
    DeviceConnection,
    ServiceResolution,
    GattDiscovery,
    IdentityVerification,
    GattWrite,
    CommandDeadline,
    CommandPreempted,
    BatteryDiscovery,
    BatteryRead,
    FirmwareRead,
    UnsupportedFirmware,
    MotorSettingsDiscovery,
    MotorSettingsRead,
}

impl BleFailureStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adapter => "adapter",
            Self::DeviceConnection => "device_connection",
            Self::ServiceResolution => "service_resolution",
            Self::GattDiscovery => "gatt_discovery",
            Self::IdentityVerification => "identity_verification",
            Self::GattWrite => "gatt_write",
            Self::CommandDeadline => "command_deadline",
            Self::CommandPreempted => "command_preempted",
            Self::BatteryDiscovery => "battery_discovery",
            Self::BatteryRead => "battery_read",
            Self::FirmwareRead => "firmware_read",
            Self::UnsupportedFirmware => "unsupported_firmware",
            Self::MotorSettingsDiscovery => "motor_settings_discovery",
            Self::MotorSettingsRead => "motor_settings_read",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleFailureCause {
    ConnectionTimeout,
    ConnectionAbortedByLocal,
    ConnectionTransportError,
}

impl BleFailureCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionTimeout => "connection_timeout",
            Self::ConnectionAbortedByLocal => "connection_aborted_by_local",
            Self::ConnectionTransportError => "connection_transport_error",
        }
    }

    fn from_connect_error(error: &impl Display) -> Self {
        let normalized = error
            .to_string()
            .to_ascii_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if normalized.contains("leconnectionabortbylocal") {
            Self::ConnectionAbortedByLocal
        } else {
            Self::ConnectionTransportError
        }
    }
}

#[derive(Clone)]
pub struct BleScanControl {
    cancelled: watch::Sender<bool>,
}

impl BleScanControl {
    pub fn new() -> Self {
        let (cancelled, _) = watch::channel(false);
        Self { cancelled }
    }

    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    pub async fn cancelled(&self) {
        let mut receiver = self.cancelled.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for BleScanControl {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleFailure {
    pub stage: BleFailureStage,
    pub cause: Option<BleFailureCause>,
    pub retryable_before_write: bool,
    pub write_may_have_been_sent: bool,
    pub outcome_unknown: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleAttemptTimings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peripheral_lookup_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_discovery_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_verification_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gatt_discovery_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_check_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gatt_write_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disconnect_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub enum BlePhase {
    PeripheralLookup,
    Connect,
    ServiceDiscovery,
    IdentityVerification,
    GattDiscovery,
    FirmwareCheck,
    GattWrite,
    Disconnect,
}

impl BleFailure {
    fn before_write(stage: BleFailureStage, retryable: bool) -> Self {
        Self {
            stage,
            cause: None,
            retryable_before_write: retryable,
            write_may_have_been_sent: false,
            outcome_unknown: false,
        }
    }

    fn during_write(outcome_unknown: bool) -> Self {
        Self {
            stage: BleFailureStage::GattWrite,
            cause: None,
            retryable_before_write: false,
            write_may_have_been_sent: true,
            outcome_unknown,
        }
    }

    pub(crate) fn command_deadline(write_may_have_been_sent: bool) -> Self {
        Self {
            stage: if write_may_have_been_sent {
                BleFailureStage::GattWrite
            } else {
                BleFailureStage::CommandDeadline
            },
            cause: None,
            retryable_before_write: !write_may_have_been_sent,
            write_may_have_been_sent,
            outcome_unknown: write_may_have_been_sent,
        }
    }

    pub(crate) fn command_preempted() -> Self {
        Self {
            stage: BleFailureStage::CommandPreempted,
            cause: None,
            retryable_before_write: false,
            write_may_have_been_sent: false,
            outcome_unknown: false,
        }
    }
}

const CONTROL_PRE_WRITE: u8 = 0;
const CONTROL_STOP_CANCELLED: u8 = 1;
const CONTROL_WRITE_STARTED: u8 = 2;
const CONTROL_DEADLINE_BEFORE_WRITE: u8 = 3;
const CONTROL_DEADLINE_AFTER_WRITE: u8 = 4;

/// Coordinates safe Stop preemption with the transition into a possibly-sent GATT write.
/// Exactly one of `request_prewrite_cancel` and `begin_write` can win.
#[derive(Clone, Default)]
pub struct BleCommandControl {
    state: Arc<AtomicU8>,
    cancelled: Arc<Notify>,
    timings: Arc<Mutex<BleAttemptTimings>>,
}

impl BleCommandControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    pub fn reset_timings(&self) {
        *self
            .timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = BleAttemptTimings::default();
    }

    pub fn record_timings(&self, timings: BleAttemptTimings) {
        *self
            .timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = timings;
    }

    pub fn record_phase(&self, phase: BlePhase, elapsed: Duration) {
        let millis = elapsed.as_millis().min(u64::MAX as u128) as u64;
        let mut timings = self
            .timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match phase {
            BlePhase::PeripheralLookup => timings.peripheral_lookup_ms = Some(millis),
            BlePhase::Connect => timings.connect_ms = Some(millis),
            BlePhase::ServiceDiscovery => timings.service_discovery_ms = Some(millis),
            BlePhase::IdentityVerification => timings.identity_verification_ms = Some(millis),
            BlePhase::GattDiscovery => timings.gatt_discovery_ms = Some(millis),
            BlePhase::FirmwareCheck => timings.firmware_check_ms = Some(millis),
            BlePhase::GattWrite => timings.gatt_write_ms = Some(millis),
            BlePhase::Disconnect => timings.disconnect_ms = Some(millis),
        }
    }

    pub fn timings(&self) -> BleAttemptTimings {
        *self
            .timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn request_prewrite_cancel(&self) -> bool {
        let cancelled = self
            .state
            .compare_exchange(
                CONTROL_PRE_WRITE,
                CONTROL_STOP_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if cancelled {
            self.cancelled.notify_waiters();
        }
        cancelled
    }

    pub fn begin_write(&self) -> bool {
        self.state
            .compare_exchange(
                CONTROL_PRE_WRITE,
                CONTROL_WRITE_STARTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn write_started(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            CONTROL_WRITE_STARTED | CONTROL_DEADLINE_AFTER_WRITE
        )
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            CONTROL_STOP_CANCELLED | CONTROL_DEADLINE_BEFORE_WRITE | CONTROL_DEADLINE_AFTER_WRITE
        )
    }

    pub fn request_deadline_cancel(&self) {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            let next = match current {
                CONTROL_PRE_WRITE => CONTROL_DEADLINE_BEFORE_WRITE,
                CONTROL_WRITE_STARTED => CONTROL_DEADLINE_AFTER_WRITE,
                _ => return,
            };
            match self.state.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.cancelled.notify_waiters();
                    return;
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub fn cancellation_failure(&self) -> BleFailure {
        match self.state.load(Ordering::Acquire) {
            CONTROL_DEADLINE_BEFORE_WRITE => BleFailure::command_deadline(false),
            CONTROL_DEADLINE_AFTER_WRITE => BleFailure::command_deadline(true),
            CONTROL_WRITE_STARTED => BleFailure::during_write(true),
            _ => BleFailure::command_preempted(),
        }
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.cancelled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[async_trait]
pub trait BleAdapter: Send + Sync {
    async fn scan(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
    ) -> Result<(), BleFailure>;

    /// Cooperatively stops an active scan when adapter capacity is needed for a command.
    /// The default is non-preemptible: it waits for `scan` to return, normally at the end of
    /// `window`. Adapters that own active radio discovery should override this method, stop the
    /// scan, and only then return; dropping an active scan future may leave radio discovery on.
    async fn scan_with_control(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
        _control: BleScanControl,
    ) -> Result<(), BleFailure> {
        self.scan(observations, window).await
    }

    /// Executes one physical action and must observe cancellation until `begin_write` wins.
    /// After cancellation it must finish best-effort disconnect cleanup before returning.
    async fn write_action(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
        control: &BleCommandControl,
    ) -> Result<(), BleFailure>;

    /// Executes an action with the current retry number available to adapters that tune
    /// connection setup by attempt. Existing adapters remain compatible through this default.
    async fn write_action_with_attempt(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
        attempt: u8,
        control: &BleCommandControl,
    ) -> Result<(), BleFailure> {
        let _ = attempt;
        self.write_action(peripheral_id, expected_name, action, control)
            .await
    }

    async fn read_battery(
        &self,
        peripheral_id: &str,
        expected_name: &str,
    ) -> Result<Vec<u8>, BleFailure>;

    /// Reads battery telemetry while allowing an accepted Stop to release the adapter early.
    /// Existing adapters keep their implementation through this default method.
    async fn read_battery_with_control(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        control: &BleCommandControl,
    ) -> Result<Vec<u8>, BleFailure> {
        if control.is_cancelled() {
            return Err(control.cancellation_failure());
        }
        self.read_battery(peripheral_id, expected_name).await
    }

    /// Reads the current firmware and motor-speed setting without moving the blind.
    async fn read_motor_settings(
        &self,
        _peripheral_id: &str,
        _expected_name: &str,
        control: &BleCommandControl,
    ) -> Result<MotorSettingsReadback, BleFailure> {
        if control.is_cancelled() {
            return Err(control.cancellation_failure());
        }
        Err(BleFailure::before_write(
            BleFailureStage::MotorSettingsDiscovery,
            false,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotorSettingsReadback {
    pub firmware_version: String,
    pub motor_speed_percent: Option<u8>,
}

#[derive(Clone)]
pub struct BtleplugAdapter {
    adapter: Adapter,
    connection_cache: Arc<Mutex<Option<CachedConnection>>>,
}

const CONNECTION_REUSE_WINDOW: Duration = Duration::from_secs(5);

struct CachedConnection {
    peripheral: Peripheral,
    peripheral_id: String,
    expected_name: String,
    last_used: std::time::Instant,
}

fn connection_reusable(
    cached_peripheral_id: &str,
    cached_expected_name: &str,
    last_used: std::time::Instant,
    requested_peripheral_id: &str,
    requested_expected_name: &str,
    now: std::time::Instant,
) -> bool {
    cached_peripheral_id.eq_ignore_ascii_case(requested_peripheral_id)
        && cached_expected_name == requested_expected_name
        && now.saturating_duration_since(last_used) < CONNECTION_REUSE_WINDOW
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionPurpose {
    Stop,
    Command,
    ReadOnly,
}

fn connect_timeout(purpose: ConnectionPurpose, attempt: u8) -> Duration {
    match (purpose, attempt) {
        (ConnectionPurpose::Stop, 1) => Duration::from_secs(4),
        (ConnectionPurpose::Stop, _) => Duration::from_secs(6),
        (ConnectionPurpose::Command, 1) => Duration::from_secs(8),
        (ConnectionPurpose::ReadOnly, _) => Duration::from_secs(6),
        (ConnectionPurpose::Command, _) => Duration::from_secs(6),
    }
}

const DEVICE_INFORMATION_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000180a00001000800000805f9b34fb);
const FIRMWARE_REVISION_CHARACTERISTIC_UUID: Uuid =
    Uuid::from_u128(0x00002a2600001000800000805f9b34fb);

fn firmware_revision_characteristic(characteristics: &[Characteristic]) -> Option<&Characteristic> {
    characteristics.iter().find(|characteristic| {
        characteristic.service_uuid == DEVICE_INFORMATION_SERVICE_UUID
            && characteristic.uuid == FIRMWARE_REVISION_CHARACTERISTIC_UUID
            && characteristic.properties.contains(CharPropFlags::READ)
    })
}

struct DisconnectGuard(Option<Peripheral>);

impl DisconnectGuard {
    fn new(peripheral: &Peripheral) -> Self {
        Self(Some(peripheral.clone()))
    }

    fn disarm(&mut self) {
        self.0 = None;
    }

    async fn disconnect(&mut self) {
        if let Some(peripheral) = self.0.take() {
            BtleplugAdapter::disconnect(&peripheral).await;
        }
    }

    async fn abort_connection(&mut self) {
        if let Some(peripheral) = self.0.take() {
            BtleplugAdapter::disconnect(&peripheral).await;
        }
    }
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        let Some(peripheral) = self.0.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                BtleplugAdapter::disconnect(&peripheral).await;
            });
        } else {
            tracing::warn!(
                operation = "disconnect",
                "BLE disconnect cleanup could not be scheduled without a Tokio runtime"
            );
        }
    }
}

impl BtleplugAdapter {
    pub async fn new() -> Result<Self, BleFailure> {
        let manager = match tokio::time::timeout(Duration::from_secs(10), Manager::new()).await {
            Ok(Ok(manager)) => manager,
            Ok(Err(error)) => {
                tracing::warn!(operation = "create_manager", %error, "BLE adapter initialization failed");
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "create_manager",
                    timeout_seconds = 10,
                    "BLE adapter initialization timed out"
                );
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
        };
        let adapters = match tokio::time::timeout(Duration::from_secs(10), manager.adapters()).await
        {
            Ok(Ok(adapters)) => adapters,
            Ok(Err(error)) => {
                tracing::warn!(operation = "list_adapters", %error, "BLE adapter discovery failed");
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "list_adapters",
                    timeout_seconds = 10,
                    "BLE adapter discovery timed out"
                );
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
        };
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| BleFailure::before_write(BleFailureStage::Adapter, true))?;
        Ok(Self {
            adapter,
            connection_cache: Arc::new(Mutex::new(None)),
        })
    }

    fn take_cached_connection(&self) -> Option<CachedConnection> {
        self.connection_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    async fn cache_connection(
        &self,
        peripheral: &Peripheral,
        peripheral_id: &str,
        expected_name: &str,
    ) {
        let cached = CachedConnection {
            peripheral: peripheral.clone(),
            peripheral_id: peripheral_id.to_owned(),
            expected_name: expected_name.to_owned(),
            last_used: std::time::Instant::now(),
        };
        let last_used = cached.last_used;
        let previous = self
            .connection_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(cached);
        if let Some(previous) = previous {
            Self::disconnect(&previous.peripheral).await;
        }

        let connection_cache = self.connection_cache.clone();
        tokio::spawn(async move {
            tokio::time::sleep(CONNECTION_REUSE_WINDOW).await;
            let expired = {
                let mut cache = connection_cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if cache
                    .as_ref()
                    .is_some_and(|cached| cached.last_used == last_used)
                {
                    cache.take()
                } else {
                    None
                }
            };
            if let Some(expired) = expired {
                Self::disconnect(&expired.peripheral).await;
            }
        });
    }

    async fn reuse_cached_connection(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        control: &BleCommandControl,
    ) -> Result<Option<Peripheral>, BleFailure> {
        let Some(cached) = self.take_cached_connection() else {
            return Ok(None);
        };
        let mut disconnect_guard = DisconnectGuard::new(&cached.peripheral);
        if !connection_reusable(
            &cached.peripheral_id,
            &cached.expected_name,
            cached.last_used,
            peripheral_id,
            expected_name,
            std::time::Instant::now(),
        ) {
            disconnect_guard.disconnect().await;
            return Ok(None);
        }

        control.record_phase(BlePhase::PeripheralLookup, Duration::ZERO);
        let connection = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(3), cached.peripheral.is_connected()) => result,
        };
        match connection {
            Ok(Ok(true)) => {
                control.record_phase(BlePhase::Connect, Duration::ZERO);
                control.record_phase(BlePhase::ServiceDiscovery, Duration::ZERO);
            }
            Ok(Ok(false)) | Ok(Err(_)) | Err(_) => {
                disconnect_guard.disconnect().await;
                return Ok(None);
            }
        }

        let identity_started = std::time::Instant::now();
        let identity = tokio::select! {
            biased;
            _ = control.cancelled() => {
                control.record_phase(
                    BlePhase::IdentityVerification,
                    identity_started.elapsed(),
                );
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = Self::verify_identity(&cached.peripheral, expected_name) => result,
        };
        control.record_phase(BlePhase::IdentityVerification, identity_started.elapsed());
        if let Err(error) = identity {
            disconnect_guard.disconnect().await;
            return Err(error);
        }
        disconnect_guard.disarm();
        Ok(Some(cached.peripheral))
    }

    async fn peripheral_at(&self, peripheral_id: &str) -> Result<Peripheral, BleFailure> {
        let peripherals = match tokio::time::timeout(
            Duration::from_secs(10),
            self.adapter.peripherals(),
        )
        .await
        {
            Ok(Ok(peripherals)) => peripherals,
            Ok(Err(error)) => {
                tracing::warn!(operation = "list_peripherals", %error, "BLE device lookup failed");
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "list_peripherals",
                    timeout_seconds = 10,
                    "BLE device lookup timed out"
                );
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
            }
        };
        peripherals
            .into_iter()
            .find(|peripheral| {
                peripheral
                    .id()
                    .to_string()
                    .eq_ignore_ascii_case(peripheral_id)
            })
            .ok_or_else(|| BleFailure::before_write(BleFailureStage::DeviceConnection, true))
    }

    async fn verify_identity(
        peripheral: &Peripheral,
        expected_name: &str,
    ) -> Result<(), BleFailure> {
        let properties = match tokio::time::timeout(Duration::from_secs(5), peripheral.properties())
            .await
        {
            Ok(Ok(Some(properties))) => properties,
            Ok(Ok(None)) => {
                return Err(BleFailure::before_write(
                    BleFailureStage::IdentityVerification,
                    false,
                ));
            }
            Ok(Err(error)) => {
                tracing::warn!(operation = "verify_identity", %error, "BLE identity verification failed");
                return Err(BleFailure::before_write(
                    BleFailureStage::IdentityVerification,
                    false,
                ));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "verify_identity",
                    timeout_seconds = 5,
                    "BLE identity verification timed out"
                );
                return Err(BleFailure::before_write(
                    BleFailureStage::IdentityVerification,
                    false,
                ));
            }
        };
        let verified = properties
            .manufacturer_data
            .get(&SOMA_MANUFACTURER_ID)
            .and_then(|payload| parse_soma_payload(payload).ok())
            .is_some_and(|advertisement| advertisement.advertised_name == expected_name);
        if verified {
            Ok(())
        } else {
            Err(BleFailure::before_write(
                BleFailureStage::IdentityVerification,
                false,
            ))
        }
    }

    async fn connect_and_discover_cancellable(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        purpose: ConnectionPurpose,
        attempt: u8,
        control: &BleCommandControl,
    ) -> Result<Peripheral, BleFailure> {
        if let Some(peripheral) = self
            .reuse_cached_connection(peripheral_id, expected_name, control)
            .await?
        {
            return Ok(peripheral);
        }

        let lookup_started = std::time::Instant::now();
        let lookup_result = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(control.cancellation_failure()),
            result = self.peripheral_at(peripheral_id) => result,
        };
        control.record_phase(BlePhase::PeripheralLookup, lookup_started.elapsed());
        let peripheral = lookup_result?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let connect_timeout = connect_timeout(purpose, attempt);
        let connect_started = std::time::Instant::now();
        let connection = tokio::select! {
            biased;
            _ = control.cancelled() => {
                control.record_phase(BlePhase::Connect, connect_started.elapsed());
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.abort_connection().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(connect_timeout, peripheral.connect()) => result,
        };
        control.record_phase(BlePhase::Connect, connect_started.elapsed());
        match connection {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "connect", %error, "BLE operation failed before write");
                let cause = BleFailureCause::from_connect_error(&error);
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.abort_connection().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(BleFailure {
                    cause: Some(cause),
                    ..BleFailure::before_write(BleFailureStage::DeviceConnection, true)
                });
            }
            Err(_) => {
                tracing::warn!(
                    operation = "connect",
                    timeout_seconds = connect_timeout.as_secs(),
                    "BLE operation timed out before write"
                );
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.abort_connection().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(BleFailure {
                    cause: Some(BleFailureCause::ConnectionTimeout),
                    ..BleFailure::before_write(BleFailureStage::DeviceConnection, true)
                });
            }
        }
        let service_discovery_started = std::time::Instant::now();
        let discovery = tokio::select! {
            biased;
            _ = control.cancelled() => {
                control.record_phase(
                    BlePhase::ServiceDiscovery,
                    service_discovery_started.elapsed(),
                );
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(15), peripheral.discover_services()) => result,
        };
        control.record_phase(
            BlePhase::ServiceDiscovery,
            service_discovery_started.elapsed(),
        );
        match discovery {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "discover_services", %error, "BLE service discovery failed before write");
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(BleFailure::before_write(
                    BleFailureStage::ServiceResolution,
                    true,
                ));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "discover_services",
                    timeout_seconds = 15,
                    "BLE service discovery timed out before write"
                );
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(BleFailure::before_write(
                    BleFailureStage::ServiceResolution,
                    true,
                ));
            }
        }
        let identity_started = std::time::Instant::now();
        let identity = tokio::select! {
            biased;
            _ = control.cancelled() => {
                control.record_phase(
                    BlePhase::IdentityVerification,
                    identity_started.elapsed(),
                );
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(control.cancellation_failure());
            }
            result = Self::verify_identity(&peripheral, expected_name) => result,
        };
        control.record_phase(BlePhase::IdentityVerification, identity_started.elapsed());
        if let Err(error) = identity {
            let disconnect_started = std::time::Instant::now();
            disconnect_guard.disconnect().await;
            control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
            return Err(error);
        }
        disconnect_guard.disarm();
        Ok(peripheral)
    }

    async fn disconnect(peripheral: &Peripheral) {
        disconnect_with_timeout(peripheral.disconnect()).await;
    }

    async fn stop_scan(&self) -> Result<(), BleFailure> {
        match tokio::time::timeout(Duration::from_secs(5), self.adapter.stop_scan()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::warn!(operation = "stop_scan", %error, "BLE scan did not stop cleanly");
                Err(BleFailure::before_write(BleFailureStage::Adapter, true))
            }
            Err(_) => {
                tracing::warn!(
                    operation = "stop_scan",
                    timeout_seconds = 5,
                    "BLE scan stop timed out"
                );
                Err(BleFailure::before_write(BleFailureStage::Adapter, true))
            }
        }
    }

    async fn scan_inner(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
        control: Option<BleScanControl>,
    ) -> Result<(), BleFailure> {
        let mut events = match tokio::time::timeout(Duration::from_secs(10), self.adapter.events())
            .await
        {
            Ok(Ok(events)) => events,
            Ok(Err(error)) => {
                tracing::warn!(operation = "events", %error, "BLE adapter event stream unavailable");
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "events",
                    timeout_seconds = 10,
                    "BLE adapter event stream request timed out"
                );
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
        };
        match tokio::time::timeout(
            Duration::from_secs(10),
            self.adapter.start_scan(ScanFilter::default()),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "start_scan", %error, "BLE scan could not start");
                let _ = self.stop_scan().await;
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "start_scan",
                    timeout_seconds = 10,
                    "BLE scan start timed out"
                );
                let _ = self.stop_scan().await;
                return Err(BleFailure::before_write(BleFailureStage::Adapter, true));
            }
        }

        let scan_events = async {
            while let Some(event) = events.next().await {
                let peripheral_id = match event {
                    CentralEvent::DeviceDiscovered(id)
                    | CentralEvent::DeviceUpdated(id)
                    | CentralEvent::ManufacturerDataAdvertisement { id, .. } => id,
                    _ => continue,
                };
                let peripheral = match self.adapter.peripheral(&peripheral_id).await {
                    Ok(peripheral) => peripheral,
                    Err(error) => {
                        tracing::debug!(operation = "lookup_scanned_device", %error, "Skipping unreadable BLE advertisement");
                        continue;
                    }
                };
                let properties = match peripheral.properties().await {
                    Ok(Some(properties)) => properties,
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::debug!(operation = "read_advertisement", %error, "Skipping unreadable BLE advertisement");
                        continue;
                    }
                };
                let Some(payload) = properties.manufacturer_data.get(&SOMA_MANUFACTURER_ID) else {
                    continue;
                };
                let address = peripheral.address().to_string();
                let observation = BleObservation {
                    peripheral_id: peripheral.id().to_string(),
                    observed_address: (!address.eq_ignore_ascii_case("00:00:00:00:00:00"))
                        .then_some(address),
                    device_name: properties.local_name,
                    rssi_dbm: properties.rssi.unwrap_or(-127),
                    manufacturer_payload: payload.clone(),
                };
                if observations.send(observation).await.is_err() {
                    return Ok(());
                }
            }
            Err(BleFailure::before_write(BleFailureStage::Adapter, true))
        };
        let (scan_result, cancelled) = if let Some(control) = control {
            tokio::select! {
                biased;
                _ = control.cancelled() => (Ok(Ok(())), true),
                result = tokio::time::timeout(window, scan_events) => (result, false),
            }
        } else {
            (tokio::time::timeout(window, scan_events).await, false)
        };
        self.stop_scan().await?;
        if cancelled {
            return Ok(());
        }
        match scan_result {
            Err(_) => Ok(()),
            Ok(result) => result,
        }
    }
}

async fn disconnect_with_timeout<Fut, Error>(disconnect: Fut)
where
    Fut: Future<Output = Result<(), Error>>,
    Error: Display,
{
    match tokio::time::timeout(Duration::from_secs(5), disconnect).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(operation = "disconnect", %error, "BLE disconnect failed")
        }
        Err(_) => tracing::warn!(
            operation = "disconnect",
            timeout_seconds = 5,
            "BLE disconnect timed out"
        ),
    }
}

#[async_trait]
impl BleAdapter for BtleplugAdapter {
    async fn scan(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
    ) -> Result<(), BleFailure> {
        self.scan_inner(observations, window, None).await
    }

    async fn scan_with_control(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
        control: BleScanControl,
    ) -> Result<(), BleFailure> {
        self.scan_inner(observations, window, Some(control)).await
    }

    async fn write_action(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
        control: &BleCommandControl,
    ) -> Result<(), BleFailure> {
        self.write_action_with_attempt(peripheral_id, expected_name, action, 1, control)
            .await
    }

    async fn write_action_with_attempt(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
        attempt: u8,
        control: &BleCommandControl,
    ) -> Result<(), BleFailure> {
        let peripheral = self
            .connect_and_discover_cancellable(
                peripheral_id,
                expected_name,
                if action == ServiceAction::Stop {
                    ConnectionPurpose::Stop
                } else {
                    ConnectionPurpose::Command
                },
                attempt,
                control,
            )
            .await?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let (service_uuid, characteristic_uuid) = service_action_target(action);
        let service_uuid = Uuid::parse_str(service_uuid)
            .map_err(|_| BleFailure::before_write(BleFailureStage::GattDiscovery, false))?;
        let characteristic_uuid = Uuid::parse_str(characteristic_uuid)
            .map_err(|_| BleFailure::before_write(BleFailureStage::GattDiscovery, false))?;
        let discovery_started = std::time::Instant::now();
        let characteristics = peripheral.characteristics().into_iter().collect::<Vec<_>>();
        let characteristic = characteristics.iter().find(|characteristic| {
            characteristic.service_uuid == service_uuid
                && characteristic.uuid == characteristic_uuid
                && characteristic.properties.contains(CharPropFlags::WRITE)
        });
        let Some(characteristic) = characteristic else {
            tracing::warn!(
                expected_service = %service_uuid,
                expected_characteristic = %characteristic_uuid,
                services = ?peripheral.services(),
                ?characteristics,
                "No writable SOMA command characteristic was found after service discovery"
            );
            control.record_phase(BlePhase::GattDiscovery, discovery_started.elapsed());
            let disconnect_started = std::time::Instant::now();
            disconnect_guard.disconnect().await;
            control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
            return Err(BleFailure::before_write(
                BleFailureStage::GattDiscovery,
                false,
            ));
        };
        control.record_phase(BlePhase::GattDiscovery, discovery_started.elapsed());
        if matches!(action, ServiceAction::SetMotorSpeedPercent(_)) {
            let firmware_started = std::time::Instant::now();
            let firmware = read_firmware_revision(&peripheral, &characteristics, control).await;
            control.record_phase(BlePhase::FirmwareCheck, firmware_started.elapsed());
            let firmware = match firmware {
                Ok(version) => version,
                Err(failure) => {
                    let disconnect_started = std::time::Instant::now();
                    disconnect_guard.disconnect().await;
                    control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                    return Err(failure);
                }
            };
            if !firmware_supports_motor_speed(&firmware) {
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                tracing::info!(firmware_version = %firmware, "Motor-speed write skipped because device firmware is unsupported");
                return Err(BleFailure::before_write(
                    BleFailureStage::UnsupportedFirmware,
                    false,
                ));
            }
        }
        if !control.begin_write() {
            let disconnect_started = std::time::Instant::now();
            disconnect_guard.disconnect().await;
            control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
            return Err(control.cancellation_failure());
        }
        let payload = encode_service_action(action);
        let write_started = std::time::Instant::now();
        let result = tokio::select! {
            biased;
            _ = control.cancelled() => {
                control.record_phase(BlePhase::GattWrite, write_started.elapsed());
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(
                Duration::from_secs(10),
                peripheral.write(
                    characteristic,
                    &payload,
                    WriteType::WithResponse,
                ),
            ) => result,
        };
        control.record_phase(BlePhase::GattWrite, write_started.elapsed());
        match result {
            Ok(Ok(())) => {
                disconnect_guard.disarm();
                self.cache_connection(&peripheral, peripheral_id, expected_name)
                    .await;
                Ok(())
            }
            Ok(Err(error)) => {
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                tracing::warn!(operation = "gatt_write", %error, "BLE write failed after write may have been sent");
                Err(BleFailure::during_write(false))
            }
            Err(_) => {
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.disconnect().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                tracing::warn!(
                    operation = "gatt_write",
                    timeout_seconds = 10,
                    "BLE write timed out after write may have been sent"
                );
                Err(BleFailure::during_write(true))
            }
        }
    }

    async fn read_battery(
        &self,
        peripheral_id: &str,
        expected_name: &str,
    ) -> Result<Vec<u8>, BleFailure> {
        let control = BleCommandControl::new();
        self.read_battery_with_control(peripheral_id, expected_name, &control)
            .await
    }

    async fn read_battery_with_control(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        control: &BleCommandControl,
    ) -> Result<Vec<u8>, BleFailure> {
        let peripheral = self
            .connect_and_discover_cancellable(
                peripheral_id,
                expected_name,
                ConnectionPurpose::ReadOnly,
                1,
                control,
            )
            .await?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(is_battery_characteristic);
        let Some(characteristic) = characteristic else {
            disconnect_guard.disconnect().await;
            return Err(BleFailure::before_write(
                BleFailureStage::BatteryDiscovery,
                false,
            ));
        };
        let result = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(10), peripheral.read(&characteristic)) => result,
        };
        disconnect_guard.disconnect().await;
        match result {
            Err(_) => {
                tracing::warn!(
                    operation = "battery_read",
                    timeout_seconds = 10,
                    "BLE battery read timed out"
                );
                Err(BleFailure::before_write(BleFailureStage::BatteryRead, true))
            }
            Ok(Err(error)) => {
                tracing::warn!(operation = "battery_read", %error, "BLE battery read failed");
                Err(BleFailure::before_write(BleFailureStage::BatteryRead, true))
            }
            Ok(Ok(value)) => Ok(value),
        }
    }

    async fn read_motor_settings(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        control: &BleCommandControl,
    ) -> Result<MotorSettingsReadback, BleFailure> {
        let peripheral = self
            .connect_and_discover_cancellable(
                peripheral_id,
                expected_name,
                ConnectionPurpose::ReadOnly,
                1,
                control,
            )
            .await?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let characteristics = peripheral.characteristics().into_iter().collect::<Vec<_>>();
        let firmware_version =
            match read_firmware_revision(&peripheral, &characteristics, control).await {
                Ok(version) => version,
                Err(failure) => {
                    disconnect_guard.disconnect().await;
                    return Err(failure);
                }
            };
        if !firmware_supports_motor_speed(&firmware_version) {
            disconnect_guard.disconnect().await;
            return Ok(MotorSettingsReadback {
                firmware_version,
                motor_speed_percent: None,
            });
        }

        let service_uuid = Uuid::parse_str(crate::controller::SHADE_CONFIG_SERVICE_UUID)
            .expect("Shade Config service UUID is a fixed valid UUID");
        let characteristic_uuid =
            Uuid::parse_str(crate::controller::SHADE_CONFIG_CHARACTERISTIC_UUID)
                .expect("Shade Config characteristic UUID is a fixed valid UUID");
        let characteristic = characteristics.iter().find(|characteristic| {
            characteristic.service_uuid == service_uuid
                && characteristic.uuid == characteristic_uuid
                && characteristic.properties.contains(CharPropFlags::WRITE)
                && characteristic
                    .properties
                    .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
        });
        let Some(characteristic) = characteristic else {
            disconnect_guard.disconnect().await;
            return Err(BleFailure::before_write(
                BleFailureStage::MotorSettingsDiscovery,
                false,
            ));
        };
        let notifications_result = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(5), peripheral.notifications()) => result,
        };
        let mut notifications = match notifications_result {
            Ok(Ok(notifications)) => notifications,
            Ok(Err(error)) => {
                tracing::warn!(operation = "motor_settings_notifications", %error, "BLE motor settings notification stream failed");
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
            Err(_) => {
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
        };
        let subscribe = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(5), peripheral.subscribe(characteristic)) => result,
        };
        match subscribe {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "motor_settings_subscribe", %error, "BLE motor settings notification subscription failed");
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
            Err(_) => {
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
        }
        let request = encode_motor_speed_readback_query();
        let write = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(
                Duration::from_secs(5),
                peripheral.write(characteristic, &request, WriteType::WithResponse),
            ) => result,
        };
        match write {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "motor_settings_query", %error, "BLE motor settings query write failed");
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
            Err(_) => {
                disconnect_guard.disconnect().await;
                return Err(BleFailure::before_write(
                    BleFailureStage::MotorSettingsRead,
                    true,
                ));
            }
        }
        let readback = tokio::select! {
            biased;
            _ = control.cancelled() => {
                disconnect_guard.disconnect().await;
                return Err(control.cancellation_failure());
            }
            result = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(notification) = notifications.next().await {
                    if notification.service_uuid == service_uuid
                        && notification.uuid == characteristic_uuid
                        && let Some(value) = parse_motor_speed_readback(&notification.value)
                    {
                        return Some(value);
                    }
                }
                None
            }) => result,
        };
        disconnect_guard.disconnect().await;
        match readback {
            Ok(Some(motor_speed_percent)) => Ok(MotorSettingsReadback {
                firmware_version,
                motor_speed_percent: Some(motor_speed_percent),
            }),
            Ok(None) | Err(_) => Err(BleFailure::before_write(
                BleFailureStage::MotorSettingsRead,
                true,
            )),
        }
    }
}

async fn read_firmware_revision(
    peripheral: &Peripheral,
    characteristics: &[Characteristic],
    control: &BleCommandControl,
) -> Result<String, BleFailure> {
    let Some(characteristic) = firmware_revision_characteristic(characteristics) else {
        return Err(BleFailure::before_write(
            BleFailureStage::FirmwareRead,
            false,
        ));
    };
    let result = tokio::select! {
        biased;
        _ = control.cancelled() => return Err(control.cancellation_failure()),
        result = tokio::time::timeout(Duration::from_secs(5), peripheral.read(characteristic)) => result,
    };
    match result {
        Ok(Ok(value)) => {
            let version = String::from_utf8_lossy(&value)
                .trim_matches('\0')
                .trim()
                .to_owned();
            if version.is_empty() {
                Err(BleFailure::before_write(
                    BleFailureStage::FirmwareRead,
                    false,
                ))
            } else {
                Ok(version)
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(operation = "firmware_read", %error, "BLE firmware revision read failed");
            Err(BleFailure::before_write(
                BleFailureStage::FirmwareRead,
                false,
            ))
        }
        Err(_) => Err(BleFailure::before_write(
            BleFailureStage::FirmwareRead,
            false,
        )),
    }
}

fn is_battery_characteristic(characteristic: &Characteristic) -> bool {
    characteristic.uuid == BA71_CHARACTERISTIC_UUID
        && characteristic.properties.contains(CharPropFlags::READ)
}

#[cfg(test)]
mod battery_tests {
    use super::*;
    use btleplug::api::Characteristic;

    #[tokio::test]
    async fn scan_cancellation_is_observed_even_if_requested_before_waiting() {
        let control = BleScanControl::new();
        control.cancel();

        tokio::time::timeout(Duration::from_millis(20), control.cancelled())
            .await
            .expect("watch-based cancellation must not lose an early signal");
        assert!(control.is_cancelled());
    }

    #[test]
    fn connect_failures_are_reduced_to_stable_public_causes() {
        assert_eq!(
            BleFailureCause::from_connect_error(&"LE-Connection-Abort-By-Local (0x16)"),
            BleFailureCause::ConnectionAbortedByLocal
        );
        assert_eq!(
            BleFailureCause::from_connect_error(&"org.bluez.Error.Failed"),
            BleFailureCause::ConnectionTransportError
        );
        assert_eq!(
            BleFailureCause::ConnectionTimeout.as_str(),
            "connection_timeout"
        );
    }
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicBool;
    use std::task::Poll;

    #[tokio::test]
    async fn disconnect_cleanup_finishes_when_disconnect_succeeds() {
        // Contract: BLE-CLEANUP-01 in docs/test-contracts.md.
        let called = Arc::new(AtomicBool::new(false));
        let called_by_disconnect = called.clone();

        disconnect_with_timeout(async move {
            called_by_disconnect.store(true, Ordering::SeqCst);
            Ok::<(), std::io::Error>(())
        })
        .await;

        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_cleanup_bounds_a_disconnect_that_never_completes() {
        // Contract: BLE-CLEANUP-01 in docs/test-contracts.md.
        let polled = Arc::new(AtomicBool::new(false));
        let polled_by_disconnect = polled.clone();
        let disconnect = std::future::poll_fn(move |_| {
            polled_by_disconnect.store(true, Ordering::SeqCst);
            Poll::<Result<(), std::io::Error>>::Pending
        });
        let started_at = tokio::time::Instant::now();

        let completed =
            tokio::time::timeout(Duration::from_secs(10), disconnect_with_timeout(disconnect))
                .await;

        assert!(
            completed.is_ok(),
            "cleanup should stop waiting within its bound"
        );
        assert!(
            polled.load(Ordering::SeqCst),
            "cleanup should attempt the disconnect"
        );
        assert!(tokio::time::Instant::now() - started_at < Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_cleanup_finishes_when_disconnect_returns_an_error() {
        // Contract: BLE-CLEANUP-01 in docs/test-contracts.md.
        let started_at = tokio::time::Instant::now();

        disconnect_with_timeout(async { Err::<(), _>(std::io::Error::other("disconnect failed")) })
            .await;

        assert_eq!(tokio::time::Instant::now(), started_at);
    }

    #[test]
    fn stop_uses_a_short_first_connection_window_then_a_bounded_recovery_window() {
        assert_eq!(
            connect_timeout(ConnectionPurpose::Stop, 1),
            Duration::from_secs(4)
        );
        assert_eq!(
            connect_timeout(ConnectionPurpose::Stop, 2),
            Duration::from_secs(6)
        );
        assert_eq!(
            connect_timeout(ConnectionPurpose::Command, 1),
            Duration::from_secs(8)
        );
        assert_eq!(
            connect_timeout(ConnectionPurpose::Command, 2),
            Duration::from_secs(6)
        );
        assert_eq!(
            connect_timeout(ConnectionPurpose::ReadOnly, 1),
            Duration::from_secs(6)
        );
    }

    #[test]
    fn cached_connection_reuse_requires_same_identity_and_a_fresh_session() {
        let now = std::time::Instant::now();
        let fresh = now - Duration::from_secs(4);
        let expired = now - Duration::from_secs(5);

        assert!(connection_reusable(
            "hallway-id",
            "HallwayBlind",
            fresh,
            "hallway-id",
            "HallwayBlind",
            now
        ));
        assert!(!connection_reusable(
            "hallway-id",
            "HallwayBlind",
            fresh,
            "other-id",
            "HallwayBlind",
            now
        ));
        assert!(!connection_reusable(
            "hallway-id",
            "HallwayBlind",
            fresh,
            "hallway-id",
            "BedroomBlind",
            now
        ));
        assert!(!connection_reusable(
            "hallway-id",
            "HallwayBlind",
            expired,
            "hallway-id",
            "HallwayBlind",
            now
        ));
    }

    #[test]
    fn cancellation_after_write_boundary_is_reported_as_uncertain() {
        let control = BleCommandControl::new();
        assert!(control.begin_write());
        assert!(!control.request_prewrite_cancel());

        let failure = control.cancellation_failure();
        assert_eq!(failure.stage, BleFailureStage::GattWrite);
        assert!(failure.write_may_have_been_sent);
        assert!(failure.outcome_unknown);
        assert!(!failure.retryable_before_write);
    }

    #[test]
    fn finds_readable_soma_vendor_battery_characteristic() {
        let make = |uuid: &str, properties| Characteristic {
            uuid: Uuid::parse_str(uuid).unwrap(),
            service_uuid: Uuid::parse_str("00001861-b87f-490c-92cb-11ba5ea5167c").unwrap(),
            properties,
            descriptors: BTreeSet::new(),
        };
        let vendor = make("0000ba71-b87f-490c-92cb-11ba5ea5167c", CharPropFlags::READ);
        let sig_base = make("0000ba71-0000-1000-8000-00805f9b34fb", CharPropFlags::READ);
        assert!(is_battery_characteristic(&vendor));
        assert!(!is_battery_characteristic(&sig_base));
        assert!(!is_battery_characteristic(&make(
            "0000ba71-b87f-490c-92cb-11ba5ea5167c",
            CharPropFlags::WRITE
        )));
    }
}
