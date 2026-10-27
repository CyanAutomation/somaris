use std::time::Duration;

use async_trait::async_trait;
use btleplug::{
    api::{
        Central, CentralEvent, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use tokio::sync::mpsc;
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
}

#[async_trait]
pub trait BleAdapter: Send + Sync {
    async fn scan(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
    ) -> Result<(), BleFailure>;

    async fn write_action(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
    ) -> Result<(), BleFailure>;

    async fn read_battery(
        &self,
        peripheral_id: &str,
        expected_name: &str,
    ) -> Result<Vec<u8>, BleFailure>;
}

#[derive(Clone)]
pub struct BtleplugAdapter {
    adapter: Adapter,
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

    async fn connect_and_discover(
        &self,
        peripheral_id: &str,
        expected_name: &str,
    ) -> Result<Peripheral, BleFailure> {
        let peripheral = self.peripheral_at(peripheral_id).await?;
        match tokio::time::timeout(Duration::from_secs(30), peripheral.connect()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "connect", %error, "BLE operation failed before write");
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
            }
            Err(_) => {
                tracing::warn!(
                    operation = "connect",
                    timeout_seconds = 30,
                    "BLE operation timed out before write"
                );
                return Err(BleFailure::before_write(
                    BleFailureStage::DeviceConnection,
                    true,
                ));
            }
        }
        match tokio::time::timeout(Duration::from_secs(15), peripheral.discover_services()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation = "discover_services", %error, "BLE service discovery failed before write");
                Self::disconnect(&peripheral).await;
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
                Self::disconnect(&peripheral).await;
                return Err(BleFailure::before_write(
                    BleFailureStage::ServiceResolution,
                    true,
                ));
            }
        }
        if let Err(error) = Self::verify_identity(&peripheral, expected_name).await {
            Self::disconnect(&peripheral).await;
            return Err(error);
        }
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
    ) -> Result<(), BleFailure> {
        let peripheral = self
            .connect_and_discover(peripheral_id, expected_name)
            .await?;
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
            Self::disconnect(&peripheral).await;
            return Err(BleFailure::before_write(
                BleFailureStage::GattDiscovery,
                false,
            ));
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            peripheral.write(
                &characteristic,
                &encode_service_action(action),
                WriteType::WithResponse,
            ),
        )
        .await;
        Self::disconnect(&peripheral).await;
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
        let peripheral = self
            .connect_and_discover(peripheral_id, expected_name)
            .await?;
        let characteristic_uuid = Uuid::from_u128(
            (BA71_CHARACTERISTIC_UUID as u128) << 96 | 0x0000_1000_8000_0080_5f9b_34fb,
        );
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|characteristic| {
                characteristic.uuid == characteristic_uuid
                    && characteristic.properties.contains(CharPropFlags::READ)
            });
        let Some(characteristic) = characteristic else {
            Self::disconnect(&peripheral).await;
            return Err(BleFailure::before_write(
                BleFailureStage::BatteryDiscovery,
                false,
            ));
        };
        let result =
            tokio::time::timeout(Duration::from_secs(10), peripheral.read(&characteristic)).await;
        Self::disconnect(&peripheral).await;
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
