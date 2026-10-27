use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::api::{AppState, router};
use tower::ServiceExt;

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
    let value = if bytes.is_empty() {
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
    assert_eq!(body["meta"]["count"], 1);
    assert_eq!(body["data"][0]["is_simulated"], true);
    assert_eq!(body["data"][0]["position"]["current_closed_percent"], 50);
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
async fn commands_are_idempotent_and_acknowledgement_updates_simulated_target() {
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
    assert_eq!(
        devices.unwrap()["data"][0]["position"]["target_closed_percent"],
        0
    );
}

#[tokio::test]
async fn simulator_reports_ack_separately_then_advances_observed_position() {
    let state = AppState::new();
    state.spawn_background_tasks();
    let app = router(state);
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
        Some(json!({"observed_device_id": observed_id, "label": "Moving blind"})),
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
        &[("idempotency-key", "close-moving")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command = wait_for_command(&app, &command_id).await;
    assert_eq!(command["status"], "write_acknowledged");

    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let position = &devices.unwrap()["data"][0]["position"];
    assert_eq!(position["target_closed_percent"], 100);
    let initial_current = position["current_closed_percent"].as_u64().unwrap();
    assert!(initial_current < 100);

    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let (_, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    let moved_current = devices.unwrap()["data"][0]["position"]["current_closed_percent"]
        .as_u64()
        .unwrap();
    assert!(moved_current > initial_current);
    assert!(moved_current < 100);
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
}

#[tokio::test]
async fn history_is_explicitly_unavailable_when_not_configured() {
    let app = router(AppState::new());
    let (status, _, body) = request(&app, Method::GET, "/api/v1/history", &[], None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.unwrap()["error"]["code"], "history_disabled");
}

#[tokio::test]
async fn api_contract_is_served_with_somaris_branding() {
    let app = router(AppState::new());
    let (status, _, body) = request(&app, Method::GET, "/openapi.json", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    assert_eq!(body["info"]["title"], "Somaris API");
    assert!(body["paths"]["/api/v1/devices"].is_object());
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
