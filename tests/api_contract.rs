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
    assert_eq!(body["data"][0]["position"]["current_closed_percent"], 100);
    let payload = hex::decode(
        body["data"][0]["manufacturer_payload_hex"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let advertisement = somaris::ble::parse_soma_payload(&payload).unwrap();
    assert_eq!(advertisement.advertised_name, "Hallway Blinds0");
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
}

#[tokio::test]
async fn history_is_explicitly_unavailable_when_not_configured() {
    let app = router(AppState::new());
    let (status, _, body) = request(&app, Method::GET, "/api/v1/history", &[], None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.unwrap()["error"]["code"], "history_disabled");
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
        ("/api/control/v1/devices", &["get", "post", "options"]),
        ("/api/control/v1/devices/{id}", &["delete", "options"]),
        (
            "/api/control/v1/devices/{id}/commands",
            &["post", "options"],
        ),
        ("/api/control/v1/devices/{id}/toggle", &["post", "options"]),
        ("/api/control/v1/batches", &["post", "options"]),
        ("/api/control/v1/batches/{id}", &["get", "options"]),
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
