//! Local stdio MCP adapter for the Somaris HTTP API.
//!
//! Keeping this adapter as an HTTP client means MCP and HTTP requests use the
//! same running service, BLE adapter, command queue, and durable journal.

use std::{collections::HashMap, env, sync::Arc};

use reqwest::{Client, Method};
use rmcp::{
    RoleServer,
    model::{
        ListResourcesResult, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
        Resource, ResourceContents, ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
};
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::Parameters, schemars, tool, tool_handler,
    tool_router, transport::stdio,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use url::Url;

const CONDITIONAL_CACHE_LIMIT: usize = 128;

#[derive(Clone)]
struct SomarisMcp {
    api_base: String,
    client: Client,
    conditional_cache: Arc<Mutex<HashMap<String, (String, String)>>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdArgs {
    /// Runtime device, enrollment, command, or batch ID.
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HistoryArgs {
    /// Exclusive history cursor; defaults to zero.
    after: Option<u64>,
    /// Page size from 1 to 1000; defaults to 500.
    limit: Option<u64>,
    /// RFC3339 lower time bound.
    since: Option<String>,
    /// Runtime device ID or logical ID.
    device_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EnrollmentArgs {
    /// Runtime ID from list_devices.
    observed_device_id: String,
    /// User-facing label, from 1 to 64 characters.
    label: String,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CommandAction {
    Open,
    Close,
    Stop,
}

impl CommandAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Close => "close",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CommandArgs {
    /// Managed-device enrollment ID.
    managed_device_id: String,
    /// Fixed supported action.
    action: CommandAction,
    /// Caller-supplied key. Reuse it only when retrying the same request.
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToggleArgs {
    /// Managed-device enrollment ID.
    managed_device_id: String,
    /// Caller-supplied key. Reuse it only when retrying the same request.
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BatchArgs {
    /// Fixed supported action applied to the ready enrolled devices.
    action: CommandAction,
    /// Caller-supplied key. Reuse it only when retrying the same request.
    idempotency_key: String,
}

impl SomarisMcp {
    fn new(api_base: String) -> Self {
        Self {
            api_base,
            client: Client::new(),
            conditional_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Url, String> {
        let base = format!("{}/", self.api_base.trim_end_matches('/'));
        let mut url = Url::parse(&base).map_err(|error| format!("Invalid API URL: {error}"))?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| "The API URL cannot be used as a base URL.".to_owned())?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    async fn call(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
        idempotency_key: Option<&str>,
    ) -> String {
        let url = match self.url(segments, query) {
            Ok(url) => url,
            Err(error) => return error,
        };
        let is_get = method == Method::GET;
        let cache_key = url.as_str().to_owned();
        let mut request = self.client.request(method, url);
        let cached_etag = if is_get {
            self.conditional_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|(etag, _)| etag.clone())
        } else {
            None
        };
        if let Some(etag) = cached_etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status == reqwest::StatusCode::NOT_MODIFIED {
                    return self
                        .conditional_cache
                        .lock()
                        .await
                        .get(&cache_key)
                        .map(|(_, response)| response.clone())
                        .unwrap_or_else(|| "HTTP 304 without a cached response".into());
                }
                let etag = response
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                match response.text().await {
                    Ok(body) => {
                        let value = format!("HTTP {status}\n{body}");
                        if is_get {
                            let mut cache = self.conditional_cache.lock().await;
                            if let Some(etag) = etag {
                                if cache.len() >= CONDITIONAL_CACHE_LIMIT
                                    && !cache.contains_key(&cache_key)
                                {
                                    cache.clear();
                                }
                                cache.insert(cache_key, (etag, value.clone()));
                            } else {
                                cache.remove(&cache_key);
                            }
                        }
                        value
                    }
                    Err(error) => format!("HTTP {status}; could not read response: {error}"),
                }
            }
            Err(error) => format!("Somaris API request failed: {error}"),
        }
    }
}

#[tool_router]
impl SomarisMcp {
    #[tool(description = "Check Somaris HTTP service liveness and Bluetooth scanner status.")]
    async fn health(&self) -> String {
        self.call(Method::GET, &["api", "v1", "health"], &[], None, None)
            .await
    }

    #[tool(description = "Check whether Somaris can currently access its Bluetooth adapter.")]
    async fn readiness(&self) -> String {
        self.call(Method::GET, &["api", "v1", "ready"], &[], None, None)
            .await
    }

    #[tool(
        description = "List recently observed devices. Enroll a fresh device before controlling it."
    )]
    async fn list_devices(&self) -> String {
        self.call(Method::GET, &["api", "v1", "devices"], &[], None, None)
            .await
    }

    #[tool(description = "Get the latest observation for one runtime device ID.")]
    async fn get_device(&self, Parameters(args): Parameters<IdArgs>) -> String {
        self.call(
            Method::GET,
            &["api", "v1", "devices", &args.id],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(
        description = "Read retained telemetry history using an exclusive cursor and optional filters."
    )]
    async fn get_history(&self, Parameters(args): Parameters<HistoryArgs>) -> String {
        let mut query = Vec::new();
        if let Some(value) = args.after {
            query.push(("after", value.to_string()));
        }
        if let Some(value) = args.limit {
            query.push(("limit", value.to_string()));
        }
        if let Some(value) = args.since {
            query.push(("since", value));
        }
        if let Some(value) = args.device_id {
            query.push(("device_id", value));
        }
        let query: Vec<_> = query
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect();
        self.call(Method::GET, &["api", "v1", "history"], &query, None, None)
            .await
    }

    #[tool(description = "List enrolled devices and their current control availability.")]
    async fn list_managed_devices(&self) -> String {
        self.call(
            Method::GET,
            &["api", "control", "v1", "devices"],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(
        description = "Enroll a freshly observed device for control. This changes persistent service state."
    )]
    async fn enroll_device(&self, Parameters(args): Parameters<EnrollmentArgs>) -> String {
        self.call(
            Method::POST,
            &["api", "control", "v1", "devices"],
            &[],
            Some(json!({
                "observed_device_id": args.observed_device_id,
                "label": args.label,
            })),
            None,
        )
        .await
    }

    #[tool(
        description = "Remove a device enrollment. Pending or running commands prevent removal."
    )]
    async fn remove_enrollment(&self, Parameters(args): Parameters<IdArgs>) -> String {
        self.call(
            Method::DELETE,
            &["api", "control", "v1", "devices", &args.id],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(
        description = "Queue an asynchronous open, close, or stop command. Returns a command ID to poll with get_command. Acknowledgement does not confirm movement; check telemetry before retrying a possibly-sent write."
    )]
    async fn submit_command(&self, Parameters(args): Parameters<CommandArgs>) -> String {
        self.call(
            Method::POST,
            &[
                "api",
                "control",
                "v1",
                "devices",
                &args.managed_device_id,
                "commands",
            ],
            &[],
            Some(json!({ "action": args.action.as_str() })),
            Some(&args.idempotency_key),
        )
        .await
    }

    #[tool(
        description = "Queue an asynchronous toggle based on the latest position and command intent. Returns a command ID to poll with get_command."
    )]
    async fn toggle_device(&self, Parameters(args): Parameters<ToggleArgs>) -> String {
        self.call(
            Method::POST,
            &[
                "api",
                "control",
                "v1",
                "devices",
                &args.managed_device_id,
                "toggle",
            ],
            &[],
            None,
            Some(&args.idempotency_key),
        )
        .await
    }

    #[tool(
        description = "Queue an asynchronous fixed action for every currently ready enrolled device. Returns a batch ID to poll with get_batch."
    )]
    async fn submit_batch(&self, Parameters(args): Parameters<BatchArgs>) -> String {
        self.call(
            Method::POST,
            &["api", "control", "v1", "batches"],
            &[],
            Some(json!({ "action": args.action.as_str() })),
            Some(&args.idempotency_key),
        )
        .await
    }

    #[tool(
        description = "Read command lifecycle status, including whether a write was not sent, possibly sent, or acknowledged."
    )]
    async fn get_command(&self, Parameters(args): Parameters<IdArgs>) -> String {
        self.call(
            Method::GET,
            &["api", "control", "v1", "commands", &args.id],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(description = "Read batch progress and per-device command outcomes.")]
    async fn get_batch(&self, Parameters(args): Parameters<IdArgs>) -> String {
        self.call(
            Method::GET,
            &["api", "control", "v1", "batches", &args.id],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(description = "Inspect command queue pressure and configured capacity.")]
    async fn get_queue(&self) -> String {
        self.call(
            Method::GET,
            &["api", "control", "v1", "queue"],
            &[],
            None,
            None,
        )
        .await
    }

    #[tool(description = "Read scanner and device registry diagnostics.")]
    async fn diagnostics(&self) -> String {
        self.call(Method::GET, &["api", "v1", "diagnostics"], &[], None, None)
            .await
    }
}

#[tool_handler]
impl ServerHandler for SomarisMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "Somaris provides telemetry and blind control tools. Control actions require an idempotency key and complete asynchronously.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("somaris://api/openapi.json", "Somaris API contract")
                .with_description("Machine-readable HTTP API contract for this Somaris service.")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        if request.uri != "somaris://api/openapi.json" {
            return Err(rmcp::ErrorData::resource_not_found(
                "Somaris resource was not found.",
                None,
            ));
        }
        let url = self
            .url(&["openapi.json"], &[])
            .map_err(|error| rmcp::ErrorData::internal_error(error, None))?;
        let resource_uri = request.uri;
        let cache_key = url.as_str().to_owned();
        let cached_etag = self
            .conditional_cache
            .lock()
            .await
            .get(&cache_key)
            .map(|(etag, _)| etag.clone());
        let mut request = self.client.get(url);
        if let Some(etag) = cached_etag {
            request = request.header("If-None-Match", etag);
        }
        let response = request
            .send()
            .await
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let cached = self
                .conditional_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|(_, response)| response.clone())
                .and_then(|response| response.split_once('\n').map(|(_, body)| body.to_owned()))
                .ok_or_else(|| {
                    rmcp::ErrorData::internal_error(
                        "API contract returned 304 without a cached body.",
                        None,
                    )
                })?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(cached, resource_uri).with_mime_type("application/json"),
            ])
            .into());
        }
        if !response.status().is_success() {
            return Err(rmcp::ErrorData::internal_error(
                format!(
                    "Somaris API returned {} for its API contract.",
                    response.status()
                ),
                None,
            ));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = response
            .text()
            .await
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        if let Some(etag) = etag {
            let mut cache = self.conditional_cache.lock().await;
            if cache.len() >= CONDITIONAL_CACHE_LIMIT && !cache.contains_key(&cache_key) {
                cache.clear();
            }
            cache.insert(cache_key, (etag, format!("HTTP 200\n{text}")));
        }
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, resource_uri).with_mime_type("application/json"),
        ])
        .into())
    }
}

