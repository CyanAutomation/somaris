use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::any,
};
use serde_json::{Value, json};
use somaris::client::{ClientOptions, RemoteAction, RemoteApiClient, validate_device_id};
use tokio::sync::Mutex;

type CapturedRequest = (String, String, HeaderMap, Value);
type Requests = Arc<Mutex<Vec<CapturedRequest>>>;

async fn mock_handler(
    State(requests): State<Requests>,
    request: Request<Body>,
) -> impl IntoResponse {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let bytes = to_bytes(request.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    requests
        .lock()
        .await
        .push((method.clone(), path.clone(), headers, body));

    if method == "DELETE" {
        return (StatusCode::NO_CONTENT, Body::empty()).into_response();
    }
    let response = if path == "/api/v1/diagnostics" {
        json!({"data": {"scanner_status": "scanning", "soma_advertisements_seen": 5}})
    } else {
        json!({"ok": true})
    };
    Json(response).into_response()
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
async fn telemetry_client_uses_documented_v1_paths() {
    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    client.get_device_status("Device A").await.unwrap();
    client.get_device_list().await.unwrap();
    let diagnostics = client.get_diagnostics().await.unwrap();
    assert_eq!(diagnostics["data"]["scanner_status"], "scanning");

    let requests = requests.lock().await;
    assert_eq!(requests[0].0, "GET");
    assert_eq!(requests[0].1, "/api/v1/devices/Device%20A");
    assert_eq!(requests[1].1, "/api/v1/devices");
    assert_eq!(requests[2].1, "/api/v1/diagnostics");
}

#[tokio::test]
async fn control_client_posts_only_fixed_actions_with_idempotency_key() {
    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    client
        .submit_command("managed A", RemoteAction::Open, "open-1")
        .await
        .unwrap();
    client.submit_toggle("managed A", "toggle-1").await.unwrap();
    client.get_command("cmd-123").await.unwrap();

    let requests = requests.lock().await;
    assert_eq!(requests[0].0, "POST");
    assert_eq!(
        requests[0].1,
        "/api/control/v1/devices/managed%20A/commands"
    );
    assert_eq!(requests[0].2.get("idempotency-key").unwrap(), "open-1");
    assert_eq!(requests[0].3, json!({"action": "open"}));
    assert_eq!(requests[1].0, "POST");
    assert_eq!(requests[1].1, "/api/control/v1/devices/managed%20A/toggle");
    assert_eq!(requests[1].2.get("idempotency-key").unwrap(), "toggle-1");
    assert_eq!(requests[2].0, "GET");
    assert_eq!(requests[2].1, "/api/control/v1/commands/cmd-123");
}

#[tokio::test]
async fn enrollment_client_matches_control_api_and_handles_no_content_removal() {
    let requests: Requests = Arc::default();
    let client = test_client(requests.clone()).await;
    client.get_managed_devices().await.unwrap();
    client
        .enroll_device("observed-1", "Bedroom Right")
        .await
        .unwrap();
    client.remove_enrollment("managed-1").await.unwrap();

    let requests = requests.lock().await;
    assert_eq!(requests[0].0, "GET");
    assert_eq!(requests[0].1, "/api/control/v1/devices");
    assert_eq!(requests[1].0, "POST");
    assert_eq!(requests[1].1, "/api/control/v1/devices");
    assert_eq!(
        requests[1].3,
        json!({"observed_device_id": "observed-1", "label": "Bedroom Right"})
    );
    assert_eq!(requests[2].0, "DELETE");
    assert_eq!(requests[2].1, "/api/control/v1/devices/managed-1");
}

#[tokio::test]
async fn client_rejects_unsafe_ids_and_invalid_idempotency_keys_before_sending() {
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
            .submit_command("blind-1", RemoteAction::Stop, "")
            .await
            .is_err()
    );
    assert!(
        client
            .submit_toggle("blind-1", &"x".repeat(129))
            .await
            .is_err()
    );
    assert!(requests.lock().await.is_empty());
}

async fn test_client(requests: Requests) -> RemoteApiClient {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_router(requests)).await.unwrap();
    });
    RemoteApiClient::new(&format!("http://{address}")).unwrap()
}
