use std::{collections::HashMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    #[serde(rename = "env")]
    Environment,
    File,
    Default,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingValue<T> {
    pub value: T,
    pub default: T,
    pub source: SettingSource,
    pub locked: bool,
    pub requires_restart: bool,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsValues {
    pub simulator_enabled: SettingValue<bool>,
    pub allowed_origins: SettingValue<Vec<String>>,
    pub log_level: SettingValue<String>,
    pub history_api_enabled: SettingValue<bool>,
    pub history_retention: SettingValue<usize>,
    pub queue_max_outstanding: SettingValue<usize>,
    pub queue_max_queued_per_device: SettingValue<usize>,
    pub adapter_concurrency: SettingValue<usize>,
    pub adapter_scan_window_ms: SettingValue<u64>,
    pub queue_wait_timeout_seconds: SettingValue<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsSnapshot {
    pub values: SettingsValues,
    pub pending_restart: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsDocument {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulator_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_origins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_api_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_retention: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_max_outstanding: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_max_queued_per_device: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_concurrency: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_scan_window_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_wait_timeout_seconds: Option<u64>,
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsUpdate {
    pub simulator_enabled: Option<bool>,
    pub allowed_origins: Option<Vec<String>>,
    pub log_level: Option<String>,
    pub history_api_enabled: Option<bool>,
    pub history_retention: Option<usize>,
    pub queue_max_outstanding: Option<usize>,
    pub queue_max_queued_per_device: Option<usize>,
    pub adapter_concurrency: Option<usize>,
    pub adapter_scan_window_ms: Option<u64>,
    pub queue_wait_timeout_seconds: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsReset {
    pub key: Option<String>,
}

impl SettingsUpdate {
    pub fn keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        macro_rules! add_key {
            ($field:ident) => {
                if self.$field.is_some() {
                    keys.push(stringify!($field));
                }
            };
        }
        add_key!(simulator_enabled);
        add_key!(allowed_origins);
        add_key!(log_level);
        add_key!(history_api_enabled);
        add_key!(history_retention);
        add_key!(queue_max_outstanding);
        add_key!(queue_max_queued_per_device);
        add_key!(adapter_concurrency);
        add_key!(adapter_scan_window_ms);
        add_key!(queue_wait_timeout_seconds);
        keys
    }
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self {
            version: 1,
            simulator_enabled: None,
            allowed_origins: None,
            log_level: None,
            history_api_enabled: None,
            history_retention: None,
            queue_max_outstanding: None,
            queue_max_queued_per_device: None,
            adapter_concurrency: None,
            adapter_scan_window_ms: None,
            queue_wait_timeout_seconds: None,
            unknown: Map::new(),
        }
    }
}

impl SettingsDocument {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("settings file is invalid: {error}"),
                )
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.simulator_enabled.is_none()
            && self.allowed_origins.is_none()
            && self.log_level.is_none()
            && self.history_api_enabled.is_none()
            && self.history_retention.is_none()
            && self.queue_max_outstanding.is_none()
            && self.queue_max_queued_per_device.is_none()
            && self.adapter_concurrency.is_none()
            && self.adapter_scan_window_ms.is_none()
            && self.queue_wait_timeout_seconds.is_none()
            && self.unknown.is_empty()
    }

    pub fn apply(&mut self, update: SettingsUpdate) -> Vec<&'static str> {
        let mut changed = Vec::new();
        macro_rules! assign {
            ($field:ident) => {
                if let Some(value) = update.$field
                    && self.$field.as_ref() != Some(&value)
                {
                    self.$field = Some(value);
                    changed.push(stringify!($field));
                }
            };
        }
        assign!(simulator_enabled);
        assign!(allowed_origins);
        assign!(log_level);
        assign!(history_api_enabled);
        assign!(history_retention);
        assign!(queue_max_outstanding);
        assign!(queue_max_queued_per_device);
        assign!(adapter_concurrency);
        assign!(adapter_scan_window_ms);
        assign!(queue_wait_timeout_seconds);
        changed
    }

    pub fn reset(&mut self, key: Option<&str>) -> Result<Vec<&'static str>, String> {
        let keys = match key {
            Some(key) => vec![key],
            None => vec![
                "simulator_enabled",
                "allowed_origins",
                "log_level",
                "history_api_enabled",
                "history_retention",
                "queue_max_outstanding",
                "queue_max_queued_per_device",
                "adapter_concurrency",
                "adapter_scan_window_ms",
                "queue_wait_timeout_seconds",
            ],
        };
        let mut reset = Vec::new();
        for key in keys {
            let removed = match key {
                "simulator_enabled" => self.simulator_enabled.take().is_some(),
                "allowed_origins" => self.allowed_origins.take().is_some(),
                "log_level" => self.log_level.take().is_some(),
                "history_api_enabled" => self.history_api_enabled.take().is_some(),
                "history_retention" => self.history_retention.take().is_some(),
                "queue_max_outstanding" => self.queue_max_outstanding.take().is_some(),
                "queue_max_queued_per_device" => self.queue_max_queued_per_device.take().is_some(),
                "adapter_concurrency" => self.adapter_concurrency.take().is_some(),
                "adapter_scan_window_ms" => self.adapter_scan_window_ms.take().is_some(),
                "queue_wait_timeout_seconds" => self.queue_wait_timeout_seconds.take().is_some(),
                _ => return Err(format!("Unknown setting key: {key}")),
            };
            if removed {
                reset.push(match key {
                    "simulator_enabled" => "simulator_enabled",
                    "allowed_origins" => "allowed_origins",
                    "log_level" => "log_level",
                    "history_api_enabled" => "history_api_enabled",
                    "history_retention" => "history_retention",
                    "queue_max_outstanding" => "queue_max_outstanding",
                    "queue_max_queued_per_device" => "queue_max_queued_per_device",
                    "adapter_concurrency" => "adapter_concurrency",
                    "adapter_scan_window_ms" => "adapter_scan_window_ms",
                    "queue_wait_timeout_seconds" => "queue_wait_timeout_seconds",
                    _ => unreachable!(),
                });
            }
        }
        if key.is_none() {
            self.unknown.clear();
        }
        Ok(reset)
    }
}

