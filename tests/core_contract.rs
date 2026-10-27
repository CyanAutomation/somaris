use async_trait::async_trait;
use serde_json::json;
use somaris::controller::{
    ControllerCommand, Notification, OPEN_CLOSE_CHARACTERISTIC_UUID, OPEN_CLOSE_SERVICE_UUID,
    PositionError, STOP_CHARACTERISTIC_UUID, STOP_SERVICE_UUID, ServiceAction, Version,
    closed_percent_from_open_percent, encode_controller_command, encode_service_action,
    open_percent_from_wire_byte, parse_notification, set_position_byte,
};
use somaris::device_info::{CharacteristicReader, DeviceInfo};
use somaris::telemetry::{
    normalize_reason_codes, resolve_motor_stop_reason_label, resolve_restart_reason_label,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct MockReader {
    reads: AtomicUsize,
    delay: Duration,
}

#[async_trait]
impl CharacteristicReader for MockReader {
    async fn read_characteristic(&self, uuid: u16) -> Result<String, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(match uuid {
            0x2a26 => "2.1.0".into(),
            0x2a24 => "Blind Controller v2".into(),
            0x2a29 => "Example Manufacturer".into(),
            _ => "Unknown".into(),
        })
    }
}

#[test]
fn controller_wire_positions_and_dashboard_positions_have_explicit_conventions() {
    assert_eq!(open_percent_from_wire_byte(0x00), 0);
    assert_eq!(open_percent_from_wire_byte(0x80), 50);
    assert_eq!(open_percent_from_wire_byte(0xff), 100);

    assert_eq!(closed_percent_from_open_percent(0), 100);
    assert_eq!(closed_percent_from_open_percent(50), 50);
    assert_eq!(closed_percent_from_open_percent(100), 0);
}

#[test]
fn controller_commands_keep_the_tested_opcodes() {
    assert_eq!(encode_controller_command(ControllerCommand::Open)[0], 0x01);
    assert_eq!(encode_controller_command(ControllerCommand::Close)[0], 0x02);
    assert_eq!(encode_controller_command(ControllerCommand::Stop)[0], 0x03);
    assert_eq!(
        encode_controller_command(ControllerCommand::SetPosition(50)),
        [0x10, 0x80]
    );
    assert_eq!(encode_service_action(ServiceAction::Stop), [0x00]);
    assert_eq!(
        OPEN_CLOSE_SERVICE_UUID,
        "00001890-b87f-490c-92cb-11ba5ea5167c"
    );
    assert_eq!(
        OPEN_CLOSE_CHARACTERISTIC_UUID,
        "00001896-b87f-490c-92cb-11ba5ea5167c"
    );
    assert_eq!(STOP_SERVICE_UUID, "00001861-b87f-490c-92cb-11ba5ea5167c");
    assert_eq!(
        STOP_CHARACTERISTIC_UUID,
        "00001530-b87f-490c-92cb-11ba5ea5167c"
    );
    assert_eq!(
        encode_service_action(ServiceAction::Open),
        [0x1a, 0x01, 0x00]
    );
    assert_eq!(
        encode_service_action(ServiceAction::Close),
        [0x1a, 0x01, 0x64]
    );
    assert_eq!(encode_service_action(ServiceAction::Stop), [0x00]);
}

#[test]
fn controller_rejects_invalid_positions_before_encoding() {
    assert_eq!(set_position_byte(0.0), Ok(0));
    assert_eq!(set_position_byte(50.0), Ok(128));
    assert_eq!(set_position_byte(100.0), Ok(255));
    assert!(matches!(
        set_position_byte(f64::NAN),
        Err(PositionError::NotFinite(_))
    ));
    assert!(matches!(
        set_position_byte(f64::INFINITY),
        Err(PositionError::NotFinite(_))
    ));
    assert!(matches!(
        set_position_byte(-1.0),
        Err(PositionError::OutOfRange(_))
    ));
    assert!(matches!(
        set_position_byte(101.0),
        Err(PositionError::OutOfRange(_))
    ));
}

#[test]
fn truncated_notifications_are_ignored_and_complete_frames_are_decoded() {
    assert_eq!(parse_notification(&[0x01]), None);
    assert_eq!(parse_notification(&[0x10]), None);
    assert_eq!(parse_notification(&[0x20, 1, 2]), None);
    assert_eq!(
        parse_notification(&[0x10, 0x80]),
        Some(Notification::Position { open_percent: 50 })
    );
    assert_eq!(
        parse_notification(&[0x20, 1, 8, 3]),
        Some(Notification::Version(Version {
            major: 1,
            minor: 8,
            patch: 3
        }))
    );
    assert_eq!(
        parse_notification(&[0x01, 0x02, 0x00, 0x00]),
        Some(Notification::Acknowledgement {
            operation: 0x02,
            status: 0
        })
    );
}