/// Run the MCP stdio process. The HTTP service must be running separately.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let api_base =
        env::var("SOMARIS_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned());
    let service = SomarisMcp::new(api_base).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SomarisMcp;
    use axum::{
        Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::Response,
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;

    async fn conditional_handler(
        State(calls): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
    ) -> Response {
        calls.fetch_add(1, Ordering::SeqCst);
        if headers
            .get("if-none-match")
            .is_some_and(|value| value == "\"v1\"")
        {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header("etag", "\"v1\"")
                .body(axum::body::Body::empty())
                .unwrap();
        }
        Response::builder()
            .status(StatusCode::OK)
            .header("etag", "\"v1\"")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"data":{"status":"healthy"}}"#))
            .unwrap()
    }

    #[test]
    fn api_urls_encode_ids_and_query_values_as_single_components() {
        let server = SomarisMcp::new("http://localhost:8080".into());
        let url = server
            .url(
                &["api", "control", "v1", "commands", "blind/a b"],
                &[("device_id", "Hallway blind".into())],
            )
            .unwrap();
        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/control/v1/commands/blind%2Fa%20b?device_id=Hallway+blind"
        );
    }

    #[tokio::test]
    async fn mcp_client_revalidates_cached_etag_responses() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/api/v1/diagnostics", get(conditional_handler))
            .with_state(calls.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mcp = SomarisMcp::new(format!("http://{address}"));
        let first = mcp
            .call(
                reqwest::Method::GET,
                &["api", "v1", "diagnostics"],
                &[],
                None,
                None,
            )
            .await;
        let second = mcp
            .call(
                reqwest::Method::GET,
                &["api", "v1", "diagnostics"],
                &[],
                None,
                None,
            )
            .await;
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
    }
}
