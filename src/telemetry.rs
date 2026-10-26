use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReasonCodeTelemetry {
    #[serde(alias = "restart_reason_code")]
    pub restart_reason_code: Option<i64>,
    pub restart_reason_label: Option<String>,
    #[serde(alias = "motor_stop_reason_code")]
    pub motor_stop_reason_code: Option<i64>,
    pub motor_stop_reason_label: Option<String>,
}

pub fn resolve_restart_reason_label(code: Option<i64>) -> Option<String> {
    code.map(|code| match code {
        1 => "POWER_ON".to_owned(),
        2 => "SOFTWARE_RESET".to_owned(),
        3 => "WATCHDOG_RESET".to_owned(),
        4 => "BROWNOUT_RESET".to_owned(),
        value => format!("UNKNOWN_RESTART_REASON_{value}"),
    })
}

pub fn resolve_motor_stop_reason_label(code: Option<i64>) -> Option<String> {
    code.map(|code| match code {
        1 => "TARGET_REACHED".to_owned(),
        3 => "STALL_PROTECTION".to_owned(),
        5 => "LOW_BATTERY".to_owned(),
        value => format!("UNKNOWN_MOTOR_STOP_REASON_{value}"),
    })
}

pub fn normalize_reason_codes(value: &Value) -> ReasonCodeTelemetry {
    let restart_reason_code =
        first(value, &["restartReasonCode", "restart_reason_code"]).and_then(parse_integer);
    let motor_stop_reason_code =
        first(value, &["motorStopReasonCode", "motor_stop_reason_code"]).and_then(parse_integer);
    ReasonCodeTelemetry {
        restart_reason_code,
        restart_reason_label: resolve_restart_reason_label(restart_reason_code),
        motor_stop_reason_code,
        motor_stop_reason_label: resolve_motor_stop_reason_label(motor_stop_reason_code),
    }
}

fn first<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| value.get(*name))
}

fn parse_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Some(integer);
            }
            let number = number.as_f64()?;
            (number.is_finite()
                && number.fract() == 0.0
                && number >= i64::MIN as f64
                && number < 9_223_372_036_854_775_808.0)
                .then_some(number as i64)
        }
        Value::String(text) if text.trim().is_empty() => None,
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}
