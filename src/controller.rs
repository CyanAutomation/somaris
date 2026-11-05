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
