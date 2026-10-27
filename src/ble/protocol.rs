use thiserror::Error;

/// Bluetooth SIG company identifier decoded from the little-endian manufacturer field.
pub const SOMA_MANUFACTURER_ID: u16 = 0x0370;
/// Payload version observed in the documented SOMA V2 advertisements.
pub const SOMA_PAYLOAD_VERSION: u8 = 0x05;
/// Vendor GATT characteristic used for the BA71 voltage reading.
pub const BA71_CHARACTERISTIC_UUID: u16 = 0xBA71;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSomaAdvertisement {
    pub advertised_name: String,
    pub current_closed_percent: u8,
    pub target_closed_percent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading {
    pub voltage_mv: u16,
    pub percent_estimate: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BleDecodeError {
    #[error("SOMA advertisement payload is too short")]
    PayloadTooShort,
    #[error("unsupported SOMA advertisement payload version {0}")]
    UnsupportedVersion(u8),
    #[error("SOMA advertisement does not contain a device name")]
    MissingAdvertisedName,
    #[error("SOMA advertisement name is not valid UTF-8")]
    InvalidAdvertisedName,
    #[error("SOMA position value is outside 0 to 100 percent")]
    PositionOutOfRange,
    #[error("BA71 characteristic value is not a valid 16-bit voltage")]
    InvalidBatteryValue,
}

/// Decodes the SOMA V2 manufacturer payload observed through the API.
///
/// The captured format has version byte 0, position bytes 2 and 3, and a
/// space-padded UTF-8 advertised name beginning at byte 4. The second byte is
/// retained by callers as raw telemetry but is not interpreted here.
pub fn parse_soma_payload(payload: &[u8]) -> Result<ParsedSomaAdvertisement, BleDecodeError> {
    if payload.len() < 4 {
        return Err(BleDecodeError::PayloadTooShort);
    }
    if payload[0] != SOMA_PAYLOAD_VERSION {
        return Err(BleDecodeError::UnsupportedVersion(payload[0]));
    }
    let name = std::str::from_utf8(&payload[4..])
        .map_err(|_| BleDecodeError::InvalidAdvertisedName)?
        .trim_matches(|character: char| character.is_whitespace() || character == '\0');
    if name.is_empty() {
        return Err(BleDecodeError::MissingAdvertisedName);
    }
    if name.chars().any(char::is_control) {
        return Err(BleDecodeError::InvalidAdvertisedName);
    }

    let current_closed_percent = payload[2];
    let target_closed_percent = payload[3];
    if current_closed_percent > 100 || target_closed_percent > 100 {
        return Err(BleDecodeError::PositionOutOfRange);
    }
    Ok(ParsedSomaAdvertisement {
        advertised_name: name.to_owned(),
        current_closed_percent,
        target_closed_percent,
    })
}

/// BA71 reports a little-endian count of 10 mV units. The estimate follows
/// the API's documented 3600-4100 mV linear range and clamps at either end.
pub fn decode_ba71_voltage(value: &[u8]) -> Result<BatteryReading, BleDecodeError> {
    let bytes: [u8; 2] = value
        .try_into()
        .map_err(|_| BleDecodeError::InvalidBatteryValue)?;
    let voltage_mv = u16::from_le_bytes(bytes)
        .checked_mul(10)
        .filter(|voltage| *voltage > 0)
        .ok_or(BleDecodeError::InvalidBatteryValue)?;
    let percent_estimate = match voltage_mv {
        ..=3600 => 0,
        4100.. => 100,
        voltage => (((voltage - 3600) as u32 * 100) / 500) as u8,
    };
    Ok(BatteryReading {
        voltage_mv,
        percent_estimate,
    })
}
