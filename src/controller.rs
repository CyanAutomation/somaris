use thiserror::Error;

pub const OPEN_CLOSE_SERVICE_UUID: &str = "00001890-b87f-490c-92cb-11ba5ea5167c";
pub const OPEN_CLOSE_CHARACTERISTIC_UUID: &str = "00001896-b87f-490c-92cb-11ba5ea5167c";
pub const STOP_SERVICE_UUID: &str = "00001861-b87f-490c-92cb-11ba5ea5167c";
pub const STOP_CHARACTERISTIC_UUID: &str = "00001530-b87f-490c-92cb-11ba5ea5167c";
pub const MOTOR_SERVICE_UUID: &str = STOP_SERVICE_UUID;
pub const MOTOR_CONTROL_CHARACTERISTIC_UUID: &str = STOP_CHARACTERISTIC_UUID;
pub const POSITION_TARGET_CHARACTERISTIC_UUID: &str = "00001526-b87f-490c-92cb-11ba5ea5167c";
pub const CALIBRATION_CHARACTERISTIC_UUID: &str = "00001529-b87f-490c-92cb-11ba5ea5167c";
pub const SHADE_CONFIG_SERVICE_UUID: &str = OPEN_CLOSE_SERVICE_UUID;
pub const SHADE_CONFIG_CHARACTERISTIC_UUID: &str = OPEN_CLOSE_CHARACTERISTIC_UUID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerCommand {
    Open,
    Close,
    Stop,
    SetPosition(u8),
    GetVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Open,
    Close,
    Stop,
    SetTargetClosedPercent(u8),
    SetMotorSpeedPercent(u8),
    CalibrationStart,
    CalibrationStop,
    CalibrationNudgeOpen,
    CalibrationNudgeClosed,
    CalibrationSetOpenLimit,
    CalibrationSetClosedLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    Acknowledgement { operation: u8, status: u8 },
    Position { open_percent: u8 },
    Version(Version),
}

#[derive(Debug, Error, PartialEq)]
pub enum PositionError {
    #[error("Invalid position: {0}. Must be 0-100.")]
    OutOfRange(String),
    #[error("Invalid position: {0}. Must be a finite number from 0-100.")]
    NotFinite(String),
}

pub fn encode_controller_command(command: ControllerCommand) -> Vec<u8> {
    match command {
        ControllerCommand::Open => vec![0x01, 0x00, 0x00],
        ControllerCommand::Close => vec![0x02, 0x00, 0x00],
        ControllerCommand::Stop => vec![0x03, 0x00, 0x00],
        ControllerCommand::SetPosition(percent) => {
            vec![0x10, ((u16::from(percent) * 255 + 50) / 100) as u8]
        }
        ControllerCommand::GetVersion => vec![0x20],
    }
}

pub fn encode_service_action(action: ServiceAction) -> Vec<u8> {
    match action {
        ServiceAction::Open => vec![0x1a, 0x01, 0x00],
        ServiceAction::Close => vec![0x1a, 0x01, 0x64],
        ServiceAction::Stop => vec![0x00],
        ServiceAction::SetTargetClosedPercent(percent) => vec![percent],
        ServiceAction::SetMotorSpeedPercent(percent) => vec![0x01, 0x01, percent],
        ServiceAction::CalibrationStart => vec![0x76],
        ServiceAction::CalibrationStop => vec![0x36],
        ServiceAction::CalibrationNudgeOpen => vec![0x68],
        ServiceAction::CalibrationNudgeClosed => vec![0x86],
        ServiceAction::CalibrationSetOpenLimit => vec![0x69],
        ServiceAction::CalibrationSetClosedLimit => vec![0x96],
    }
}

/// Requests the motor-speed setting through SOMA's typed Shade Config protocol.
pub fn encode_motor_speed_readback_query() -> [u8; 3] {
    [0xff, 0x01, 0x01]
}

/// Extracts motor speed from a Shade Config response containing typed setting TLVs.
pub fn parse_motor_speed_readback(response: &[u8]) -> Option<u8> {
    if response.len() < 2 || response[0] != 0xff {
        return None;
    }
    // Byte 1 counts the TLV payload after the two-byte header.
    let payload_end = 2usize.checked_add(usize::from(response[1]))?;
    if response.len() < payload_end {
        return None;
    }
    let mut offset = 2;
    while offset < payload_end {
        let setting = *response.get(offset)?;
        let value_len = usize::from(*response.get(offset + 1)?);
        let value_start = offset + 2;
        let value_end = value_start.checked_add(value_len)?;
        if value_end > payload_end {
            return None;
        }
        let value = response.get(value_start..value_end)?;
        if setting == 0x01 {
            return (value.len() == 1 && value[0] <= 100).then_some(value[0]);
        }
        offset = value_end;
    }
    None
}

/// Motor-speed writes are supported by SOMA firmware 2.2.0 and newer.
/// Semantic pre-releases above 2.2.0 pass the threshold; pre-releases of 2.2.0 do not.
pub fn firmware_supports_motor_speed(version: &str) -> bool {
    let version = version.trim();
    let version = version
        .strip_prefix('v')
        .or_else(|| version.strip_prefix('V'))
        .unwrap_or(version);
    let (version, build_metadata) = version
        .split_once('+')
        .map_or((version, None), |(version, metadata)| {
            (version, Some(metadata))
        });
    if build_metadata.is_some_and(|metadata| !valid_version_identifiers(metadata, false)) {
        return false;
    }
    let (version, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(version, prerelease)| {
            (version, Some(prerelease))
        });
    if prerelease.is_some_and(|prerelease| !valid_version_identifiers(prerelease, true)) {
        return false;
    }
    let mut components = version.split('.');
    let (Some(major), Some(minor), Some(patch), None) = (
        components.next().and_then(parse_version_number),
        components.next().and_then(parse_version_number),
        components.next().and_then(parse_version_number),
        components.next(),
    ) else {
        return false;
    };
    match (major, minor, patch).cmp(&(2, 2, 0)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => prerelease.is_none(),
        std::cmp::Ordering::Less => false,
    }
}

