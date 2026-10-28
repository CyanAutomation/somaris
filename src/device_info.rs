use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, OnceCell};

use crate::telemetry::normalize_reason_codes;

type CachedRead = Arc<OnceCell<CachedValue>>;
type ReadCache = Arc<Mutex<HashMap<u16, CachedRead>>>;
const DEVICE_INFO_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

struct CachedValue {
    result: Result<String, String>,
    expires_at: tokio::time::Instant,
}

#[async_trait]
pub trait CharacteristicReader: Send + Sync {
    async fn read_characteristic(&self, uuid: u16) -> Result<String, String>;
}

#[derive(Clone)]
pub struct DeviceInfo {
    reader: Option<Arc<dyn CharacteristicReader>>,
    read_timeout: Duration,
    reads: ReadCache,
}

impl DeviceInfo {
    pub fn new(reader: Option<Arc<dyn CharacteristicReader>>, read_timeout: Duration) -> Self {
        Self {
            reader,
            read_timeout,
            reads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_device_version(&self) -> Result<String, String> {
        self.read_named(0x2a26, "device version").await
    }

    pub async fn get_device_model(&self) -> Result<String, String> {
        self.read_named(0x2a24, "device model").await
    }

    pub async fn get_device_manufacturer(&self) -> Result<String, String> {
        self.read_named(0x2a29, "device manufacturer").await
    }

    pub async fn get_all_device_info(&self) -> Result<DeviceInfoValues, String> {
        let (version, model, manufacturer) = tokio::try_join!(
            self.get_device_version(),
            self.get_device_model(),
            self.get_device_manufacturer()
        )?;
        Ok(DeviceInfoValues {
            version,
            model,
            manufacturer,
        })
    }

    pub async fn get_controller_info(&self, telemetry: &Value) -> Result<Value, String> {
        let sun = telemetry
            .get("sun")
            .or_else(|| telemetry.get("sun_times"))
            .unwrap_or(&Value::Null);
        let mut result = json!({
            "inputVoltageMv": optional_number(telemetry, &["inputVoltageMv", "input_voltage_mv"]),
            "batteryVoltageMv": optional_number(telemetry, &["batteryVoltageMv", "battery_voltage_mv", "battery_voltage"]),
            "bootCount": optional_integer(telemetry, &["bootCount", "boot_count"]),
            "undervoltageCount": optional_integer(telemetry, &["undervoltageCount", "undervoltage_count"]),
            "sunrise": optional_string_nested(telemetry, sun, &["sunrise", "sunrise_time"]),
            "sunset": optional_string_nested(telemetry, sun, &["sunset", "sunset_time"]),
            "connectId": optional_string(telemetry, &["connectId", "connect_id"]),
            "homekitCode": optional_string(telemetry, &["homekitCode", "homekit_code", "pairing_code"]),
            "hardwareRevision": optional_string(telemetry, &["hardwareRevision", "hardware_revision"]),
            "firmwareVersion": optional_string(telemetry, &["firmwareVersion", "firmware_version"]),
        });
        let reasons = normalize_reason_codes(telemetry);
        result["restartReasonCode"] = json!(reasons.restart_reason_code);
        result["restartReasonLabel"] = json!(reasons.restart_reason_label);
        result["motorStopReasonCode"] = json!(reasons.motor_stop_reason_code);
        result["motorStopReasonLabel"] = json!(reasons.motor_stop_reason_label);
        if result["firmwareVersion"].is_null() {
            result["firmwareVersion"] = json!(self.get_device_version().await?);
        }
        Ok(result)
    }

    async fn read_named(&self, uuid: u16, field: &str) -> Result<String, String> {
        let Some(reader) = self.reader.clone() else {
            return Err(format!(
                "Failed to read {field}: hciSocket.readCharacteristic(uuid) is required for device info reads"
            ));
        };
        let cell = {
            let mut reads = self.reads.lock().await;
            if reads
                .get(&uuid)
                .and_then(|cell| cell.get())
                .is_some_and(|cached| cached.expires_at <= tokio::time::Instant::now())
            {
                reads.remove(&uuid);
            }
            reads
                .entry(uuid)
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let timeout = self.read_timeout;
        let result = cell
            .get_or_init(|| async move {
                let result =
                    match tokio::time::timeout(timeout, reader.read_characteristic(uuid)).await {
                        Ok(result) => result,
                        Err(_) => Err(format!(
                            "Timed out reading characteristic 0x{uuid:04x} after {}ms",
                            timeout.as_millis()
                        )),
                    };
                CachedValue {
                    result,
                    expires_at: tokio::time::Instant::now() + DEVICE_INFO_CACHE_TTL,
                }
            })
            .await
            .result
            .clone();
        if result.is_err() {
            let mut reads = self.reads.lock().await;
            if reads
                .get(&uuid)
                .is_some_and(|cached| Arc::ptr_eq(cached, &cell))
            {
                reads.remove(&uuid);
            }
        }
        result.map_err(|error| format!("Failed to read {field}: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfoValues {
    pub version: String,
    pub model: String,
    pub manufacturer: String,
}

fn optional_number(value: &Value, names: &[&str]) -> Value {
    names
        .iter()
        .find_map(|name| value.get(*name).filter(|value| value.is_number()).cloned())
        .unwrap_or(Value::Null)
}

fn optional_integer(value: &Value, names: &[&str]) -> Value {
    names
        .iter()
        .find_map(|name| {
            value.get(*name).and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.trim().parse().ok())
            })
        })
        .map_or(Value::Null, |value| json!(value))
}

fn optional_string(value: &Value, names: &[&str]) -> Value {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .map_or(Value::Null, |value| json!(value))
}

fn optional_string_nested(value: &Value, nested: &Value, names: &[&str]) -> Value {
    let direct = optional_string(value, names);
    if direct.is_null() {
        optional_string(nested, names)
    } else {
        direct
    }
}
