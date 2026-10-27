mod hardware;
mod protocol;

pub use hardware::{BleAdapter, BleFailure, BleFailureStage, BleObservation, BtleplugAdapter};
pub use protocol::{
    BA71_CHARACTERISTIC_UUID, BatteryReading, BleDecodeError, ParsedSomaAdvertisement,
    SOMA_MANUFACTURER_ID, SOMA_PAYLOAD_VERSION, decode_ba71_voltage, parse_soma_payload,
};