fn parse_version_number(component: &str) -> Option<u64> {
    if component.is_empty()
        || (component.len() > 1 && component.starts_with('0'))
        || !component.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    component.parse().ok()
}

fn valid_version_identifiers(identifiers: &str, prerelease: bool) -> bool {
    !identifiers.is_empty()
        && identifiers.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!prerelease
                    || !identifier.bytes().all(|byte| byte.is_ascii_digit())
                    || identifier.len() == 1
                    || !identifier.starts_with('0'))
        })
}

pub fn service_action_target(action: ServiceAction) -> (&'static str, &'static str) {
    match action {
        ServiceAction::Open | ServiceAction::Close => {
            (OPEN_CLOSE_SERVICE_UUID, OPEN_CLOSE_CHARACTERISTIC_UUID)
        }
        ServiceAction::Stop => (STOP_SERVICE_UUID, STOP_CHARACTERISTIC_UUID),
        ServiceAction::SetTargetClosedPercent(_) => {
            (MOTOR_SERVICE_UUID, POSITION_TARGET_CHARACTERISTIC_UUID)
        }
        ServiceAction::SetMotorSpeedPercent(_) => {
            (SHADE_CONFIG_SERVICE_UUID, SHADE_CONFIG_CHARACTERISTIC_UUID)
        }
        ServiceAction::CalibrationStart
        | ServiceAction::CalibrationStop
        | ServiceAction::CalibrationSetOpenLimit
        | ServiceAction::CalibrationSetClosedLimit => {
            (MOTOR_SERVICE_UUID, CALIBRATION_CHARACTERISTIC_UUID)
        }
        ServiceAction::CalibrationNudgeOpen | ServiceAction::CalibrationNudgeClosed => {
            (MOTOR_SERVICE_UUID, MOTOR_CONTROL_CHARACTERISTIC_UUID)
        }
    }
}

pub fn set_position_byte(position: f64) -> Result<u8, PositionError> {
    if !position.is_finite() {
        return Err(PositionError::NotFinite(position.to_string()));
    }
    if !(0.0..=100.0).contains(&position) {
        return Err(PositionError::OutOfRange(position.to_string()));
    }
    Ok((position * 255.0 / 100.0).round() as u8)
}

pub fn open_percent_from_wire_byte(value: u8) -> u8 {
    ((u16::from(value) * 100 + 127) / 255) as u8
}

pub fn closed_percent_from_open_percent(open_percent: u8) -> u8 {
    100u8.saturating_sub(open_percent.min(100))
}

pub fn parse_notification(bytes: &[u8]) -> Option<Notification> {
    let operation = *bytes.first()?;
    match operation {
        0x01 if bytes.len() >= 3 => Some(Notification::Acknowledgement {
            operation: *bytes.get(1)?,
            status: *bytes.get(2)?,
        }),
        0x10 if bytes.len() >= 2 => Some(Notification::Position {
            open_percent: open_percent_from_wire_byte(*bytes.get(1)?),
        }),
        0x20 if bytes.len() >= 4 => Some(Notification::Version(Version {
            major: *bytes.get(1)?,
            minor: *bytes.get(2)?,
            patch: *bytes.get(3)?,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod shade_config_tests {
    use super::*;

    #[test]
    fn motor_speed_readback_uses_a_typed_shade_config_query() {
        assert_eq!(encode_motor_speed_readback_query(), [0xff, 0x01, 0x01]);
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x03, 0x01, 0x01, 37]),
            Some(37)
        );
    }

    #[test]
    fn motor_speed_readback_skips_other_settings_and_rejects_malformed_values() {
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x07, 0x02, 0x02, 0x10, 0x01, 0x01, 0x01, 42]),
            Some(42)
        );
        assert_eq!(parse_motor_speed_readback(&[0xff, 0x02, 0x01]), None);
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x03, 0x01, 0x01, 101]),
            None
        );
        assert_eq!(parse_motor_speed_readback(&[0x01, 0x01, 0x01, 37]), None);
    }

    #[test]
    fn motor_speed_readback_never_reads_beyond_the_declared_payload() {
        // A valid non-speed setting fills the declared payload; a speed TLV follows it.
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x03, 0x02, 0x01, 0x10, 0x01, 0x01, 42]),
            None
        );

        // The declared payload is truncated even though a complete speed TLV is present.
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x06, 0x01, 0x01, 42]),
            None
        );

        // A TLV value that crosses the declared boundary is malformed.
        assert_eq!(
            parse_motor_speed_readback(&[0xff, 0x02, 0x01, 0x01, 42]),
            None
        );
    }

    #[test]
    fn motor_speed_firmware_gate_requires_version_2_2_0() {
        assert!(!firmware_supports_motor_speed("2.1.9"));
        assert!(firmware_supports_motor_speed("2.2.0"));
        assert!(firmware_supports_motor_speed("2.2.0+build.7"));
        assert!(firmware_supports_motor_speed("2.3.0-beta"));
        assert!(firmware_supports_motor_speed("v2.3.1-beta"));
        assert!(!firmware_supports_motor_speed("2.2.0-beta"));
        assert!(!firmware_supports_motor_speed("2.2.0rc1"));
        assert!(!firmware_supports_motor_speed("2.3.0rc1"));
        assert!(!firmware_supports_motor_speed("unknown"));
        assert!(!firmware_supports_motor_speed("2.2"));
    }
}