#[test]
fn reason_labels_preserve_codes_and_supply_known_or_fallback_labels() {
    assert_eq!(
        resolve_restart_reason_label(Some(1)).as_deref(),
        Some("POWER_ON")
    );
    assert_eq!(
        resolve_restart_reason_label(Some(3)).as_deref(),
        Some("WATCHDOG_RESET")
    );
    assert_eq!(
        resolve_restart_reason_label(Some(88)).as_deref(),
        Some("UNKNOWN_RESTART_REASON_88")
    );
    assert_eq!(resolve_restart_reason_label(None), None);

    assert_eq!(
        resolve_motor_stop_reason_label(Some(1)).as_deref(),
        Some("TARGET_REACHED")
    );
    assert_eq!(
        resolve_motor_stop_reason_label(Some(5)).as_deref(),
        Some("LOW_BATTERY")
    );
    assert_eq!(
        resolve_motor_stop_reason_label(Some(42)).as_deref(),
        Some("UNKNOWN_MOTOR_STOP_REASON_42")
    );
    assert_eq!(resolve_motor_stop_reason_label(None), None);
}

#[test]
fn reason_telemetry_accepts_integer_strings_but_rejects_blank_and_fractional_values() {
    let normalized = normalize_reason_codes(&json!({
        "restart_reason_code": "4",
        "motor_stop_reason_code": 5
    }));
    assert_eq!(normalized.restart_reason_code, Some(4));
    assert_eq!(
        normalized.restart_reason_label.as_deref(),
        Some("BROWNOUT_RESET")
    );
    assert_eq!(normalized.motor_stop_reason_code, Some(5));
    assert_eq!(
        normalized.motor_stop_reason_label.as_deref(),
        Some("LOW_BATTERY")
    );

    let invalid = normalize_reason_codes(&json!({
        "restartReasonCode": "   ",
        "motorStopReasonCode": 3.25
    }));
    assert_eq!(invalid.restart_reason_code, None);
    assert_eq!(invalid.restart_reason_label, None);
    assert_eq!(invalid.motor_stop_reason_code, None);
    assert_eq!(invalid.motor_stop_reason_label, None);
}

#[tokio::test]
async fn device_info_reads_are_shared_while_in_flight() {
    let reader = Arc::new(MockReader {
        reads: AtomicUsize::new(0),
        delay: Duration::from_millis(10),
    });
    let info = DeviceInfo::new(Some(reader.clone()), Duration::from_secs(1));
    let (first, second) = tokio::join!(info.get_device_version(), info.get_device_version());
    assert_eq!(first.as_deref(), Ok("2.1.0"));
    assert_eq!(second.as_deref(), Ok("2.1.0"));
    assert_eq!(reader.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn device_info_read_timeout_and_controller_metadata_are_explicit() {
    let reader = Arc::new(MockReader {
        reads: AtomicUsize::new(0),
        delay: Duration::from_millis(30),
    });
    let timed = DeviceInfo::new(Some(reader.clone()), Duration::from_millis(1));
    let error = timed.get_device_version().await.unwrap_err();
    assert!(error.contains("Timed out reading characteristic 0x2a26 after 1ms"));

    let info = DeviceInfo::new(Some(reader), Duration::from_secs(1));
    let details = info
        .get_controller_info(&json!({
            "input_voltage_mv": 5100,
            "battery_voltage": 3980,
            "boot_count": 3,
            "restart_reason_code": 1,
            "motor_stop_reason_code": 3,
            "sun": { "sunrise_time": "06:15", "sunset_time": "19:40" }
        }))
        .await
        .unwrap();
    assert_eq!(details["inputVoltageMv"], 5100);
    assert_eq!(details["batteryVoltageMv"], 3980);
    assert_eq!(details["restartReasonLabel"], "POWER_ON");
    assert_eq!(details["motorStopReasonLabel"], "STALL_PROTECTION");
    assert_eq!(details["sunrise"], "06:15");
    assert_eq!(details["sunset"], "19:40");
    assert_eq!(details["firmwareVersion"], "2.1.0");
}
