use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::Request,
    routing::any,
};
use serde_json::{Value, json};
use somaris::client::{ClientOptions, PositionInput, RemoteApiClient, validate_device_id};
use tokio::sync::Mutex;

type Requests = Arc<Mutex<Vec<(String, String, Value)>>>;

async fn mock_handler(State(requests): State<Requests>, request: Request<Body>) -> Json<Value> {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let bytes = to_bytes(request.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    requests.lock().await.push((method, path.clone(), body));
    if path.ends_with("/diagnostics") || path.ends_with("/connect-metadata") {
        Json(json!({
            "input_voltage_mv": 5100,
            "battery_voltage": 3980,
            "boot_count": "3",
            "undervoltage_count": "",
            "restart_reason_code": "4",
            "motor_stop_reason_code": 5,
            "sun": { "sunrise_time": "06:15", "sunset_time": "19:40" }
        }))
    } else {
        Json(json!({"ok": true}))
    }
}

fn mock_router(requests: Requests) -> Router {
    Router::new()
        .fallback(any(mock_handler))
        .with_state(requests)
}

#[tokio::test]
async fn client_defaults_to_thirty_seconds_and_validates_tls_by_default() {
    let client = RemoteApiClient::new("https://example.invalid/api").unwrap();
    assert_eq!(client.options().timeout.as_secs(), 30);
    assert!(client.options().reject_unauthorized);

    let options = ClientOptions {
        timeout: std::time::Duration::from_secs(7),
        reject_unauthorized: false,
    };
    let configured = RemoteApiClient::with_options("http://127.0.0.1:8080", options).unwrap();
    assert_eq!(configured.options(), options);
}

#[tokio::test]
async fn client_encodes_device_path_and_clamps_numeric_string_position() {
    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    client.set_device_position("Device A", "150").await.unwrap();

    let requests = requests.lock().await;
    let (_, path, body) = requests.last().unwrap();
    assert_eq!(path, "/api/device/Device%20A/position");
    assert_eq!(body, &json!({"position": 100.0}));
}

#[tokio::test]
async fn client_rejects_unsafe_identifiers_and_invalid_positions_before_sending() {
    for invalid in [
        "", "  ", " edge", "edge ", "a/b", "a\\b", "a?b", "a#b", "a..b",
    ] {
        assert!(validate_device_id(invalid).is_err(), "accepted {invalid:?}");
    }
    for valid in ["blind-1", "zone:west", "Device A"] {
        assert!(validate_device_id(valid).is_ok(), "rejected {valid:?}");
    }

    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    assert!(client.get_device_status("bad/id").await.is_err());
    assert!(
        client
            .set_device_position("blind-1", PositionInput::Number(f64::NAN))
            .await
            .is_err()
    );
    assert!(
        client
            .set_device_position("blind-1", "not a number")
            .await
            .is_err()
    );
    assert!(requests.lock().await.is_empty());
}

#[tokio::test]
async fn diagnostics_are_normalized_with_reason_labels_and_null_defaults() {
    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    let diagnostics = client.get_device_diagnostics("blind-1").await.unwrap();
    assert_eq!(diagnostics["inputVoltageMv"], 5100);
    assert_eq!(diagnostics["batteryVoltageMv"], 3980);
    assert_eq!(diagnostics["bootCount"], 3);
    assert_eq!(diagnostics["undervoltageCount"], Value::Null);
    assert_eq!(diagnostics["restartReasonLabel"], "BROWNOUT_RESET");
    assert_eq!(diagnostics["motorStopReasonLabel"], "LOW_BATTERY");
    assert_eq!(diagnostics["sunrise"], "06:15");
    assert_eq!(diagnostics["sunset"], "19:40");
    assert_eq!(diagnostics["firmwareVersion"], Value::Null);

    let metadata = client.get_connect_metadata("blind-1").await.unwrap();
    assert_eq!(metadata["sunrise"], "06:15");
    let requests = requests.lock().await;
    assert!(
        requests
            .iter()
            .any(|(_, path, _)| path == "/api/device/blind-1/connect-metadata")
    );
}

async fn test_client(requests: Requests) -> RemoteApiClient {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_router(requests)).await.unwrap();
    });
    RemoteApiClient::new(&format!("http://{address}/api")).unwrap()
}
