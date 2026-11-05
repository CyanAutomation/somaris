use std::{
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
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

use crate::controller::{
    OPEN_CLOSE_CHARACTERISTIC_UUID, OPEN_CLOSE_SERVICE_UUID, STOP_CHARACTERISTIC_UUID,
    STOP_SERVICE_UUID, ServiceAction, encode_service_action,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleFailure {
    pub stage: BleFailureStage,
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
    GattWrite,
    Disconnect,
}

impl BleFailure {
    fn before_write(stage: BleFailureStage, retryable: bool) -> Self {
        Self {
            stage,
            retryable_before_write: retryable,
            write_may_have_been_sent: false,
            outcome_unknown: false,
        }
    }

    fn during_write(outcome_unknown: bool) -> Self {
        Self {
            stage: BleFailureStage::GattWrite,
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
            retryable_before_write: !write_may_have_been_sent,
            write_may_have_been_sent,
            outcome_unknown: write_may_have_been_sent,
        }
    }

    pub(crate) fn command_preempted() -> Self {
        Self {
            stage: BleFailureStage::CommandPreempted,
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
}

#[derive(Clone)]
pub struct BtleplugAdapter {
    adapter: Adapter,
}

fn connect_timeout(action: ServiceAction, attempt: u8) -> Duration {
    match (action, attempt) {
        (ServiceAction::Stop, 1) => Duration::from_secs(12),
        (ServiceAction::Stop, _) => Duration::from_secs(20),
        _ => Duration::from_secs(30),
    }
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
            BtleplugAdapter::disconnect_after_interruption(&peripheral).await;
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
        Ok(Self { adapter })
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
        action: ServiceAction,
        attempt: u8,
        control: &BleCommandControl,
    ) -> Result<Peripheral, BleFailure> {
        let lookup_started = std::time::Instant::now();
        let lookup_result = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(control.cancellation_failure()),
            result = self.peripheral_at(peripheral_id) => result,
        };
        control.record_phase(BlePhase::PeripheralLookup, lookup_started.elapsed());
        let peripheral = lookup_result?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let connect_timeout = connect_timeout(action, attempt);
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
                let disconnect_started = std::time::Instant::now();
                disconnect_guard.abort_connection().await;
                control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
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
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
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
        match tokio::time::timeout(Duration::from_secs(3), peripheral.is_connected()).await {
            Ok(Ok(false)) => return,
            Ok(Ok(true)) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "connection_state", %error, "Could not confirm BLE connection state before disconnect");
                return;
            }
            Err(_) => {
                tracing::warn!(
                    operation = "connection_state",
                    timeout_seconds = 3,
                    "BLE connection state check timed out before disconnect"
                );
                return;
            }
        }
        match tokio::time::timeout(Duration::from_secs(5), peripheral.disconnect()).await {
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

    async fn disconnect_after_interruption(peripheral: &Peripheral) {
        match tokio::time::timeout(Duration::from_secs(5), peripheral.disconnect()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "disconnect", %error, "BLE connection cleanup after interruption failed");
            }
            Err(_) => {
                tracing::warn!(
                    operation = "disconnect",
                    timeout_seconds = 5,
                    "BLE connection cleanup after interruption timed out"
                );
            }
        }
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
}

#[async_trait]
impl BleAdapter for BtleplugAdapter {
    async fn scan(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
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

        let scan_result = tokio::time::timeout(window, async {
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
        })
        .await;
        self.stop_scan().await?;
        match scan_result {
            Err(_) => Ok(()),
            Ok(result) => result,
        }
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
                action,
                attempt,
                control,
            )
            .await?;
        let mut disconnect_guard = DisconnectGuard::new(&peripheral);
        let (service_uuid, characteristic_uuid) = match action {
            ServiceAction::Open | ServiceAction::Close => {
                (OPEN_CLOSE_SERVICE_UUID, OPEN_CLOSE_CHARACTERISTIC_UUID)
            }
            ServiceAction::Stop => (STOP_SERVICE_UUID, STOP_CHARACTERISTIC_UUID),
        };
        let service_uuid = Uuid::parse_str(service_uuid)
            .map_err(|_| BleFailure::before_write(BleFailureStage::GattDiscovery, false))?;
        let characteristic_uuid = Uuid::parse_str(characteristic_uuid)
            .map_err(|_| BleFailure::before_write(BleFailureStage::GattDiscovery, false))?;
        let discovery_started = std::time::Instant::now();
        let characteristics = peripheral.characteristics();
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
        let disconnect_started = std::time::Instant::now();
        disconnect_guard.disconnect().await;
        control.record_phase(BlePhase::Disconnect, disconnect_started.elapsed());
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::warn!(operation = "gatt_write", %error, "BLE write failed after write may have been sent");
                Err(BleFailure::during_write(false))
            }
            Err(_) => {
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
                ServiceAction::Stop,
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
}

fn is_battery_characteristic(characteristic: &Characteristic) -> bool {
    characteristic.uuid == BA71_CHARACTERISTIC_UUID
        && characteristic.properties.contains(CharPropFlags::READ)
}

#[cfg(test)]
mod battery_tests {
    use super::*;
    use btleplug::api::Characteristic;
    use std::collections::BTreeSet;

    #[test]
    fn stop_uses_a_short_first_connection_window_then_a_bounded_recovery_window() {
        assert_eq!(
            connect_timeout(ServiceAction::Stop, 1),
            Duration::from_secs(12)
        );
        assert_eq!(
            connect_timeout(ServiceAction::Stop, 2),
            Duration::from_secs(20)
        );
        assert_eq!(
            connect_timeout(ServiceAction::Close, 1),
            Duration::from_secs(30)
        );
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