pub fn resolve(
    document: &SettingsDocument,
    environment: &HashMap<String, String>,
) -> Result<SettingsSnapshot, String> {
    if document.version != 1 {
        return Err(format!(
            "Unsupported settings document version {}; this service supports version 1",
            document.version
        ));
    }
    let default_queue_outstanding = 1000;
    let default_queue_per_device = 32;
    let default_adapter_concurrency = 1;
    let default_scan_window_ms = 1000;
    let default_queue_wait_seconds = 120;

    let simulator_enabled = choose(
        parse_environment_bool(environment, "SOMARIS_SIMULATOR")?,
        document.simulator_enabled,
        true,
    );
    let allowed_origins = if let Some(value) = environment.get("SOMARIS_ALLOWED_ORIGINS") {
        Chosen {
            value: parse_allowed_origins(value)?,
            default: Vec::new(),
            source: SettingSource::Environment,
            locked: true,
        }
    } else if let Some(value) = &document.allowed_origins {
        Chosen {
            value: validate_origins(value.clone())?,
            default: Vec::new(),
            source: SettingSource::File,
            locked: false,
        }
    } else {
        choose(None, None, Vec::new())
    };
    let log_level = if let Some(value) = environment.get("RUST_LOG") {
        Chosen {
            value: summarize_rust_log_level(value),
            default: "info".to_owned(),
            source: SettingSource::Environment,
            locked: true,
        }
    } else if let Some(value) = &document.log_level {
        Chosen {
            value: validate_log_level(value)?,
            default: "info".to_owned(),
            source: SettingSource::File,
            locked: false,
        }
    } else {
        choose(None, None, "info".to_owned())
    };
    let history_api_enabled = choose(
        parse_history_environment(environment)?,
        document.history_api_enabled,
        false,
    );
    let history_retention = choose(
        parse_environment_number(environment, "SOMARIS_HISTORY_RETENTION")?,
        document.history_retention,
        50_000,
    );
    let queue_max_outstanding = choose(
        parse_environment_number(environment, "SOMARIS_COMMAND_MAX_OUTSTANDING")?,
        document.queue_max_outstanding,
        default_queue_outstanding,
    );
    let queue_max_queued_per_device = choose(
        parse_environment_number(environment, "SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE")?,
        document.queue_max_queued_per_device,
        default_queue_per_device,
    );
    let adapter_concurrency = choose(
        parse_environment_number(environment, "SOMARIS_ADAPTER_CONCURRENCY")?,
        document.adapter_concurrency,
        default_adapter_concurrency,
    );
    let adapter_scan_window_ms = choose(
        parse_environment_number(environment, "SOMARIS_ADAPTER_SCAN_WINDOW_MS")?,
        document.adapter_scan_window_ms,
        default_scan_window_ms,
    );
    let queue_wait_timeout_seconds = choose(
        parse_environment_number(environment, "SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS")?,
        document.queue_wait_timeout_seconds,
        default_queue_wait_seconds,
    );

    validate_range("history_retention", history_retention.value, 100, 500_000)?;
    validate_range(
        "queue_max_outstanding",
        queue_max_outstanding.value,
        1,
        1000,
    )?;
    validate_range(
        "queue_max_queued_per_device",
        queue_max_queued_per_device.value,
        1,
        queue_max_outstanding.value,
    )?;
    validate_range("adapter_concurrency", adapter_concurrency.value, 1, 64)?;
    validate_range(
        "adapter_scan_window_ms",
        adapter_scan_window_ms.value,
        100,
        10_000,
    )?;
    validate_range(
        "queue_wait_timeout_seconds",
        queue_wait_timeout_seconds.value,
        1,
        86_400,
    )?;

    Ok(SettingsSnapshot {
        values: SettingsValues {
            simulator_enabled: metadata(simulator_enabled, false, vec!["boolean".into()]),
            allowed_origins: metadata(
                allowed_origins,
                false,
                vec!["comma-separated HTTP or HTTPS origins".into()],
            ),
            log_level: metadata(
                log_level,
                false,
                vec!["error".into(), "warn".into(), "info".into(), "debug".into()],
            ),
            history_api_enabled: metadata(history_api_enabled, false, vec!["boolean".into()]),
            history_retention: metadata(
                history_retention,
                false,
                vec!["100..500000 samples".into()],
            ),
            queue_max_outstanding: metadata(
                queue_max_outstanding,
                false,
                vec!["1..1000 commands".into()],
            ),
            queue_max_queued_per_device: metadata(
                queue_max_queued_per_device,
                false,
                vec!["1..queue_max_outstanding commands".into()],
            ),
            adapter_concurrency: metadata(
                adapter_concurrency,
                true,
                vec!["1..64 operations".into()],
            ),
            adapter_scan_window_ms: metadata(
                adapter_scan_window_ms,
                false,
                vec!["100..10000 milliseconds".into()],
            ),
            queue_wait_timeout_seconds: metadata(
                queue_wait_timeout_seconds,
                false,
                vec!["1..86400 seconds".into()],
            ),
        },
        pending_restart: Vec::new(),
    })
}

