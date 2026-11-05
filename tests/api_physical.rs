use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
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
    api::{AppState, QueueConfig, router},
    ble::{BleAdapter, BleCommandControl, BleFailure, BleFailureStage, BleObservation},
    controller::ServiceAction,
};
use tokio::sync::{Notify, mpsc};
use tower::ServiceExt;

#[derive(Default)]
struct FakeBleAdapter {
    observations: Vec<BleObservation>,
    action_results: Mutex<VecDeque<Result<(), BleFailure>>>,
    actions: Mutex<Vec<(String, String, ServiceAction)>>,
    scan_active: AtomicBool,
    gatt_overlapped_scan: AtomicBool,
    completed_scans: AtomicUsize,
    scans_at_write: Mutex<Vec<usize>>,
    scan_finished: Notify,
    writes: Mutex<Vec<ServiceAction>>,
    gate: Mutex<Option<Arc<ActionGate>>>,
}

struct ActionGate {
    action: ServiceAction,
    after_write: bool,
    paused: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl ActionGate {
    fn new(action: ServiceAction, after_write: bool) -> Arc<Self> {
        Arc::new(Self {
            action,
            after_write,
            paused: AtomicBool::new(true),
            entered: Notify::new(),
            release: Notify::new(),
        })
    }
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
            scan_active: AtomicBool::default(),
            gatt_overlapped_scan: AtomicBool::default(),
            completed_scans: AtomicUsize::default(),
            scans_at_write: Mutex::default(),
            scan_finished: Notify::default(),
            writes: Mutex::default(),
            gate: Mutex::default(),
        }
    }

    fn with_gate(observations: Vec<BleObservation>, gate: Arc<ActionGate>) -> Self {
        let adapter = Self::with_observations(observations);
        *adapter.gate.lock().unwrap() = Some(gate);
        adapter
    }

    fn actions(&self) -> Vec<(String, String, ServiceAction)> {
        self.actions.lock().unwrap().clone()
    }

    fn gatt_overlapped_scan(&self) -> bool {
        self.gatt_overlapped_scan.load(Ordering::SeqCst)
    }

    fn scans_at_write(&self) -> Vec<usize> {
        self.scans_at_write.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<ServiceAction> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait]
impl BleAdapter for FakeBleAdapter {
    async fn scan(
        &self,
        observations: mpsc::Sender<BleObservation>,
        window: Duration,
    ) -> Result<(), BleFailure> {
        self.scan_active.store(true, Ordering::SeqCst);
        for observation in &self.observations {
            if observations.send(observation.clone()).await.is_err() {
                self.scan_active.store(false, Ordering::SeqCst);
                return Ok(());
            }
        }
        tokio::time::sleep(window).await;
        self.scan_active.store(false, Ordering::SeqCst);
        self.completed_scans.fetch_add(1, Ordering::SeqCst);
        self.scan_finished.notify_one();
        Ok(())
    }

    async fn write_action(
        &self,
        peripheral_id: &str,
        expected_name: &str,
        action: ServiceAction,
        control: &BleCommandControl,
    ) -> Result<(), BleFailure> {
        if self.scan_active.load(Ordering::SeqCst) {
            self.gatt_overlapped_scan.store(true, Ordering::SeqCst);
        }
        self.scans_at_write
            .lock()
            .unwrap()
            .push(self.completed_scans.load(Ordering::SeqCst));
        self.actions.lock().unwrap().push((
            peripheral_id.to_owned(),
            expected_name.to_owned(),
            action,
        ));
        let result = self
            .action_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(()));
        let gate = self.gate.lock().unwrap().clone();
        let should_pause = gate
            .as_ref()
            .is_some_and(|gate| gate.action == action && gate.paused.swap(false, Ordering::SeqCst));
        if should_pause && !gate.as_ref().unwrap().after_write {
            let gate = gate.as_ref().unwrap();
            gate.entered.notify_one();
            tokio::select! {
                biased;
                _ = control.cancelled() => return Err(control.cancellation_failure()),
                _ = gate.release.notified() => {}
            }
        }
        if result
            .as_ref()
            .is_err_and(|failure| !failure.write_may_have_been_sent)
        {
            return result;
        }
        if !control.begin_write() {
            return Err(BleFailure {
                stage: BleFailureStage::CommandPreempted,
                retryable_before_write: false,
                write_may_have_been_sent: false,
                outcome_unknown: false,
            });
        }
        self.writes.lock().unwrap().push(action);
        if should_pause && gate.as_ref().unwrap().after_write {
            let gate = gate.as_ref().unwrap();
            gate.entered.notify_one();
            tokio::select! {
                biased;
                _ = control.cancelled() => return Err(control.cancellation_failure()),
                _ = gate.release.notified() => {}
            }
        }
        result
    }

    async fn read_battery(
        &self,
        _peripheral_id: &str,
        _expected_name: &str,
    ) -> Result<Vec<u8>, BleFailure> {
        Ok(vec![0x8e, 0x01])
    }
}

