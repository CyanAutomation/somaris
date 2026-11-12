use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::api::{AppState, router};
use tower::ServiceExt;

const ORIGIN: &str = "http://somaris.test";

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Option<Value>) {
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(uri)
        .header(header::HOST, "somaris.test");
    if method != Method::GET && method != Method::OPTIONS {
        builder = builder.header(header::ORIGIN, ORIGIN);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if uri.contains("/commands") || uri == "/api/control/v1/batches" {
        builder = builder.header("idempotency-key", "demo-isolation-test");
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

async fn create_session(app: &Router) -> String {
    let (status, headers, _) = request(app, Method::POST, "/api/demo/session", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert!(cookie.starts_with("somaris-demo-session="));
    cookie
}

async fn devices(app: &Router, cookie: &str) -> Value {
    let (status, _, body) = request(
        app,
        Method::GET,
        "/api/control/v1/devices",
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body.unwrap()["data"].clone()
}

async fn wait_for_command(app: &Router, cookie: &str, command_id: &str) -> Value {
    for _ in 0..100 {
        let (status, _, body) = request(
            app,
            Method::GET,
            &format!("/api/control/v1/commands/{command_id}"),
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let command = body.unwrap()["data"].clone();
        if !matches!(command["status"].as_str(), Some("queued" | "connecting")) {
            return command;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("demo command did not reach a terminal state");
}

fn positions(devices: &Value) -> Vec<(String, u64, u64)> {
    devices
        .as_array()
        .unwrap()
        .iter()
        .map(|device| {
            (
                device["enrollment"]["label"].as_str().unwrap().to_owned(),
                device["position"]["current_closed_percent"]
                    .as_u64()
                    .unwrap(),
                device["position"]["target_closed_percent"]
                    .as_u64()
                    .unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn demo_bootstrap_seeds_four_observed_blinds_and_reports_simulator_readiness() {
    let app = router(AppState::demo());
    let (status, _, mode) = request(&app, Method::GET, "/api/v1/demo", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mode.unwrap()["data"]["demo_mode"], true);

    let cookie = create_session(&app).await;
    let managed = devices(&app, &cookie).await;
    assert_eq!(managed.as_array().unwrap().len(), 4);
    let expected = [
        ("Bedroom — Blackout Blind", "Bedroom", 100),
        ("Living Room — Main Window", "Living Room", 75),
        ("Living Room — Side Window", "Living Room", 50),
        ("Study — Desk Window", "Study", 25),
    ];
    for (device, (name, room, position)) in managed.as_array().unwrap().iter().zip(expected) {
        assert_eq!(device["enrollment"]["label"], name);
        assert_eq!(device["enrollment"]["room"], room);
        assert_eq!(device["availability"], "ready");
        assert_eq!(device["position"]["current_closed_percent"], position);
        assert_eq!(device["position"]["target_closed_percent"], position);
        assert_eq!(device["position_source"], "simulator");
    }
    let ids: std::collections::HashSet<_> = managed
        .as_array()
        .unwrap()
        .iter()
        .map(|device| device["enrollment"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 4);

    let (status, _, ready) = request(&app, Method::GET, "/api/v1/ready", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready.as_ref().unwrap()["data"]["demo_mode"], true);
    assert_eq!(ready.as_ref().unwrap()["data"]["simulator_ready"], true);
    assert_eq!(
        ready.as_ref().unwrap()["data"]["bluetooth_adapter_ready"],
        false
    );
    assert_eq!(ready.unwrap()["data"]["scanner_status"], "unavailable");
}

#[tokio::test]
async fn each_seeded_demo_blind_accepts_its_own_percentage_target() {
    let app = router(AppState::demo());
    let cookie = create_session(&app).await;
    let seeded = devices(&app, &cookie).await;
    let targets = [10, 30, 70, 90];

    for (device, target) in seeded.as_array().unwrap().iter().zip(targets) {
        let id = device["enrollment"]["id"].as_str().unwrap();
        let (status, _, accepted) = request(
            &app,
            Method::POST,
            &format!("/api/control/v1/devices/{id}/commands"),
            Some(&cookie),
            Some(json!({
                "action": "set_position",
                "position_closed_percent": target
            })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let accepted = accepted.unwrap()["data"].clone();
        assert_eq!(accepted["target_closed_percent"], target);
        let command_id = accepted["id"].as_str().unwrap();
        assert_eq!(
            wait_for_command(&app, &cookie, command_id).await["status"],
            "write_acknowledged"
        );
    }

    let controlled = devices(&app, &cookie).await;
    for (device, target) in controlled.as_array().unwrap().iter().zip(targets) {
        assert_eq!(device["position"]["target_closed_percent"], target);
    }
}

#[tokio::test]
async fn demo_visitors_are_isolated_and_reset_only_the_current_session() {
    let app = router(AppState::demo());
    let visitor_a = create_session(&app).await;
    let visitor_b = create_session(&app).await;
    let before_a = devices(&app, &visitor_a).await;
    let before_b = devices(&app, &visitor_b).await;
    let main_id = before_a
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["label"] == "Living Room — Main Window")
        .unwrap()["enrollment"]["id"]
        .as_str()
        .unwrap();

    let (status, _, command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{main_id}/commands"),
        Some(&visitor_a),
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = command.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let mut command_status = String::new();
    for _ in 0..100 {
        let (status, _, command) = request(
            &app,
            Method::GET,
            &format!("/api/control/v1/commands/{command_id}"),
            Some(&visitor_a),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        command_status = command.unwrap()["data"]["status"]
            .as_str()
            .unwrap()
            .to_owned();
        if command_status != "queued" && command_status != "connecting" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(command_status, "write_acknowledged");

    let (status, _, cross_session_command) = request(
        &app,
        Method::GET,
        &format!("/api/control/v1/commands/{command_id}"),
        Some(&visitor_b),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        cross_session_command.unwrap()["error"]["code"],
        "command_not_found"
    );
    let after_command_b = devices(&app, &visitor_b).await;
    assert_eq!(positions(&after_command_b), positions(&before_b));
    let after_command_a = devices(&app, &visitor_a).await;
    assert_eq!(after_command_a[1]["position"]["target_closed_percent"], 100);

    let (status, _, batch) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        Some(&visitor_a),
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let batch_id = batch.unwrap()["data"]["id"].as_str().unwrap().to_owned();

    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/demo/reset",
        Some(&visitor_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        positions(&devices(&app, &visitor_a).await),
        positions(&before_a)
    );
    assert_eq!(
        positions(&devices(&app, &visitor_b).await),
        positions(&before_b)
    );
    let (status, _, missing_batch) = request(
        &app,
        Method::GET,
        &format!("/api/control/v1/batches/{batch_id}"),
        Some(&visitor_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing_batch.unwrap()["error"]["code"], "batch_not_found");

    let (status, _, activity) = request(
        &app,
        Method::GET,
        "/api/control/v1/audit-log",
        Some(&visitor_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(activity.unwrap()["data"], json!([]));

    let (status, _, replacement_command) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{main_id}/commands"),
        Some(&visitor_a),
        Some(json!({"action": "close"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_ne!(
        replacement_command.unwrap()["data"]["id"],
        command_id,
        "reset must clear the old session idempotency record"
    );
}

#[tokio::test]
async fn expired_cookie_is_replaced_and_normal_mode_has_no_demo_session_route() {
    let demo = router(AppState::demo());
    let (status, _, missing) = request(
        &demo,
        Method::GET,
        "/api/control/v1/devices",
        Some("somaris-demo-session=expired-invalid"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing.unwrap()["error"]["code"], "demo_session_expired");
    let (status, _, unscoped_simulators) =
        request(&demo, Method::GET, "/api/dev/simulators", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unscoped_simulators.unwrap()["error"]["code"],
        "demo_session_expired"
    );
    let (status, headers, _) = request(
        &demo,
        Method::POST,
        "/api/demo/session",
        Some("somaris-demo-session=expired-invalid"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let replacement = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_ne!(replacement, "somaris-demo-session=expired-invalid");
    assert_eq!(
        devices(&demo, &replacement).await.as_array().unwrap().len(),
        4
    );

    let normal = router(AppState::new());
    let (status, _, mode) = request(&normal, Method::GET, "/api/v1/demo", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mode.unwrap()["data"]["demo_mode"], false);
    let (status, _, _) = request(&normal, Method::POST, "/api/demo/session", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn batches_and_simulator_settings_are_scoped_to_the_demo_cookie() {
    let app = router(AppState::demo());
    let visitor_a = create_session(&app).await;
    let visitor_b = create_session(&app).await;

    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/dev/simulators/simulator-1/scenario",
        Some(&visitor_a),
        Some(json!({"scenario": "telemetry_paused"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, scenario_a) = request(
        &app,
        Method::GET,
        "/api/dev/simulators",
        Some(&visitor_a),
        None,
    )
    .await;
    let (_, _, scenario_b) = request(
        &app,
        Method::GET,
        "/api/dev/simulators",
        Some(&visitor_b),
        None,
    )
    .await;
    let scenario_for = |body: &Value| {
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|simulator| simulator["id"] == "simulator-1")
            .unwrap()["scenario"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(scenario_for(&scenario_a.unwrap()), "telemetry_paused");
    assert_eq!(scenario_for(&scenario_b.unwrap()), "normal");

    let (_, _, untouched_activity) = request(
        &app,
        Method::GET,
        "/api/control/v1/audit-log",
        Some(&visitor_b),
        None,
    )
    .await;
    assert_eq!(untouched_activity.unwrap()["data"], json!([]));

    let (status_a, _, batch_a) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        Some(&visitor_a),
        Some(json!({"action": "open"})),
    )
    .await;
    let (status_b, _, batch_b) = request(
        &app,
        Method::POST,
        "/api/control/v1/batches",
        Some(&visitor_b),
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status_a, StatusCode::ACCEPTED);
    assert_eq!(status_b, StatusCode::ACCEPTED);
    let batch_a_id = batch_a.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let batch_b_id = batch_b.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    assert_ne!(batch_a_id, batch_b_id);
    let (status, _, foreign_batch) = request(
        &app,
        Method::GET,
        &format!("/api/control/v1/batches/{batch_a_id}"),
        Some(&visitor_b),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(foreign_batch.unwrap()["error"]["code"], "batch_not_found");
}

#[tokio::test]
async fn reset_cancels_in_flight_simulator_commands_before_restoring_seed_state() {
    let app = router(AppState::demo());
    let visitor = create_session(&app).await;
    let main = devices(&app, &visitor).await;
    let main_id = main
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["label"] == "Bedroom — Blackout Blind")
        .unwrap()["enrollment"]["id"]
        .as_str()
        .unwrap();
    let (status, _, accepted) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{main_id}/commands"),
        Some(&visitor),
        Some(json!({"action": "open"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let command_id = accepted.unwrap()["data"]["id"].as_str().unwrap().to_owned();
    let mut command_state = String::new();
    for _ in 0..100 {
        let (status, _, command) = request(
            &app,
            Method::GET,
            &format!("/api/control/v1/commands/{command_id}"),
            Some(&visitor),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        command_state = command.unwrap()["data"]["status"]
            .as_str()
            .unwrap()
            .to_owned();
        if command_state == "connecting" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert_eq!(command_state, "connecting");

    let (status, _, _) = request(&app, Method::POST, "/api/demo/reset", Some(&visitor), None).await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let (status, _, missing_command) = request(
        &app,
        Method::GET,
        &format!("/api/control/v1/commands/{command_id}"),
        Some(&visitor),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing_command.unwrap()["error"]["code"],
        "command_not_found"
    );
    let after_reset = devices(&app, &visitor).await;
    let bedroom = after_reset
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["label"] == "Bedroom — Blackout Blind")
        .unwrap();
    assert_eq!(bedroom["position"]["target_closed_percent"], 100);
    assert_eq!(bedroom["position"]["current_closed_percent"], 100);
}

#[tokio::test]
async fn demo_cookie_is_http_only_secure_on_https_and_mutations_reject_cross_site_origins() {
    let app = router(AppState::demo());
    let bootstrap = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/demo/session")
                .header(header::HOST, "somaris.test")
                .header(header::ORIGIN, "https://somaris.test")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), StatusCode::OK);
    let cookie = bootstrap
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("Path=/"));
    let cookie_pair = cookie.split(';').next().unwrap().to_owned();
    let original = devices(&app, &cookie_pair).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/demo/reset")
                .header(header::HOST, "somaris.test")
                .header(header::ORIGIN, "https://attacker.test")
                .header(header::COOKIE, &cookie_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "demo_origin_denied");
    assert_eq!(
        positions(&devices(&app, &cookie_pair).await),
        positions(&original)
    );
}

#[tokio::test]
async fn demo_session_capacity_is_bounded() {
    let app = router(AppState::demo());
    let mut last_status = StatusCode::OK;
    for _ in 0..33 {
        let (status, _, _) = request(&app, Method::POST, "/api/demo/session", None, None).await;
        last_status = status;
    }
    assert_eq!(last_status, StatusCode::SERVICE_UNAVAILABLE);
}