#[derive(Debug, Clone)]
struct Chosen<T> {
    value: T,
    default: T,
    source: SettingSource,
    locked: bool,
}

fn choose<T: Clone>(environment: Option<(T, bool)>, file: Option<T>, default: T) -> Chosen<T> {
    if let Some((value, from_environment)) = environment {
        return Chosen {
            value,
            default,
            source: if from_environment {
                SettingSource::Environment
            } else {
                SettingSource::File
            },
            locked: from_environment,
        };
    }
    if let Some(value) = file {
        return Chosen {
            value,
            default,
            source: SettingSource::File,
            locked: false,
        };
    }
    Chosen {
        value: default.clone(),
        default,
        source: SettingSource::Default,
        locked: false,
    }
}

fn metadata<T>(
    chosen: Chosen<T>,
    requires_restart: bool,
    constraints: Vec<String>,
) -> SettingValue<T> {
    SettingValue {
        value: chosen.value,
        default: chosen.default,
        source: chosen.source,
        locked: chosen.locked,
        requires_restart,
        constraints,
    }
}

fn parse_environment_bool(
    environment: &HashMap<String, String>,
    key: &str,
) -> Result<Option<(bool, bool)>, String> {
    environment
        .get(key)
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" => Ok((true, true)),
            "0" | "false" | "off" => Ok((false, true)),
            _ => Err(format!("{key} must be a boolean")),
        })
        .transpose()
}

