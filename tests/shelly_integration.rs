use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode, header},
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use somaris::api::{AppState, router};
use somaris::integration::{BlindCommand, BlindDriver, DriverFailureKind, ShellyCoverDriver};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

type RpcRequests = Arc<Mutex<Vec<Value>>>;

async fn mock_rpc(
    State(requests): State<RpcRequests>,
    Path(method): Path<String>,
    Json(params): Json<Value>,
) -> Json<Value> {
    requests
        .lock()
        .unwrap()
        .push(json!({"method": method, "params": params}));
    match method.as_str() {
        "Cover.GetStatus" => {
            let id = params["id"].as_u64().unwrap_or_default();
            Json(json!({
                "id": id,
                "state": "closing",
                "current_pos": 25.0,
                "target_pos": 60.0,
                "pos_control": id != 2,
            }))
        }
        _ => Json(json!({})),
    }
}

async fn mock_shelly() -> (String, RpcRequests, JoinHandle<()>) {
    let requests = RpcRequests::default();
    let app = Router::new()
        .route("/rpc/{method}", post(mock_rpc))
        .with_state(requests.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), requests, task)
}

#[tokio::test]
async fn shelly_rpc_server_error_leaves_command_outcome_unknown() {
    let app = Router::new().route(
        "/rpc/{method}",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "device_failure"})),
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let driver = ShellyCoverDriver::new();
    let failure = driver
        .execute(
            &json!({
                "base_url": format!("http://{address}"),
                "component_id": 0
            }),
            BlindCommand::Open,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.kind, DriverFailureKind::OutcomeUnknown);
    server.abort();
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Option<Value>) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = (!bytes.is_empty())
        .then(|| serde_json::from_slice(&bytes).expect("response should be JSON"));
    (status, json)
}

async fn enroll_shelly(app: &Router, base_url: &str) -> String {
    enroll_shelly_component(app, base_url, 1).await
}

async fn enroll_shelly_component(app: &Router, base_url: &str, component_id: u32) -> String {
    let (status, body) = request(
        app,
        Method::POST,
        "/api/control/v1/devices",
        &[(header::CONTENT_TYPE.as_str(), "application/json")],
        Some(json!({
            "label": "Office Shade",
            "driver_id": "shelly_cover",
            "driver_config": {
                "base_url": base_url,
                "component_id": component_id
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    body.unwrap()["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn uncalibrated_shelly_covers_reject_position_commands_by_capability() {
    let (base_url, requests, server) = mock_shelly().await;
    let app = router(AppState::new());
    let enrollment_id = enroll_shelly_component(&app, &base_url, 2).await;
    let (status, body) = request(
        &app,
        Method::GET,
        "/api/control/v1/devices",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let device = body.unwrap()["data"][0].clone();
    assert_eq!(device["enrollment"]["id"], enrollment_id);
    assert_eq!(device["capabilities"]["set_position"], false);

    let (status, body) = request(
        &app,
        Method::POST,
        &format!("/api/control/v1/devices/{enrollment_id}/commands"),
        &[
            (header::CONTENT_TYPE.as_str(), "application/json"),
            ("idempotency-key", "shelly-uncalibrated-position"),
        ],
        Some(json!({
            "action": "set_position",
            "position_closed_percent": 40
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body:?}");
    assert_eq!(body.unwrap()["error"]["code"], "unsupported_action");
    assert!(
        !requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request["method"] == "Cover.GoToPosition")
    );
    server.abort();
}

#[tokio::test]
async fn shelly_cover_can_be_enrolled_and_its_position_is_normalized() {
    let (base_url, requests, server) = mock_shelly().await;
    let app = router(AppState::new());
    let (status, body) = request(&app, Method::GET, "/api/control/v1/drivers", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|driver| driver == "shelly_cover")
    );
    let enrollment_id = enroll_shelly(&app, &base_url).await;

    let (status, body) = request(
        &app,
        Method::GET,
        "/api/control/v1/devices",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let device = body.unwrap()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["enrollment"]["id"] == enrollment_id)
        .unwrap()
        .clone();
    assert_eq!(device["capabilities"]["open"], true);
    assert_eq!(device["capabilities"]["set_position"], true);
    assert_eq!(device["position"]["current_closed_percent"], 75);
    assert_eq!(device["position"]["target_closed_percent"], 40);
    assert_eq!(device["movement_status"], "closing");
    assert_eq!(device["position_source"], "lan_integration");
    assert!(requests.lock().unwrap().iter().any(|request| {
        request["method"] == "Cover.GetStatus" && request["params"]["id"] == 1
    }));
    server.abort();
}

#[tokio::test]
async fn set_position_dispatches_through_shelly_rpc_with_inverted_position() {
    let (base_url, requests, server) = mock_shelly().await;
    let app = router(AppState::new());
    let enrollment_id = enroll_shelly(&app, &base_url).await;
    let path = format!("/api/control/v1/devices/{enrollment_id}/commands");
    let (status, accepted) = request(
        &app,
        Method::POST,
        &path,
        &[
            (header::CONTENT_TYPE.as_str(), "application/json"),
            ("idempotency-key", "shelly-position-1"),
        ],
        Some(json!({
            "action": "set_position",
            "position_closed_percent": 40
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted:?}");
    let command_id = accepted.unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let command_path = format!("/api/control/v1/commands/{command_id}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let command = loop {
        let (status, body) = request(&app, Method::GET, &command_path, &[], None).await;
        assert_eq!(status, StatusCode::OK);
        let command = body.unwrap()["data"].clone();
        if command["status"] != "queued" && command["status"] != "connecting" {
            break command;
        }
        assert!(tokio::time::Instant::now() < deadline, "command did not finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(command["status"], "write_acknowledged");
    assert!(requests.lock().unwrap().iter().any(|request| {
        request["method"] == "Cover.GoToPosition"
            && request["params"]["id"] == 1
            && request["params"]["pos"] == 60.0
    }));
    server.abort();
}

#[tokio::test]
async fn unsupported_driver_and_unsafe_urls_are_rejected_before_rpc() {
    let (base_url, requests, server) = mock_shelly().await;
    let app = router(AppState::new());
    let payload = |driver_id: &str, base_url: &str| {
        json!({
            "label": "Office Shade",
            "driver_id": driver_id,
            "driver_config": {"base_url": base_url, "component_id": 0}
        })
    };
    let (status, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[(header::CONTENT_TYPE.as_str(), "application/json")],
        Some(payload("unknown_driver", &base_url)),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = request(
        &app,
        Method::POST,
        "/api/control/v1/devices",
        &[(header::CONTENT_TYPE.as_str(), "application/json")],
        Some(payload("shelly_cover", "http://user:password@127.0.0.1:8123")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(requests.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn driver_configuration_persists_and_reconnects_after_restart() {
    let (base_url, _requests, server) = mock_shelly().await;
    let directory = tempfile::tempdir().unwrap();
    let state_path: PathBuf = directory.path().join("state.json");
    let app = router(AppState::from_data_path(&state_path).await.unwrap());
    let enrollment_id = enroll_shelly(&app, &base_url).await;
    drop(app);

    let restored = router(AppState::from_data_path(&state_path).await.unwrap());
    let (status, body) = request(
        &restored,
        Method::GET,
        "/api/control/v1/devices",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let device = body.unwrap()["data"][0].clone();
    assert_eq!(device["enrollment"]["id"], enrollment_id);
    assert_eq!(device["enrollment"]["driver_id"], "shelly_cover");
    assert_eq!(
        device["enrollment"]["driver_config"]["component_id"],
        1
    );
    assert_eq!(device["availability"], "ready");
    server.abort();
}
