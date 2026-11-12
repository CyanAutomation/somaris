use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::api::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn enrollment_emits_internal_state_change_notification() {
    let state = AppState::new();
    let mut changes = state.subscribe_state_changes();
    let app = router(state);
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({
            "observed_device_id": "simulator-1",
            "label": "Bedroom"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(changes.recv().await.unwrap().revision >= 1);
}

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Option<Value>) {
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
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("json"));
    let value = if bytes.is_empty() || !is_json {
        None
    } else {
        Some(serde_json::from_slice(&bytes).unwrap())
    };
    (status, headers, value)
}

async fn wait_for_command(app: &Router, command_id: &str) -> Value {
    for _ in 0..200 {
        let (_, _, body) = request(
            app,
            Method::GET,
            &format!("/api/control/v1/commands/{command_id}"),
            &[],
            None,
        )
        .await;
        let command = body.unwrap()["data"].clone();
        if !matches!(command["status"].as_str(), Some("queued" | "connecting")) {
            return command;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("command did not reach a terminal state");
}

async fn wait_for_batch(app: &Router, batch_id: &str) -> Value {
    for _ in 0..1000 {
        let (_, _, body) = request(
            app,
            Method::GET,
            &format!("/api/control/v1/batches/{batch_id}"),
            &[],
            None,
        )
        .await;
        let batch = body.unwrap()["data"].clone();
        if !matches!(batch["status"].as_str(), Some("queued" | "running")) {
            return batch;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("batch did not reach a terminal state");
}

#[tokio::test]
async fn health_reports_liveness_while_readiness_tracks_scanner_availability() {
    let app = router(AppState::new());
    let (status, _, body) = request(&app, Method::GET, "/api/v1/health", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.unwrap()["data"]["status"], "degraded");

    let (status, _, body) = request(&app, Method::GET, "/api/v1/ready", &[], None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.unwrap()["data"]["scanner_status"], "unavailable");
}

#[tokio::test]
async fn device_list_uses_closed_percent_and_supports_conditional_reads() {
    let app = router(AppState::new());
    let (status, headers, body) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    assert_eq!(body["meta"]["count"], 2);
    let names: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|device| device["advertised_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Somaris Simulator 1", "Somaris Simulator 2"]);
    let simulator = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["id"] == "simulator-1")
        .unwrap();
    assert_eq!(simulator["is_simulated"], true);
    assert_eq!(simulator["position"]["current_closed_percent"], 100);
    let payload = hex::decode(simulator["manufacturer_payload_hex"].as_str().unwrap()).unwrap();
    let advertisement = somaris::ble::parse_soma_payload(&payload).unwrap();
    assert_eq!(advertisement.advertised_name, "Somaris Simulator 1");
    assert_eq!(advertisement.current_closed_percent, 100);
    assert_eq!(advertisement.target_closed_percent, 100);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();

    let (status, _, _) = request(
        &app,
        Method::GET,
        "/api/v1/devices",
        &[("if-none-match", etag)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn position_commands_validate_targets_and_preserve_idempotency() {
    let app = router(AppState::new());
    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": simulator_id, "label": "Position test"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let enrollment_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, _, accepted) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[("idempotency-key", "position-half")],
        Some(json!({"action": "set_position", "position_closed_percent": 50})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command = accepted.unwrap()["data"].clone();
    assert_eq!(command["action"], "set_position");
    assert_eq!(command["target_closed_percent"], 50);
    let command_id = command["id"].as_str().unwrap();
    assert_eq!(
        wait_for_command(&app, command_id).await["status"],
        "write_acknowledged"
    );

    let (status, _, replay) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[("idempotency-key", "position-half")],
        Some(json!({"action": "set_position", "position_closed_percent": 50})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(replay.unwrap()["data"]["id"], command_id);

    let (status, _, conflict) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[("idempotency-key", "position-half")],
        Some(json!({"action": "set_position", "position_closed_percent": 51})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict.unwrap()["error"]["code"], "idempotency_conflict");

    for (key, body, expected_code) in [
        (
            "position-too-high",
            json!({"action": "set_position", "position_closed_percent": 101}),
            "invalid_position",
        ),
        (
            "position-missing",
            json!({"action": "set_position"}),
            "position_required",
        ),
        (
            "position-unexpected",
            json!({"action": "open", "position_closed_percent": 50}),
            "unexpected_position",
        ),
    ] {
        let (status, _, error) = request(
            &app,
            Method::POST,
            &format!("/api/control/v1/devices/{enrollment_id}/commands"),
            &[("idempotency-key", key)],
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.unwrap()["error"]["code"], expected_code);
    }

    let (_, _, managed) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let managed = managed.unwrap()["data"].clone();
    let device = managed
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == enrollment_id)
        .unwrap();
    assert_eq!(device["capabilities"]["set_position"], true);
    assert_eq!(device["capabilities"]["set_motor_speed"], true);
    assert_eq!(device["capabilities"]["calibrate_endpoints"], true);
    assert_eq!(device["position"]["target_closed_percent"], 50);
    assert_eq!(device["requested_position_closed_percent"], 50);
}

#[tokio::test]
async fn simulators_have_distinct_names_independent_settings_and_positions() {
    let app = router(AppState::new());

    let (status, _, listed) = request(&app, Method::GET, "/api/dev/simulators", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = listed.unwrap()["data"].clone();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[0]["id"], "simulator-1");
    assert_eq!(listed[0]["advertised_name"], "Somaris Simulator 1");
    assert_eq!(listed[1]["id"], "simulator-2");
    assert_eq!(listed[1]["advertised_name"], "Somaris Simulator 2");

    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/dev/simulators/simulator-1/reset",
        &[],
        Some(json!({"position_closed_percent": 25})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/dev/simulators/simulator-2/reset",
        &[],
        Some(json!({"position_closed_percent": 65})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, scenario) = request(
        &app,
        Method::PUT,
        "/api/dev/simulators/simulator-2/scenario",
        &[],
        Some(json!({"wireless_profile": "standard"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scenario.unwrap()["data"]["wireless_profile"], "standard");

    let (status, _, listed) = request(&app, Method::GET, "/api/dev/simulators", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = listed.unwrap()["data"].clone();
    assert_eq!(listed[0]["current_closed_percent"], 25);
    assert_eq!(listed[0]["wireless_profile"], "hallway_observed");
    assert_eq!(listed[1]["current_closed_percent"], 65);
    assert_eq!(listed[1]["wireless_profile"], "standard");

    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let devices = devices.unwrap()["data"].clone();
    for id in ["simulator-1", "simulator-2"] {
        let device = devices
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["id"] == id)
            .unwrap();
        let (status, _, enrolled) = request(
            &app,
            Method::POST,
            "/api/control/v1/devices",
            &[],
            Some(json!({
                "observed_device_id": id,
                "label": device["advertised_name"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(enrolled.unwrap()["data"]["is_simulated"], true);
    }

    let (status, _, managed) =
        request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let managed = managed.unwrap()["data"].clone();
    assert_eq!(managed.as_array().unwrap().len(), 2);
    assert_eq!(managed[0]["capabilities"]["open"], true);
    assert_eq!(managed[0]["capabilities"]["close"], true);
    assert_eq!(managed[0]["capabilities"]["stop"], true);
    assert_eq!(managed[0]["capabilities"]["set_position"], true);
    assert_eq!(managed[0]["capabilities"]["position_feedback"], true);
    assert_eq!(managed[0]["position_freshness"], "fresh");
    assert_eq!(managed[0]["position_source"], "simulator");
    assert!(managed[0]["position_observed_at"].is_string());
    let first_id = managed[0]["enrollment"]["id"].as_str().unwrap();
    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{first_id}/commands"),
        &[("idempotency-key", "open-simulator-1")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &command_id).await["status"],
        "write_acknowledged"
    );
    let (_, _, reconciled) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let reconciled = reconciled.unwrap()["data"].clone();
    let first = reconciled
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == first_id)
        .unwrap();
    assert_eq!(first["requested_position_closed_percent"], 0);
    assert_eq!(first["requested_command_status"], "write_acknowledged");
    assert_eq!(first["position"]["target_closed_percent"], 0);

    let (status, _, stop) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{first_id}/commands"),
        &[("idempotency-key", "stop-after-open-simulator-1")],
        Some(json!({"action": "stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let stop_id = stop.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &stop_id).await["status"],
        "write_acknowledged"
    );
    let (_, _, stopped) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let stopped = stopped.unwrap()["data"].clone();
    let first = stopped
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == first_id)
        .unwrap();
    assert_eq!(first["requested_position_closed_percent"], 0);
    assert_eq!(first["requested_command_status"], "write_acknowledged");

    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let devices = devices.unwrap()["data"].clone();
    let first = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["id"] == "simulator-1")
        .unwrap();
    let second = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["id"] == "simulator-2")
        .unwrap();
    assert_eq!(first["position"]["target_closed_percent"], 25);
    assert_eq!(second["position"]["current_closed_percent"], 65);
    assert_eq!(second["position"]["target_closed_percent"], 65);
}

#[tokio::test]
async fn motor_speed_and_endpoint_calibration_are_typed_and_sequenced() {
    let app = router(AppState::new());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator = devices.unwrap()["data"][0].clone();
    let (_, _, enrolled) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({
            "observed_device_id": simulator["id"],
            "label": simulator["advertised_name"]
        })),
    )
    .await;
    let enrollment_id = enrolled.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command_url = format!("/api/control/v1/devices/{enrollment_id}/commands");

    for (key, speed, expected) in [("speed-high", 101, "invalid_motor_speed")] {
        let (status, _, error) = request(
            &app,
            Method::POST,
            &command_url,
            &[("idempotency-key", key)],
            Some(json!({"action": "set_motor_speed", "motor_speed_percent": speed})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.unwrap()["error"]["code"], expected);
    }

    let (status, _, response) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "speed-37")],
        Some(json!({"action": "set_motor_speed", "motor_speed_percent": 37})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command = response.unwrap()["data"].clone();
    assert_eq!(command["motor_speed_percent"], 37);
    assert_eq!(
        wait_for_command(&app, command["id"].as_str().unwrap()).await["status"],
        "write_acknowledged"
    );

    let (status, _, response) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "speed-zero")],
        Some(json!({"action": "set_motor_speed", "motor_speed_percent": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command = response.unwrap()["data"].clone();
    assert_eq!(command["motor_speed_percent"], 0);
    assert_eq!(
        wait_for_command(&app, command["id"].as_str().unwrap()).await["status"],
        "write_acknowledged"
    );

    let (status, _, before_start) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "calibration-step-before-start")],
        Some(json!({"action": "calibration_nudge_open"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        before_start.unwrap()["error"]["code"],
        "calibration_not_active"
    );

    let (status, _, stop_before_start) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "calibration-stop-before-start")],
        Some(json!({"action": "calibration_stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        stop_before_start.unwrap()["error"]["code"],
        "calibration_not_active"
    );

    let (status, _, response) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "calibration-start")],
        Some(json!({"action": "calibration_start"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &command_id).await["status"],
        "write_acknowledged"
    );

    let (_, _, managed) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let managed = managed.unwrap()["data"].clone();
    let device = managed
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == enrollment_id)
        .unwrap();
    assert_eq!(device["calibration_mode_active"], true);

    let (status, _, remove_error) = request(
        &app,
        Method::DELETE,
        &format!("/api/control/v1/devices/{enrollment_id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(remove_error.unwrap()["error"]["code"], "calibration_active");

    let (status, _, batch_error) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        &[("idempotency-key", "calibration-batch-close")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(batch_error.unwrap()["error"]["code"], "no_available_blinds");

    let (status, _, blocked_move) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "position-during-calibration")],
        Some(json!({"action": "set_position", "position_closed_percent": 45})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(blocked_move.unwrap()["error"]["code"], "calibration_active");

    for (key, action) in [
        ("calibration-nudge-open", "calibration_nudge_open"),
        ("calibration-mark-open", "calibration_set_open_limit"),
        ("calibration-nudge-closed", "calibration_nudge_closed"),
        ("calibration-mark-closed", "calibration_set_closed_limit"),
    ] {
        let (status, _, response) = request(
            &app,
            Method::POST,
            &command_url,
            &[("idempotency-key", key)],
            Some(json!({"action": action})),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(
            wait_for_command(&app, &command_id).await["status"],
            "write_acknowledged"
        );
    }

    let (status, _, response) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "calibration-stop")],
        Some(json!({"action": "calibration_stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &command_id).await["status"],
        "write_acknowledged"
    );
    let (_, _, managed) = request(&app, Method::GET, "/api/control/v1/devices", &[], None).await;
    let managed = managed.unwrap()["data"].clone();
    let device = managed
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == enrollment_id)
        .unwrap();
    assert_eq!(device["calibration_mode_active"], false);
}

#[tokio::test]
async fn calibration_lock_survives_service_restart_until_explicit_stop() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, enrolled) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": simulator_id, "label": "Calibration persistence"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let managed_id = enrolled.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command_url = format!("/api/control/v1/devices/{managed_id}/commands");
    let (status, _, response) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "persist-calibration-start")],
        Some(json!({"action": "calibration_start"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &command_id).await["status"],
        "write_acknowledged"
    );
    drop(app);

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, managed) =
        request(&restored, Method::GET, "/api/control/v1/devices", &[], None).await;
    let device = managed.unwrap()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == managed_id)
        .unwrap()
        .clone();
    assert_eq!(device["calibration_mode_active"], true);
    let (status, _, blocked) = request(
        &restored,
        Method::POST,
        &command_url,
        &[("idempotency-key", "persist-calibration-blocked-move")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(blocked.unwrap()["error"]["code"], "calibration_active");

    let (status, _, response) = request(
        &restored,
        Method::POST,
        &command_url,
        &[("idempotency-key", "persist-calibration-stop")],
        Some(json!({"action": "calibration_stop"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = response.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&restored, &command_id).await["status"],
        "write_acknowledged"
    );
    let (_, _, managed) =
        request(&restored, Method::GET, "/api/control/v1/devices", &[], None).await;
    let device = managed.unwrap()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == managed_id)
        .unwrap()
        .clone();
    assert_eq!(device["calibration_mode_active"], false);
}

#[tokio::test]
async fn commands_are_idempotent_and_acknowledgement_does_not_claim_observed_movement() {
    let app = router(AppState::new());
    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let observed_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": observed_id, "label": "Office blind"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let managed_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "open-1")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command = command.unwrap()["data"].clone();
    let command_id = command["id"].as_str().unwrap().to_owned();

    let (status, _, repeated) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "open-1")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(repeated.unwrap()["data"]["id"], command_id);

    let (status, _, conflict) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "open-1")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict.unwrap()["error"]["code"], "idempotency_conflict");

    let final_command = wait_for_command(&app, &command_id).await;
    assert_eq!(final_command["status"], "write_acknowledged");
    assert_eq!(final_command["write_state"], "acknowledged");

    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let position = &devices.unwrap()["data"][0]["position"];
    assert_eq!(position["target_closed_percent"], 0);
    assert_eq!(position["current_closed_percent"], 100);
}

#[tokio::test]
async fn uncertain_simulator_outcomes_are_never_safe_to_retry() {
    let app = router(AppState::new());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let observed_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": observed_id, "label": "Test blind"})),
    )
    .await;
    let managed_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({"scenario": "outcome_unknown"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "uncertain-1")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command = wait_for_command(&app, &command_id).await;
    assert_eq!(command["status"], "outcome_unknown");
    assert_eq!(command["write_state"], "possibly_sent");
    assert_eq!(command["retryable"], false);
    let (_, _, activity) = request(&app, Method::GET, "/api/control/v1/audit-log", &[], None).await;
    assert_eq!(activity.unwrap()["data"][0]["status"], "outcome_unknown");
}

#[tokio::test]
async fn history_is_explicitly_unavailable_when_not_configured() {
    let app = router(AppState::new());
    let (status, _, body) = request(&app, Method::GET, "/api/v1/history", &[], None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.unwrap()["error"]["code"], "history_disabled");
}

#[tokio::test]
async fn recent_audit_log_is_available_by_default_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("nested/state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());

    let (status, headers, empty) =
        request(&app, Method::GET, "/api/control/v1/audit-log", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty.unwrap()["data"], json!([]));
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, _, _) = request(
        &app,
        Method::GET,
        "/api/control/v1/audit-log",
        &[("if-none-match", etag)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);

    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, enrolled) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": simulator_id, "label": "Study blind"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let enrollment_id = enrolled.unwrap()["data"]["id"].as_str().unwrap().to_owned();

    let (_, _, accepted) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[("idempotency-key", "do-not-store-this-key")],
        Some(json!({"action": "open"})),
    )
    .await;
    let command_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        wait_for_command(&app, &command_id).await["status"],
        "write_acknowledged"
    );

    let (status, _, _) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[("idempotency-key", "do-not-store-this-key")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, activity) =
        request(&app, Method::GET, "/api/control/v1/audit-log", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let activity = activity.unwrap()["data"].as_array().unwrap().clone();
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0]["kind"], "command");
    assert_eq!(activity[0]["action"], "open");
    assert_eq!(activity[0]["status"], "write_acknowledged");
    assert_eq!(activity[0]["subject"], "Study blind");
    assert_eq!(activity[1]["kind"], "enrollment_added");
    assert_eq!(activity[1]["subject"], "Study blind");
    assert!(
        !serde_json::to_string(&activity)
            .unwrap()
            .contains("do-not-store-this-key")
    );

    drop(app);
    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (status, _, restored_activity) = request(
        &restored,
        Method::GET,
        "/api/control/v1/audit-log",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restored_activity.unwrap()["data"], json!(activity));
}

#[tokio::test]
async fn recent_audit_log_keeps_only_the_last_thirty_entries() {
    let app = router(AppState::new());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    for _ in 0..16 {
        let (status, _, enrolled) = request(
            &app,
            Method::POST,
            "/api/control/v1/devices",
            &[],
            Some(json!({"observed_device_id": simulator_id, "label": "Temporary blind"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let enrollment_id = enrolled.unwrap()["data"]["id"].as_str().unwrap().to_owned();
        let (status, _, _) = request(
            &app,
            Method::DELETE,
            &format!("/api/control/v1/devices/{enrollment_id}"),
            &[],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    let (status, _, activity) =
        request(&app, Method::GET, "/api/control/v1/audit-log", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let activity = activity.unwrap()["data"].as_array().unwrap().clone();
    assert_eq!(activity.len(), 30);
    assert_eq!(activity[0]["kind"], "enrollment_removed");
    assert_eq!(activity[29]["kind"], "enrollment_added");
}

#[tokio::test]
async fn recent_audit_log_records_batch_outcomes_and_simulator_changes() {
    let app = router(AppState::new());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let devices = devices.unwrap();
    for device in devices["data"].as_array().unwrap() {
        let (status, _, _) = request(
            &app,
            Method::POST,
            "/api/control/v1/devices",
            &[],
            Some(json!({
                "observed_device_id": device["id"],
                "label": device["advertised_name"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, _, accepted) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        &[("idempotency-key", "batch-key-not-logged")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let batch_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(wait_for_batch(&app, &batch_id).await["status"], "completed");

    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulators/simulator-1/scenario",
        &[],
        Some(json!({"scenario": "outcome_unknown", "battery_state": "observed_sample"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/dev/simulators/simulator-1/reset",
        &[],
        Some(json!({"position_closed_percent": 25})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, activity) =
        request(&app, Method::GET, "/api/control/v1/audit-log", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let activity = activity.unwrap()["data"].as_array().unwrap().clone();
    assert_eq!(activity[0]["kind"], "simulator_reset");
    assert_eq!(activity[0]["detail"], "Reset to 25% closed");
    assert_eq!(activity[1]["kind"], "simulator_settings_changed");
    assert_eq!(
        activity[1]["detail"],
        "Scenario: outcome_unknown, Battery: observed_sample"
    );
    assert_eq!(activity[2]["kind"], "batch");
    assert_eq!(activity[2]["status"], "completed");
    assert_eq!(activity[2]["action"], "close");
    assert!(
        !serde_json::to_string(&activity)
            .unwrap()
            .contains("batch-key-not-logged")
    );
}

#[tokio::test]
async fn durable_state_without_an_audit_log_field_loads_with_an_empty_log() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": simulator_id, "label": "Legacy state"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    drop(app);

    let bytes = std::fs::read(&state_path).unwrap();
    let mut persisted: Value = serde_json::from_slice(&bytes).unwrap();
    persisted.as_object_mut().unwrap().remove("audit_log");
    std::fs::write(&state_path, serde_json::to_vec_pretty(&persisted).unwrap()).unwrap();

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (status, _, activity) = request(
        &restored,
        Method::GET,
        "/api/control/v1/audit-log",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(activity.unwrap()["data"], json!([]));
}

#[tokio::test]
async fn durable_audit_log_is_trimmed_to_thirty_entries_when_loaded() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let simulator_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": simulator_id, "label": "Audit state"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    drop(app);

    let bytes = std::fs::read(&state_path).unwrap();
    let mut persisted: Value = serde_json::from_slice(&bytes).unwrap();
    persisted["audit_log"] = json!(
        (0..32)
            .map(|index| json!({
                "id": format!("entry-{index}"),
                "occurred_at": "2026-01-01T00:00:00Z",
                "kind": "enrollment_added",
                "subject": format!("Blind {index}")
            }))
            .collect::<Vec<_>>()
    );
    std::fs::write(&state_path, serde_json::to_vec_pretty(&persisted).unwrap()).unwrap();

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (status, _, activity) = request(
        &restored,
        Method::GET,
        "/api/control/v1/audit-log",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let activity = activity.unwrap()["data"].as_array().unwrap().clone();
    assert_eq!(activity.len(), 30);
    assert_eq!(activity[0]["id"], "entry-31");
    assert_eq!(activity[29]["id"], "entry-2");
}

#[tokio::test]
async fn documented_routes_are_registered_and_openapi_paths_are_complete() {
    let app = router(AppState::new());
    let expected: &[(&str, &[&str])] = &[
        ("/api/v1/health", &["get", "options"]),
        ("/api/v1/ready", &["get", "options"]),
        ("/api/v1/diagnostics", &["get", "options"]),
        ("/api/v1/devices", &["get", "options"]),
        ("/api/v1/devices/{id}", &["get", "options"]),
        ("/api/v1/history", &["get", "options"]),
        ("/api/dev/simulator", &["get", "options"]),
        ("/api/dev/simulator/scenario", &["put", "options"]),
        ("/api/dev/simulator/reset", &["post", "options"]),
        ("/api/dev/simulators", &["get", "options"]),
        ("/api/dev/simulators/{id}/scenario", &["put", "options"]),
        ("/api/dev/simulators/{id}/reset", &["post", "options"]),
        ("/api/control/v1/devices", &["get", "post", "options"]),
        ("/api/control/v1/devices/{id}", &["delete", "options"]),
        (
            "/api/control/v1/devices/{id}/commands",
            &["post", "options"],
        ),
        ("/api/control/v1/devices/{id}/toggle", &["post", "options"]),
        ("/api/control/v1/batches", &["post", "options"]),
        ("/api/control/v1/batches/{id}", &["get", "options"]),
        ("/api/control/v1/audit-log", &["get", "options"]),
        ("/api/control/v1/queue", &["get", "options"]),
        ("/api/control/v1/commands/{id}", &["get", "options"]),
        ("/openapi.json", &["get"]),
        ("/openapi.yaml", &["get"]),
        ("/docs", &["get"]),
    ];
    let (status, _, contract) = request(&app, Method::GET, "/openapi.json", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let contract = contract.unwrap();
    assert_eq!(contract["info"]["title"], "Somaris API");
    let managed = &contract["components"]["schemas"]["ManagedDevice"]["properties"];
    for field in [
        "capabilities",
        "calibration_mode_active",
        "advertisement_age_ms",
        "motion_feedback",
        "position_freshness",
        "position_source",
        "position_confidence",
        "position_observed_at",
        "requested_position_closed_percent",
        "requested_command_status",
    ] {
        assert!(
            managed.get(field).is_some(),
            "missing ManagedDevice.{field}"
        );
    }
    assert_eq!(
        contract["components"]["schemas"]["DeviceCapabilities"]["properties"]["set_position"]["type"],
        "boolean"
    );
    assert_eq!(
        contract["components"]["schemas"]["DeviceCapabilities"]["properties"]["set_motor_speed"]["type"],
        "boolean"
    );
    assert_eq!(
        contract["components"]["schemas"]["DeviceCapabilities"]["properties"]["calibrate_endpoints"]
            ["type"],
        "boolean"
    );
    assert_eq!(
        contract["components"]["schemas"]["ManagedDevice"]["properties"]["calibration_mode_active"]
            ["type"],
        "boolean"
    );
    let command_request = &contract["components"]["schemas"]["CommandRequest"];
    assert_eq!(
        command_request["properties"]["action"]["enum"],
        json!([
            "open",
            "close",
            "stop",
            "set_position",
            "set_motor_speed",
            "calibration_start",
            "calibration_stop",
            "calibration_nudge_open",
            "calibration_nudge_closed",
            "calibration_set_open_limit",
            "calibration_set_closed_limit"
        ])
    );
    assert_eq!(
        command_request["properties"]["position_closed_percent"]["maximum"],
        100
    );
    assert_eq!(
        command_request["properties"]["motor_speed_percent"]["minimum"],
        0
    );
    assert_eq!(
        command_request["properties"]["motor_speed_percent"]["maximum"],
        100
    );
    assert!(
        contract["components"]["schemas"]["Command"]["properties"]
            .get("target_closed_percent")
            .is_some()
    );
    assert!(
        contract["components"]["schemas"]["GattReadiness"]["properties"]
            .get("transport_evidence")
            .is_some()
    );
    assert!(
        contract["components"]["schemas"]["CommandAttempt"]["properties"]
            .get("phase_timings")
            .is_some()
    );
    assert!(
        contract["components"]["schemas"]["CommandAttempt"]["properties"]
            .get("failure_cause")
            .is_some()
    );
    assert!(
        contract["components"]["schemas"]["Command"]["properties"]
            .get("queue_wait_ms")
            .is_some()
    );
    assert!(
        contract["paths"]["/api/control/v1/devices/{id}/commands"]["post"]["responses"]
            .get("429")
            .is_some()
    );

    for (path, methods) in expected {
        let path_item = &contract["paths"][*path];
        assert!(path_item.is_object(), "missing OpenAPI path {path}");
        for method in *methods {
            assert!(
                path_item.get(method).is_some(),
                "missing OpenAPI operation {method} {path}"
            );
            let uri = path.replace("{id}", "contract-probe");
            let method_name = method.to_ascii_uppercase();
            let method = match *method {
                "get" => Method::GET,
                "post" => Method::POST,
                "put" => Method::PUT,
                "delete" => Method::DELETE,
                "options" => Method::OPTIONS,
                other => panic!("unexpected method in route inventory: {other}"),
            };
            let (status, headers, body) = request(&app, method, &uri, &[], None).await;
            assert_ne!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "documented method was not registered: {method_name} {uri}"
            );
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if status == StatusCode::NOT_FOUND {
                assert!(
                    content_type.contains("application/json")
                        && body
                            .as_ref()
                            .is_some_and(|body| body["error"]["code"].is_string()),
                    "documented route did not resolve: {method_name} {uri}"
                );
            }
        }
    }

    for (path, path_item) in contract["paths"].as_object().unwrap() {
        let Some((_, expected_methods)) =
            expected.iter().find(|(known, _)| *known == path.as_str())
        else {
            panic!("OpenAPI documents a route missing from the route inventory: {path}");
        };
        assert!(path_item.is_object());
        let mut documented_methods: Vec<_> = path_item
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| matches!(key.as_str(), "get" | "post" | "put" | "delete" | "options"))
            .map(String::as_str)
            .collect();
        let mut expected_methods: Vec<_> = expected_methods.to_vec();
        documented_methods.sort_unstable();
        expected_methods.sort_unstable();
        assert_eq!(
            documented_methods, expected_methods,
            "method drift on {path}"
        );
    }

    let (status, _, _) = request(&app, Method::GET, "/openapi.yaml", &[], None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn durable_state_restores_enrollment_and_idempotent_commands() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("nested/state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let observed_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": observed_id, "label": "Persisted blind"})),
    )
    .await;
    let managed_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, accepted) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "persisted-open")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let terminal = wait_for_command(&app, &command_id).await;
    assert_eq!(terminal["status"], "write_acknowledged");
    drop(app);

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (status, _, repeated) = request(
        &restored,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "persisted-open")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(repeated.unwrap()["data"]["id"], command_id);

    let (status, _, command) = request(
        &restored,
        Method::GET,
        &format!("/api/control/v1/commands/{command_id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(command.unwrap()["data"]["status"], "write_acknowledged");
}

#[tokio::test]
async fn failed_command_lookup_and_idempotency_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let observed_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": observed_id, "label": "Failed command blind"})),
    )
    .await;
    let managed_id = enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({"scenario": "gatt_write_error"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let command_url = format!("/api/control/v1/devices/{managed_id}/commands");
    let (status, _, accepted) = request(
        &app,
        Method::POST,
        &command_url,
        &[("idempotency-key", "persisted-write-failure")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let terminal = wait_for_command(&app, &command_id).await;
    assert_eq!(terminal["status"], "write_failed");
    assert_eq!(terminal["write_state"], "possibly_sent");
    assert_eq!(terminal["retryable"], false);
    drop(app);

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let command_url = format!("/api/control/v1/commands/{command_id}");
    let (status, _, command) = request(&restored, Method::GET, &command_url, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(command.unwrap()["data"]["status"], "write_failed");

    let (status, _, replay) = request(
        &restored,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "persisted-write-failure")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(replay.unwrap()["data"]["id"], command_id);

    let (status, _, command) = request(&restored, Method::GET, &command_url, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(command.unwrap()["data"]["status"], "write_failed");
}