#[tokio::test(start_paused = true)]
async fn enrolled_blind_battery_is_polled_and_reported() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let app = app_with_adapter(adapter).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    enroll(&app, &blind).await;
    tokio::time::advance(Duration::from_secs(61)).await;
    let battery = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let (_, response) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
            let devices = response.unwrap();
            let battery = &devices["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|device| device["advertised_name"] == "Bedroom Right0")
                .unwrap()["battery"];
            if battery["status"] == "observed" {
                break battery.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("battery reading should be reported");
    assert_eq!(battery["voltage_mv"], 3980);
    assert_eq!(battery["percent_estimate"], 76);
    assert_eq!(battery["source"], "gatt_ba71");
}

#[tokio::test]
async fn scanned_blind_can_be_enrolled_and_only_fixed_actions_are_written() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let app = app_with_adapter(adapter.clone()).await;

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
    let (_, managed) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let managed = managed.unwrap()["data"][0].clone();
    assert_eq!(managed["availability"], "ready");
    assert_eq!(managed["gatt_readiness"]["status"], "untested");

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
        assert_eq!(command["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(command["attempts"][0]["phase"], "gatt_write_acknowledged");
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
    let (_, managed) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let gatt = &managed.unwrap()["data"][0]["gatt_readiness"];
    assert_eq!(gatt["status"], "write_acknowledged");
    assert_eq!(gatt["attempt_count"], 1);
    assert!(gatt["last_write_acknowledged_at"].is_string());
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
    assert_eq!(command["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(command["attempts"][0]["phase"], "service_resolution");
    assert_eq!(
        command["attempts"][0]["error_code"],
        "service_resolution_failed"
    );
    assert_eq!(command["attempts"][0]["write_state"], "not_sent");
    assert_eq!(command["attempts"][1]["phase"], "gatt_write_acknowledged");
    assert!(command["attempts"][0]["duration_ms"].is_number());
    assert!(command["attempts"][1]["duration_ms"].is_number());
}

#[tokio::test]
async fn gatt_readiness_keeps_last_ack_after_a_later_connection_failure() {
    let unavailable = BleFailure {
        stage: BleFailureStage::DeviceConnection,
        retryable_before_write: false,
        write_may_have_been_sent: false,
        outcome_unknown: false,
    };
    let adapter = Arc::new(FakeBleAdapter::with_results(
        vec![closed_blind()],
        [Ok(()), Err(unavailable)],
    ));
    let app = app_with_adapter(adapter).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    assert_eq!(
        submit(&app, &managed_id, "first-ack", "close").await["status"],
        "write_acknowledged"
    );
    let failed = submit(&app, &managed_id, "later-connect-failure", "open").await;
    assert_eq!(failed["status"], "connection_failed");

    let (_, response) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let gatt = &response.unwrap()["data"][0]["gatt_readiness"];
    assert_eq!(gatt["status"], "connection_failed");
    assert_eq!(gatt["failure_stage"], "device_connection");
    assert_eq!(gatt["error_code"], "device_connection_failed");
    assert_eq!(gatt["attempt_count"], 1);
    assert!(gatt["last_write_acknowledged_at"].is_string());
    let (ready_status, readiness) = request(&app, Method::GET, "/api/v1/ready", &[], None).await;
    assert_eq!(ready_status, StatusCode::OK);
    assert_eq!(readiness.unwrap()["data"]["status"], "healthy");
}

#[tokio::test]
async fn stop_preempts_an_in_flight_movement_before_the_gatt_write() {
    let gate = ActionGate::new(ServiceAction::Close, false);
    let adapter = Arc::new(FakeBleAdapter::with_gate(
        vec![closed_blind()],
        gate.clone(),
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "preemptible-close")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let close_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("movement should reach the pre-write gate");

    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "priority-stop")],
        Some(json!({"action": "stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let stop_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let close = wait_for_command(&app, &close_id).await;
    let stop = wait_for_command(&app, &stop_id).await;

    assert_eq!(close["status"], "cancelled");
    assert_eq!(close["error_code"], "preempted_by_stop");
    assert_eq!(close["write_state"], "not_sent");
    assert_eq!(stop["status"], "write_acknowledged");
    assert_eq!(adapter.writes(), [ServiceAction::Stop]);
}

#[tokio::test]
async fn stop_waits_for_a_movement_once_its_gatt_write_has_started() {
    let gate = ActionGate::new(ServiceAction::Close, true);
    let adapter = Arc::new(FakeBleAdapter::with_gate(
        vec![closed_blind()],
        gate.clone(),
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "started-close")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let close_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("movement should reach the post-write gate");
    let (_, in_flight) = request(
        &app,
        Method::GET,
        &format!("/api/control/v1/commands/{close_id}"),
        &[],
        None,
    )
    .await;
    let in_flight = in_flight.unwrap()["data"].clone();
    assert_eq!(in_flight["write_state"], "possibly_sent");
    assert_eq!(in_flight["attempts"][0]["write_state"], "possibly_sent");

    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "serialized-stop")],
        Some(json!({"action": "stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let stop_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(adapter.writes(), [ServiceAction::Close]);
    gate.release.notify_one();

    assert_eq!(
        wait_for_command(&app, &close_id).await["status"],
        "write_acknowledged"
    );
    assert_eq!(
        wait_for_command(&app, &stop_id).await["status"],
        "write_acknowledged"
    );
    assert_eq!(
        adapter.writes(),
        [ServiceAction::Close, ServiceAction::Stop]
    );
}

#[tokio::test(start_paused = true)]
async fn command_deadline_is_bounded_and_retryable_before_a_write() {
    let gate = ActionGate::new(ServiceAction::Open, false);
    let adapter = Arc::new(FakeBleAdapter::with_gate(
        vec![closed_blind()],
        gate.clone(),
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "deadline-before-write")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("command should reach the pre-write gate");
    tokio::time::advance(Duration::from_secs(46)).await;

    let command = wait_for_command(&app, &command_id).await;
    assert_eq!(command["status"], "connection_failed");
    assert_eq!(command["error_code"], "command_timeout");
    assert_eq!(command["write_state"], "not_sent");
    assert_eq!(command["retryable"], true);
    assert_eq!(command["attempt"], 1);
    assert_eq!(command["attempts"][0]["phase"], "command_deadline");
    assert!(adapter.writes().is_empty());
}

#[tokio::test(start_paused = true)]
async fn command_deadline_after_write_start_is_reported_as_uncertain() {
    let gate = ActionGate::new(ServiceAction::Open, true);
    let adapter = Arc::new(FakeBleAdapter::with_gate(
        vec![closed_blind()],
        gate.clone(),
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let (status, response) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "deadline-after-write")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("command should reach the post-write gate");
    tokio::time::advance(Duration::from_secs(46)).await;

    let command = wait_for_command(&app, &command_id).await;
    assert_eq!(command["status"], "outcome_unknown");
    assert_eq!(command["error_code"], "outcome_unknown");
    assert_eq!(command["write_state"], "possibly_sent");
    assert_eq!(command["retryable"], false);
    assert_eq!(command["attempts"][0]["write_state"], "possibly_sent");
    assert_eq!(adapter.writes(), [ServiceAction::Open]);
}

#[tokio::test]
async fn retry_is_refused_when_write_may_have_been_sent() {
    let ambiguous_prewrite_flag = BleFailure {
        stage: BleFailureStage::ServiceResolution,
        retryable_before_write: true,
        write_may_have_been_sent: true,
        outcome_unknown: true,
    };
    let adapter = Arc::new(FakeBleAdapter::with_results(
        vec![closed_blind()],
        [Err(ambiguous_prewrite_flag)],
    ));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let command = submit(&app, &managed_id, "no-ambiguous-retry", "close").await;

    assert_eq!(command["attempt"], 1);
    assert_eq!(command["status"], "outcome_unknown");
    assert_eq!(adapter.actions().len(), 1);
}

#[tokio::test]
async fn gatt_commands_wait_until_the_adapter_scan_window_ends() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let app = app_with_adapter(adapter.clone()).await;
    let blind = wait_for_blind(&app, "Bedroom Right0").await;
    let managed_id = enroll(&app, &blind).await;
    let command = submit(&app, &managed_id, "serialized-gatt", "close").await;

    assert_eq!(command["status"], "write_acknowledged");
    assert!(!adapter.gatt_overlapped_scan());
    let scans_at_write = adapter.scans_at_write();
    assert_eq!(scans_at_write.len(), 1, "the command issued one GATT write");
    assert!(
        scans_at_write.iter().all(|completed| *completed > 0),
        "the GATT write must wait until a scan window has ended"
    );
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
    let app = app_with_adapter(adapter.clone()).await;

    let names = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (_, response) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
            let mut names: Vec<String> = response.unwrap()["data"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|device| device["source"] == "ble_advertisement")
                .map(|device| device["advertised_name"].as_str().unwrap().to_owned())
                .collect();
            if names.len() == 2 {
                names.sort();
                break names;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("scanner should publish both observed blinds");
    assert_eq!(names, ["Bedroom Right0", "Hallway Blinds0"]);
}

#[tokio::test]
async fn scanner_yield_keeps_api_ready_while_adapter_is_available() {
    let adapter = Arc::new(FakeBleAdapter::with_observations(vec![closed_blind()]));
    let app = app_with_adapter(adapter.clone()).await;
    let _ = wait_for_blind(&app, "Bedroom Right0").await;

    let scanner = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            adapter.scan_finished.notified().await;
            let (_, diagnostics) =
                request(&app, Method::GET, "/api/v1/diagnostics", &[], None).await;
            let scanner = diagnostics.unwrap()["data"]["scanner"].clone();
            if scanner["adapter_available"] == true && scanner["scan_active"] == false {
                break scanner;
            }
        }
    })
    .await
    .expect("scanner should expose an adapter yield between bounded scan windows");
    assert_eq!(scanner["retry_count"], 0);
    assert_eq!(scanner["error_count"], 0);
    let (status, readiness) = request(&app, Method::GET, "/api/v1/ready", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(readiness.unwrap()["data"]["status"], "healthy");
}

async fn app_with_adapter(adapter: Arc<FakeBleAdapter>) -> Router {
    let state = AppState::new()
        .with_ble_adapter(adapter)
        .with_queue_config(QueueConfig {
            adapter_scan_window_ms: 100,
            ..QueueConfig::default()
        })
        .unwrap();
    state.spawn_background_tasks();
    router(state)
}

async fn wait_for_blind(app: &Router, name: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (_, response) = request(app, Method::GET, "/api/v1/devices", &[], None).await;
            if let Some(blind) = response.unwrap()["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|device| device["advertised_name"] == name)
            {
                break blind.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("scanner did not publish {name} within two seconds"))
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
    let mut last_command = Value::Null;
    for _ in 0..2_000 {
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
        last_command = command;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("command {command_id} did not reach a terminal state: {last_command}");
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
