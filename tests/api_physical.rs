use std::{
    collections::VecDeque,
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::{
    api::{AppState, router},
    ble::{BleAdapter, BleFailure, BleFailureStage, BleObservation},
    controller::ServiceAction,
};
use tokio::sync::mpsc;
use tower::ServiceExt;

#[derive(Default)]
struct FakeBleAdapter {
    observations: Vec<BleObservation>,
    action_results: Mutex<VecDeque<Result<(), BleFailure>>>,
    actions: Mutex<Vec<(String, String, ServiceAction)>>,
}

impl FakeBleAdapter {
    fn with_observations(observations: Vec<BleObservation>) -> Self {
        Self {
            observations,
            ..Self::default()
        }
    }

    fn with_results(
        observations: Vec<BleObservation>,
        results: impl IntoIterator<Item = Result<(), BleFailure>>,
    ) -> Self {
        Self {
            observations,
            action_results: Mutex::new(results.into_iter().collect()),
            actions: Mutex::default(),
        }
    }

    fn actions(&self) -> Vec<(String, String, ServiceAction)> {
        self.actions.lock().unwrap().clone()
    }
}

#[async_trait]
impl BleAdapter for FakeBleAdapter {
    async fn scan(&self, observations: mpsc::Sender<BleObservation>) -> Result<(), BleFailure> {
        for observation in &self.observations {
            observations
                .send(observation.clone())
                .await
                .expect("scanner should keep receiving observations");
        }
        pending::<()>().await;
        Ok(())
    }

    async fn write_action(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
    ) -> Result<(), BleFailure> {
        self.actions.lock().unwrap().push((
            peripheral_id.to_owned(),
            expected_name.to_owned(),
            action,
        ));
        self.action_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(()))
    }

    async fn read_battery(
        &self,
        _peripheral_id: &str,
        _expected_name: &str,
    ) -> Result<Vec<u8>, BleFailure> {
        Ok(vec![0x8e, 0x01])
    }
}

