use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::api::QueueConfig;
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
        Some(
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    };
    (status, headers, value)
}

async fn enroll_device(app: &Router, label: &str) -> String {
    let (_, _, devices) = request(app, Method::GET, "/api/v1/devices", &[], None).await;
    let observed_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, enrollment) = request(
        app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": observed_id, "label": label})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    enrollment.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn wait_for_command(app: &Router, command_id: &str) -> Value {
    for _ in 0..500 {
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
    for _ in 0..500 {
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
async fn simulator_prewrite_failure_is_retryable() {
    let app = router(AppState::new());
    let managed_id = enroll_device(&app, "Test window").await;
    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({"scenario": "pre_write_failure"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "prewrite-1")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command = wait_for_command(&app, &id).await;
    assert_eq!(command["status"], "connection_failed");
    assert_eq!(command["write_state"], "not_sent");
    assert_eq!(command["retryable"], true);
    assert_eq!(command["attempt"], 1);
}

#[tokio::test]
async fn simulator_write_error_is_uncertain_and_not_retryable() {
    let app = router(AppState::new());
    let managed_id = enroll_device(&app, "Test window").await;
    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({"scenario": "gatt_write_error"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/commands"),
        &[("idempotency-key", "write-error-1")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let command = wait_for_command(&app, &id).await;
    assert_eq!(command["status"], "write_failed");
    assert_eq!(command["write_state"], "possibly_sent");
    assert_eq!(command["retryable"], false);
    assert_eq!(command["attempt"], 1);
}

#[tokio::test]
async fn toggle_uses_observed_closed_percent_and_returns_its_basis() {
    let app = router(AppState::new());
    let managed_id = enroll_device(&app, "Test window").await;
    let (status, _, toggle) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/toggle"),
        &[("idempotency-key", "toggle-1")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let toggle = toggle.unwrap()["data"].clone();
    assert_eq!(toggle["action"], "open");
    assert_eq!(toggle["toggle_basis"], "observed_target");
    assert_eq!(toggle["toggle_position"]["target_closed_percent"], 100);
    let id = toggle["id"].as_str().unwrap().to_owned();
    let _ = wait_for_command(&app, &id).await;

    let (status, _, toggle) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{managed_id}/toggle"),
        &[("idempotency-key", "toggle-2")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(toggle.unwrap()["data"]["action"], "close");
}

#[tokio::test]
async fn batches_are_idempotent_and_report_child_command_results() {
    let app = router(AppState::new());
    let _managed_id = enroll_device(&app, "Test window").await;
    let (status, _, batch) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        &[("idempotency-key", "batch-close-1")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let batch_id = batch.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let final_batch = wait_for_batch(&app, &batch_id).await;
    assert_eq!(final_batch["status"], "completed");
    assert_eq!(final_batch["acknowledged_count"], 1);
    assert_eq!(
        final_batch["targets"][0]["command"]["status"],
        "write_acknowledged"
    );

    let (status, _, repeated) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        &[("idempotency-key", "batch-close-1")],
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(repeated.unwrap()["data"]["id"], batch_id);

    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        &[("idempotency-key", "batch-close-1")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn history_can_be_enabled_and_paged_from_a_persistent_file() {
    let directory = tempfile::tempdir().unwrap();
    let history_path = directory.path().join("history.jsonl");
    let mut app = router(AppState::with_history_path(&history_path));

    let (status, _, first_page) = request(
        &app,
        Method::GET,
        "/api/v1/history?after=0&limit=10",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first_page.unwrap()["data"], json!([]));

    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/dev/simulator/reset",
        &[],
        Some(json!({"position_closed_percent": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(history_path.exists());

    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&history_path)
        .unwrap()
        .write_all(b"{\"partial\"")
        .unwrap();
    app = router(AppState::with_history_path(&history_path));

    for position in [20, 40] {
        let (status, _, _) = request(
            &app,
            Method::POST,
            "/api/dev/simulator/reset",
            &[],
            Some(json!({"position_closed_percent": position})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, _, page) = request(
        &app,
        Method::GET,
        "/api/v1/history?after=0&limit=1",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = page.unwrap();
    assert_eq!(page["data"][0]["position"]["current_closed_percent"], 0);
    assert_eq!(page["meta"]["next_cursor"], 1);
    let saved_lines = std::fs::read_to_string(&history_path).unwrap();
    assert_eq!(saved_lines.lines().count(), 3);
}

#[tokio::test]
async fn history_etag_ignores_the_response_generation_timestamp() {
    let directory = tempfile::tempdir().unwrap();
    let history_path = directory.path().join("history.jsonl");
    let app = router(AppState::with_history_path(&history_path));
    let (status, headers, _) = request(
        &app,
        Method::GET,
        "/api/v1/history?after=0&limit=10",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    let (status, headers, _) = request(
        &app,
        Method::GET,
        "/api/v1/history?after=0&limit=10",
        &[("if-none-match", etag)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "private, no-cache"
    );
}

#[tokio::test]
async fn filtered_history_does_not_advertise_an_empty_next_page() {
    let directory = tempfile::tempdir().unwrap();
    let history_path = directory.path().join("history.jsonl");
    let app = router(AppState::with_history_path(&history_path));
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/dev/simulator/reset",
        &[],
        Some(json!({"position_closed_percent": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, page) = request(
        &app,
        Method::GET,
        "/api/v1/history?after=0&limit=1&device_id=not-a-device",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = page.unwrap();
    assert_eq!(page["data"], json!([]));
    assert_eq!(page["meta"]["has_more"], false);
    assert_eq!(page["meta"]["next_cursor"], 0);
}

#[tokio::test]
async fn conditional_get_accepts_weak_form_of_its_etag() {
    let app = router(AppState::new());
    let (status, headers, _) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    assert!(etag.starts_with("W/"));
    let weak_etag = if etag.starts_with("W/") {
        etag.to_owned()
    } else {
        format!("W/{etag}")
    };

    let (status, headers, _) = request(
        &app,
        Method::GET,
        "/api/v1/devices",
        &[("if-none-match", &weak_etag)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "private, no-cache"
    );
}

#[tokio::test]
async fn mutable_api_gets_have_explicit_cache_policies_and_command_etags() {
    let app = router(AppState::new());
    let (status, headers, _) = request(&app, Method::GET, "/api/v1/health", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");

    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let device_id = devices.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, enrollment) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[],
        Some(json!({"observed_device_id": device_id, "label": "Test"})),
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
        &[("idempotency-key", "cache-test")],
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let path = format!("/api/control/v1/commands/{command_id}");
    let (status, headers, _) = request(&app, Method::GET, &path, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "private, no-cache"
    );
    let (status, headers, _) =
        request(&app, Method::GET, &path, &[("if-none-match", etag)], None).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "private, no-cache"
    );
}

#[tokio::test]
async fn static_api_documents_are_cacheable() {
    let app = router(AppState::new());
    for path in ["/docs", "/openapi.json", "/openapi.yaml"] {
        let (status, headers, _) = request(&app, Method::GET, path, &[], None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=3600"
        );
        let etag = headers.get(header::ETAG).unwrap().to_str().unwrap();
        let (status, headers, _) =
            request(&app, Method::GET, path, &[("if-none-match", etag)], None).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=3600"
        );
    }
}

#[tokio::test]
async fn simulator_profiles_drive_battery_and_wireless_snapshots() {
    let app = router(AppState::new());
    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({
            "wireless_profile": "hallway_observed",
            "battery_state": "observed_sample"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, simulator) = request(&app, Method::GET, "/api/dev/simulator", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(simulator.unwrap()["data"]["connection_delay_ms"], 80);
    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices.unwrap()["data"][0]["battery"]["voltage_mv"], 4230);

    let (status, _, reset) = request(
        &app,
        Method::POST,
        "/api/dev/simulator/reset",
        &[],
        Some(json!({"position_closed_percent": 100})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reset.unwrap()["data"]["scenario"], "normal");
}

#[tokio::test]
async fn simulator_reset_restores_position_and_reports_at_target() {
    let app = router(AppState::new());
    let (status, _, reset) = request(
        &app,
        Method::POST,
        "/api/dev/simulator/reset",
        &[],
        Some(json!({"position_closed_percent": 35})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let reset = reset.unwrap()["data"].clone();
    assert_eq!(reset["current_closed_percent"], 35);
    assert_eq!(reset["target_closed_percent"], 35);

    let (status, _, devices) = request(&app, Method::GET, "/api/v1/devices", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let device = &devices.unwrap()["data"][0];
    assert_eq!(device["position"]["current_closed_percent"], 35);
    assert_eq!(device["position"]["target_closed_percent"], 35);
}

#[tokio::test]
async fn invalid_simulator_values_are_rejected_with_json_errors() {
    let app = router(AppState::new());
    let (status, _, body) = request(
        &app,
        Method::PUT,
        "/api/dev/simulator/scenario",
        &[],
        Some(json!({"wireless_profile": "impossible"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.unwrap()["error"]["code"], "invalid_request_body");

    let (status, _, body) = request(
        &app,
        Method::POST,
        "/api/dev/simulator/reset",
        &[],
        Some(json!({"position_closed_percent": 101})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.unwrap()["error"]["code"], "invalid_position");
}

#[tokio::test]
async fn openapi_yaml_matches_the_json_api_contract() {
    let app = router(AppState::new());
    let (status, headers, body) = request(&app, Method::GET, "/openapi.yaml", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("yaml")
    );
    let yaml: serde_yaml::Value = serde_yaml::from_str(body.unwrap().as_str().unwrap()).unwrap();
    let yaml: Value = serde_json::to_value(yaml).unwrap();
    let (_, _, json_contract) = request(&app, Method::GET, "/openapi.json", &[], None).await;
    let json_contract = json_contract.unwrap();
    assert_eq!(yaml["info"]["title"], "Somaris API");
    assert_eq!(yaml["paths"], json_contract["paths"]);
}

#[tokio::test]
async fn api_docs_page_serves_html_with_json_and_yaml_contract_links() {
    let app = router(AppState::new());
    let (status, headers, body) = request(&app, Method::GET, "/docs", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/html")
    );
    let html = body.unwrap().as_str().unwrap().to_owned();
    assert!(html.contains("OpenAPI reference"));
    assert!(html.contains("/openapi.json"));
    assert!(html.contains("/openapi.yaml"));
}

#[tokio::test]
async fn cors_preflight_requires_an_explicit_allowed_origin() {
    let app = router(
        AppState::new()
            .with_allowed_origins(["https://console.example"])
            .unwrap(),
    );
    let (status, headers, _) = request(
        &app,
        Method::OPTIONS,
        "/api/v1/devices",
        &[
            ("origin", "https://console.example"),
            ("access-control-request-method", "GET"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap()
            .to_str()
            .unwrap(),
        "https://console.example"
    );

    let (status, headers, _) = request(
        &app,
        Method::OPTIONS,
        "/api/v1/devices",
        &[
            ("origin", "https://unlisted.example"),
            ("access-control-request-method", "GET"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn queue_limits_match_documented_defaults_and_can_be_configured() {
    let default_app = router(AppState::new());
    let (status, _, body) = request(
        &default_app,
        Method::GET,
        "/api/control/v1/queue",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let queue = body.unwrap()["data"].clone();
    assert_eq!(queue["max_outstanding_commands"], 1000);
    assert_eq!(queue["max_queued_per_device"], 32);
    assert_eq!(queue["adapter_concurrency"], 1);
    assert_eq!(queue["adapter_scan_window_ms"], 5000);

    let configured_app = router(
        AppState::new()
            .with_queue_config(QueueConfig {
                max_outstanding_commands: 7,
                max_queued_per_device: 3,
                adapter_concurrency: 2,
                adapter_scan_window_ms: 250,
                queue_wait_timeout_seconds: 90,
            })
            .unwrap(),
    );
    let (_, _, body) = request(
        &configured_app,
        Method::GET,
        "/api/control/v1/queue",
        &[],
        None,
    )
    .await;
    let queue = body.unwrap()["data"].clone();
    assert_eq!(queue["max_outstanding_commands"], 7);
    assert_eq!(queue["max_queued_per_device"], 3);
    assert_eq!(queue["adapter_concurrency"], 2);
    assert_eq!(queue["adapter_scan_window_ms"], 250);
}

#[test]
fn adapter_scan_window_must_stay_within_bounded_limits() {
    for invalid_window in [99, 10_001] {
        assert!(
            AppState::new()
                .with_queue_config(QueueConfig {
                    adapter_scan_window_ms: invalid_window,
                    ..QueueConfig::default()
                })
                .is_err()
        );
    }
}