fn parse_environment_number<T>(
    environment: &HashMap<String, String>,
    key: &str,
) -> Result<Option<(T, bool)>, String>
where
    T: std::str::FromStr,
{
    environment
        .get(key)
        .map(|value| {
            value
                .parse::<T>()
                .map(|number| (number, true))
                .map_err(|_| format!("{key} must be a positive integer"))
        })
        .transpose()
}

fn parse_history_environment(
    environment: &HashMap<String, String>,
) -> Result<Option<(bool, bool)>, String> {
    if let Some(value) = parse_environment_bool(environment, "SOMARIS_HISTORY_API_ENABLED")? {
        return Ok(Some(value));
    }
    if environment.contains_key("SOMARIS_HISTORY_PATH") {
        return Ok(Some((true, true)));
    }
    Ok(None)
}

fn validate_range<T>(name: &str, value: T, minimum: T, maximum: T) -> Result<(), String>
where
    T: PartialOrd + std::fmt::Display,
{
    if value < minimum || value > maximum {
        Err(format!("{name} must be between {minimum} and {maximum}"))
    } else {
        Ok(())
    }
}

fn validate_log_level(value: &str) -> Result<String, String> {
    let level = value.trim().to_ascii_lowercase();
    if matches!(level.as_str(), "error" | "warn" | "info" | "debug") {
        Ok(level)
    } else {
        Err("log_level must be one of error, warn, info, or debug".into())
    }
}

fn summarize_rust_log_level(value: &str) -> String {
    let mut service_levels = Vec::new();
    let mut global_levels = Vec::new();
    for directive in value.split(',').map(str::trim) {
        let (target, level) = directive
            .rsplit_once('=')
            .map_or((None, directive), |(target, level)| (Some(target), level));
        let level = match level.trim().to_ascii_lowercase().as_str() {
            "error" | "off" => Some((0, "error")),
            "warn" | "warning" => Some((1, "warn")),
            "info" => Some((2, "info")),
            "debug" | "trace" => Some((3, "debug")),
            _ => None,
        };
        if let Some(level) = level {
            if target.is_some_and(|target| target == "somaris" || target.starts_with("somaris::")) {
                service_levels.push(level);
            } else if target.is_none() {
                global_levels.push(level);
            }
        }
    }
    service_levels
        .into_iter()
        .max_by_key(|(rank, _)| *rank)
        .or_else(|| global_levels.into_iter().max_by_key(|(rank, _)| *rank))
        .map(|(_, level)| level.to_owned())
        .unwrap_or_else(|| "info".to_owned())
}

fn parse_allowed_origins(value: &str) -> Result<Vec<String>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    validate_origins(value.split(',').map(str::trim).map(str::to_owned).collect())
}