#[tokio::test]
async fn scanned_blind_can_be_enrolled_and_only_fixed_actions_are_written() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let state = AppState::new().with_ble_adapter(adapter.clone());
    state.spawn_background_tasks();
    let app = router(state);

    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    assert_eq!(blind["source"], "ble_advertisement");
    assert_eq!(
        blind["manufacturer_data_hex"],
        "7003055c6464426564726f6f6d2052696768743020"
    );
    assert!(blind.get("adapter_device_id").is_none());
    assert!(blind.get("observed_address").is_none());
    assert_eq!(blind["position"]["current_closed_percent"], 100);
    assert_eq!(blind["battery"]["status"], "not_observed");

    let (status, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({
            "observed_device_id": blind["id"],
            "label": "Bedroom right"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let managed_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    for (key, action, expected) in [
        ("physical-open", "open", ServiceAction::Open),
        ("physical-close", "close", ServiceAction::Close),
        ("physical-stop", "stop", ServiceAction::Stop),
    ] {
        let (status, response) = request(
            &app,
            Method::POST,
            &format!("/api/control/v1/devices/{managed_id}/commands"),
            &[("idempotency-key", key)],
            Some(json!({"action": action})),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
        let command = wait_for_command(&app, &command_id).await;
        assert_eq!(command["status"], "write_acknowledged");
        assert_eq!(command["write_state"], "acknowledged");
        assert_eq!(command["retryable"], false);
        assert_eq!(command["attempt"], 1);
        assert_eq!(adapter.actions().last().unwrap().2, expected);
    }

    let (_, after_commands) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let blind_after_commands = after_commands.unwrap()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["advertised_name"] == "Bedroom Right0")
        .unwrap()
        .clone();
    assert_eq!(
        blind_after_commands["position"]["current_closed_percent"],
        100
    );
    assert_eq!(adapter.actions().len(), 3);
}

#[tokio::test]
async fn transient_service_discovery_failure_retries_only_before_a_write() {
    let retryable = BleFailure {
        stage: BleFailureStage::ServiceResolution,
        retryable_before_write: true,
        write_may_have_been_sent: false,
        outcome_unknown: false,
    };
    let adapter = Arc::new(FakeBleAdapter::with_results(
        vec![closed_blind()],
        [Err(retryable), Ok(())],
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let command = submit(&app, &managed_id, "retry-safe", "close").await;

    assert_eq!(command["status"], "write_acknowledged");
    assert_eq!(command["attempt"], 2);
    assert_eq!(command["retryable"], false);
    assert_eq!(adapter.actions().len(), 2);
}

#[tokio::test]
async fn physical_toggle_uses_fresh_closed_position_and_enqueues_open() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/toggle"),
        &[("idempotency-key", "physical-toggle")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.as_ref().unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(response.as_ref().unwrap()["data"]["action"], "open");
    assert_eq!(
        response.as_ref().unwrap()["data"]["toggle_basis"],
        "observed_target"
    );
    assert_eq!(
        response.as_ref().unwrap()["data"]["toggle_position"]["current_closed_percent"],
        100
    );

    let command = wait_for_command(&app, &command_id).await;
    assert_eq!(command["status"], "write_acknowledged");
    assert_eq!(adapter.actions().last().unwrap().2, ServiceAction::Open);
}

#[tokio::test]
async fn uncertain_gatt_write_is_reported_without_automatic_retry() {
    let uncertain = BleFailure {
        stage: BleFailureStage::GattWrite,
        retryable_before_write: false,
        write_may_have_been_sent: true,
        outcome_unknown: true,
    };
    let adapter = Arc::new(FakeBleAdapter::with_results(
        vec![closed_blind()],
        [Err(uncertain)],
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let command = submit(&app, &managed_id, "uncertain-write", "open").await;

    assert_eq!(command["status"], "outcome_unknown");
    assert_eq!(command["write_state"], "possibly_sent");
    assert_eq!(command["retryable"], false);
    assert_eq!(command["attempt"], 1);
    assert_eq!(adapter.actions().len(), 1);
}

#[tokio::test]
async fn scanner_keeps_blinds_separate_by_opaque_adapter_identifier() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![
        blind("Bedroom Right0", "corebluetooth-id-bedroom"),
        blind("Hallway Blinds0", "corebluetooth-id-hallway"),
    ]));
    let app = app_with_adapter(adapter).await;

    let mut names = Vec::new();
    for _ in 0..100 {
        let (_, response) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
        names = response.unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|device| device["source"] == "ble_advertisement")
            .map(|device| device["advertised_name"].as_str().unwrap().to_owned())
            .collect();
        if names.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    names.sort();
    assert_eq!(names, ["Bedroom Right0", "Hallway Blinds0"]);
}

async fn app_with_adapter(adapter: Arc<FakeBleAdapter>) -> Router {
    let state = AppState::new().with_ble_adapter(adapter);
    state.spawn_background_tasks();
    router(state)
}

async fn wait_for_blind(app: &Router, name: &str) -> Value {
    for _ in 0..100 {
        let (_, response) = request(app, Method::GET, "/api/v1/devices", &[], None).await;
        if let Some(blind) = response.unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["advertised_name"] == name)
        {
            return blind.clone();
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("scanner did not publish {name}");
}

async fn enroll(app: &Router, blind: &Value) -> String {
    let (status, response) = request(
        app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({
            "observed_device_id": blind["id"],
            "label": "Test blind"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    response.unwrap()["data"]["id"].as_str().unwrap().to_owned()
}

async fn submit(app: &Router, managed_id: &str, key: &str, action: &str) -> Value {
    let (status, response) = request(
        app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", key)],
        Some(json!({"action": action})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    wait_for_command(app, &command_id).await
}

async fn wait_for_command(app: &Router, command_id: &str) -> Value {
    for _ in 0..200 {
        let (_, response) = request(
            app,
            Method::GET,
            &format!("/api/control/v1/commands/{command_id}"),
            &[],
            None,
        )
        .await;
        let command = response.unwrap()["data"].clone();
        if !matches!(command["status"].as_str(), Some("queued" | "connecting")) {
            return command;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("command did not reach a terminal state");
}

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Option<Value>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let payload = body.map(|value| value.to_string()).unwrap_or_default();
    if !payload.is_empty() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(payload)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = (!bytes.is_empty()).then(|| serde_json::from_slice(&bytes).unwrap());
    (status, body)
}

fn closed_blind() -> BleObservation {
    blind("Bedroom Right0", "AA:BB:CC:DD:EE:01")
}

fn blind(name: &str, peripheral_id: &str) -> BleObservation {
    let mut manufacturer_payload = vec![0x05, 0x5c, 0x64, 0x64];
    manufacturer_payload.extend_from_slice(name.as_bytes());
    manufacturer_payload.push(b' ');
    BleObservation {
        peripheral_id: peripheral_id.into(),
        observed_address: None,
        device_name: Some("SOMA blind".into()),
        rssi_dbm: -56,
        manufacturer_payload,
    }
}