fn validate_origins(origins: Vec<String>) -> Result<Vec<String>, String> {
    origins
        .into_iter()
        .map(|origin| {
            if origin.is_empty() {
                return Err("allowed_origins cannot contain an empty entry".to_owned());
            }
            let url =
                Url::parse(&origin).map_err(|_| format!("Invalid allowed origin: {origin}"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(format!("Allowed origin must be an HTTP origin: {origin}"));
            }
            Ok(url.origin().ascii_serialization())
        })
        .collect()
}

pub fn process_environment() -> HashMap<String, String> {
    [
        "SOMARIS_SIMULATOR",
        "SOMARIS_ALLOWED_ORIGINS",
        "RUST_LOG",
        "SOMARIS_HISTORY_API_ENABLED",
        "SOMARIS_HISTORY_PATH",
        "SOMARIS_HISTORY_RETENTION",
        "SOMARIS_COMMAND_MAX_OUTSTANDING",
        "SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE",
        "SOMARIS_ADAPTER_CONCURRENCY",
        "SOMARIS_ADAPTER_SCAN_WINDOW_MS",
        "SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{SettingSource, SettingsDocument, SettingsUpdate, resolve};
    use std::collections::HashMap;

    #[test]
    fn built_in_defaults_have_default_source() {
        let settings = resolve(&SettingsDocument::default(), &HashMap::new()).unwrap();
        assert!(settings.values.simulator_enabled.value);
        assert_eq!(
            settings.values.simulator_enabled.source,
            SettingSource::Default
        );
        assert_eq!(settings.values.queue_max_outstanding.value, 1000);
        assert!(settings.values.adapter_concurrency.requires_restart);
    }

    #[test]
    fn environment_wins_over_file_and_marks_setting_locked() {
        let document = SettingsDocument {
            simulator_enabled: Some(false),
            ..SettingsDocument::default()
        };
        let environment = HashMap::from([("SOMARIS_SIMULATOR".into(), "true".into())]);
        let setting = resolve(&document, &environment)
            .unwrap()
            .values
            .simulator_enabled;
        assert!(setting.value);
        assert!(setting.locked);
        assert_eq!(setting.source, SettingSource::Environment);
    }

    #[test]
    fn environment_overrides_invalid_file_origins_and_log_level() {
        let document = SettingsDocument {
            allowed_origins: Some(vec!["file-value-is-not-an-origin".into()]),
            log_level: Some("verbose".into()),
            ..SettingsDocument::default()
        };
        let environment = HashMap::from([
            (
                "SOMARIS_ALLOWED_ORIGINS".into(),
                "https://dashboard.example".into(),
            ),
            ("RUST_LOG".into(), "warn,somaris::api=debug".into()),
        ]);

        let settings = resolve(&document, &environment).unwrap();

        assert_eq!(
            settings.values.allowed_origins.value,
            ["https://dashboard.example"]
        );
        assert!(settings.values.allowed_origins.locked);
        assert_eq!(settings.values.log_level.value, "debug");
        assert!(settings.values.log_level.locked);
    }

    #[test]
    fn file_values_apply_when_environment_is_unset_and_unknown_keys_survive() {
        let source = r#"{"version":1,"queue_max_outstanding":17,"queue_max_queued_per_device":10,"future_setting":"kept"}"#;
        let document: SettingsDocument = serde_json::from_str(source).unwrap();
        let settings = resolve(&document, &HashMap::new()).unwrap();
        assert_eq!(settings.values.queue_max_outstanding.value, 17);
        assert_eq!(
            settings.values.queue_max_outstanding.source,
            SettingSource::File
        );
        assert_eq!(document.unknown["future_setting"], "kept");
    }

    #[test]
    fn invalid_values_are_rejected_before_they_can_be_applied() {
        let document = SettingsDocument {
            adapter_scan_window_ms: Some(99),
            ..SettingsDocument::default()
        };
        assert!(resolve(&document, &HashMap::new()).is_err());
    }

    #[test]
    fn custom_environment_log_directives_remain_valid_and_locked() {
        let environment = HashMap::from([(
            "RUST_LOG".into(),
            "warn,somaris::api=debug,hyper=trace".into(),
        )]);
        let setting = resolve(&SettingsDocument::default(), &environment)
            .unwrap()
            .values
            .log_level;
        assert_eq!(setting.value, "debug");
        assert_eq!(setting.source, SettingSource::Environment);
        assert!(setting.locked);
    }

    #[test]
    fn updates_preserve_unknown_keys_and_reset_known_or_all_overrides() {
        let mut document: SettingsDocument = serde_json::from_str(
            r#"{"version":1,"simulator_enabled":false,"future_setting":{"enabled":true}}"#,
        )
        .unwrap();
        let changed = document.apply(SettingsUpdate {
            queue_max_outstanding: Some(24),
            ..SettingsUpdate::default()
        });
        assert_eq!(changed, ["queue_max_outstanding"]);
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(encoded["future_setting"]["enabled"], true);

        assert_eq!(
            document.reset(Some("simulator_enabled")).unwrap(),
            ["simulator_enabled"]
        );
        assert!(document.unknown.contains_key("future_setting"));
        document.reset(None).unwrap();
        assert!(document.unknown.is_empty());
        assert!(document.is_empty());
    }

    #[test]
    fn unsupported_settings_versions_are_rejected() {
        let document = SettingsDocument {
            version: 2,
            ..SettingsDocument::default()
        };
        assert!(resolve(&document, &HashMap::new()).is_err());
    }
}
