//! Mock server source generation for Rust e2e tests.

use std::fmt::Write as FmtWrite;

use alef_core::hash::{self, CommentStyle};

use crate::config::E2eConfig;
use crate::escape::rust_raw_string;
use crate::fixture::Fixture;

/// Emit mock server setup lines into a test function body.
///
/// Builds `MockRoute` objects from the fixture's `mock_response` (single-response schema)
/// or `input.mock_responses` (array schema for multiple responses per fixture).
/// The resulting `mock_server` variable is in scope for the rest of the test function.
pub fn render_mock_server_setup(out: &mut String, fixture: &Fixture, e2e_config: &E2eConfig) {
    // Try array schema first: input.mock_responses
    let mut routes = Vec::new();

    if let Some(mock_responses) = fixture.input.get("mock_responses").and_then(|v| v.as_array()) {
        // Array schema: input.mock_responses[{ path, status_code, headers, body_inline, ... }]
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        let default_path = call_config.path.as_deref().unwrap_or("/");
        let default_method = call_config.method.as_deref().unwrap_or("POST");

        for response in mock_responses {
            if let Ok(obj) = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(response.clone()) {
                let path = obj
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(default_path)
                    .to_string();
                let method = obj
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or(default_method)
                    .to_string();
                let status: u16 = obj.get("status_code").and_then(|v| v.as_u64()).unwrap_or(200) as u16;

                let headers: Vec<(String, String)> = obj
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|h| {
                        let mut entries: Vec<_> = h
                            .iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                            .collect();
                        entries.sort_by(|a, b| a.0.cmp(&b.0));
                        entries
                    })
                    .unwrap_or_default();

                let body_str = if let Some(body_inline) = obj.get("body_inline").and_then(|v| v.as_str()) {
                    rust_raw_string(body_inline)
                } else {
                    // Note: body_file support would require fixture-dir context at codegen time.
                    // For now, we emit a placeholder; the standalone binary handles body_file.
                    rust_raw_string("{}")
                };

                let delay_ms = obj.get("delay_ms").and_then(|v| v.as_u64());

                routes.push((path, method, status, body_str, headers, delay_ms));
            }
        }
    } else if let Some(mock) = fixture.mock_response.as_ref() {
        // Single-response schema: mock_response { status, body, stream_chunks, headers }
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        let path = call_config.path.as_deref().unwrap_or("/");
        let method = call_config.method.as_deref().unwrap_or("POST");

        let status = mock.status;

        // Render headers map as a Vec<(String, String)> literal for stable iteration order.
        let mut header_entries: Vec<(&String, &String)> = mock.headers.iter().collect();
        header_entries.sort_by(|a, b| a.0.cmp(b.0));
        let header_tuples: Vec<(String, String)> = header_entries
            .into_iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let body_str = match &mock.body {
            Some(b) => {
                let s = serde_json::to_string(b).unwrap_or_default();
                rust_raw_string(&s)
            }
            None => rust_raw_string("{}"),
        };

        // Handle streaming separately within the single-response case.
        if let Some(chunks) = &mock.stream_chunks {
            // Streaming SSE response.
            let _ = writeln!(out, "    let mock_route = MockRoute {{");
            let _ = writeln!(out, "        path: \"{path}\",");
            let _ = writeln!(out, "        method: \"{method}\",");
            let _ = writeln!(out, "        status: {status},");
            let _ = writeln!(out, "        body: String::new(),");
            let _ = writeln!(out, "        is_streaming: true,");
            let _ = writeln!(out, "        stream_chunks: vec![");
            for chunk in chunks {
                let chunk_str = match chunk {
                    serde_json::Value::String(s) => rust_raw_string(s),
                    other => {
                        let s = serde_json::to_string(other).unwrap_or_default();
                        rust_raw_string(&s)
                    }
                };
                let _ = writeln!(out, "            {chunk_str}.to_string(),");
            }
            let _ = writeln!(out, "        ],");
            let _ = writeln!(out, "        headers: vec![");
            for (name, value) in &header_tuples {
                let n = rust_raw_string(name);
                let v = rust_raw_string(value);
                let _ = writeln!(out, "            ({n}.to_string(), {v}.to_string()),");
            }
            let _ = writeln!(out, "        ],");
            let _ = writeln!(out, "        delay_ms: None,");
            let _ = writeln!(out, "    }};");
            let _ = writeln!(out, "    let mock_server = MockServer::start(vec![mock_route]).await;");
            return;
        }

        routes.push((
            path.to_string(),
            method.to_string(),
            status,
            body_str,
            header_tuples,
            None,
        ));
    } else {
        return;
    }

    // Emit all routes (array schema produces multiple; single schema produces one).
    if routes.len() == 1 {
        let (path, method, status, body_str, header_entries, delay_ms) = routes.pop().unwrap();
        let delay_literal = match delay_ms {
            Some(ms) => format!("Some({ms})"),
            None => "None".to_string(),
        };
        let _ = writeln!(out, "    let mock_route = MockRoute {{");
        let _ = writeln!(out, "        path: \"{path}\",");
        let _ = writeln!(out, "        method: \"{method}\",");
        let _ = writeln!(out, "        status: {status},");
        let _ = writeln!(out, "        body: {body_str}.to_string(),");
        let _ = writeln!(out, "        is_streaming: false,");
        let _ = writeln!(out, "        stream_chunks: vec![],");
        let _ = writeln!(out, "        headers: vec![");
        for (name, value) in &header_entries {
            let n = rust_raw_string(name);
            let v = rust_raw_string(value);
            let _ = writeln!(out, "            ({n}.to_string(), {v}.to_string()),");
        }
        let _ = writeln!(out, "        ],");
        let _ = writeln!(out, "        delay_ms: {delay_literal},");
        let _ = writeln!(out, "    }};");
        let _ = writeln!(out, "    let mock_server = MockServer::start(vec![mock_route]).await;");
    } else {
        // Multiple routes from array schema.
        let _ = writeln!(out, "    let mut mock_routes = vec![];");
        for (path, method, status, body_str, header_entries, delay_ms) in routes {
            let delay_literal = match delay_ms {
                Some(ms) => format!("Some({ms})"),
                None => "None".to_string(),
            };
            let _ = writeln!(out, "    mock_routes.push(MockRoute {{");
            let _ = writeln!(out, "        path: \"{path}\",");
            let _ = writeln!(out, "        method: \"{method}\",");
            let _ = writeln!(out, "        status: {status},");
            let _ = writeln!(out, "        body: {body_str}.to_string(),");
            let _ = writeln!(out, "        is_streaming: false,");
            let _ = writeln!(out, "        stream_chunks: vec![],");
            let _ = writeln!(out, "        headers: vec![");
            for (name, value) in &header_entries {
                let n = rust_raw_string(name);
                let v = rust_raw_string(value);
                let _ = writeln!(out, "            ({n}.to_string(), {v}.to_string()),");
            }
            let _ = writeln!(out, "        ],");
            let _ = writeln!(out, "        delay_ms: {delay_literal},");
            let _ = writeln!(out, "    }});");
        }
        let _ = writeln!(out, "    let mock_server = MockServer::start(mock_routes).await;");
    }
}

/// Generate the complete `mock_server.rs` module source.
pub fn render_mock_server_module() -> String {
    // This is parameterized Axum mock server code identical in structure to
    // liter-llm's mock_server.rs but without any project-specific imports.
    //
    // The module is included via `mod mock_server;` in every integration-test
    // binary that needs MockRoute/MockServer, but only fixtures using
    // `MockServer::start` actually invoke `start()` / `handle_request()`. Each
    // integration test compiles as a separate binary, so the unused-in-some
    // helpers would otherwise trip `-D dead_code` under
    // `cargo test`/`cargo clippy`. The crate-level `#![allow(dead_code)]`
    // mirrors the pattern used by other generated helper modules
    // (e.g. `tests/common.rs`).
    hash::header(CommentStyle::DoubleSlash)
        + r#"//
// Minimal axum-based mock HTTP server for e2e tests.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;

/// A single mock route: match by path + method, return a configured response.
#[derive(Clone, Debug)]
pub struct MockRoute {
    /// URL path to match, e.g. `"/v1/chat/completions"`.
    pub path: &'static str,
    /// HTTP method to match, e.g. `"POST"` or `"GET"`.
    pub method: &'static str,
    /// HTTP status code to return.
    pub status: u16,
    /// Response body JSON string (used when `is_streaming` is false).
    pub body: String,
    /// Whether this route serves a streaming SSE response.
    /// True even when `stream_chunks` is empty (e.g. an empty stream still sends
    /// `data: [DONE]\n\n` with the correct `content-type: text/event-stream` header).
    pub is_streaming: bool,
    /// Ordered SSE data payloads for streaming responses.
    /// Each entry becomes `data: <chunk>\n\n` in the response.
    /// A final `data: [DONE]\n\n` is always appended.
    pub stream_chunks: Vec<String>,
    /// Response headers to apply (name, value) pairs.
    /// Multiple entries with the same name produce multiple header lines.
    pub headers: Vec<(String, String)>,
    /// Optional artificial response delay in milliseconds. Used by timeout-error
    /// fixtures to force a client-side request timeout.
    pub delay_ms: Option<u64>,
}

struct ServerState {
    routes: Vec<MockRoute>,
}

pub struct MockServer {
    /// Base URL of the mock server, e.g. `"http://127.0.0.1:54321"`.
    pub url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    /// Start a mock server with the given routes.  Binds to a random port on
    /// localhost and returns immediately once the server is listening.
    pub async fn start(routes: Vec<MockRoute>) -> Self {
        let state = Arc::new(ServerState { routes });

        let app = Router::new().fallback(handle_request).with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock server port");
        let addr: SocketAddr = listener.local_addr().expect("Failed to get local addr");
        let url = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("Mock server failed");
        });

        MockServer { url, handle }
    }

    /// Stop the mock server.
    pub fn shutdown(self) {
        self.handle.abort();
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_request(State(state): State<Arc<ServerState>>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();
    let method = req.method().as_str().to_uppercase();

    for route in &state.routes {
        // Match on method and either exact path or path prefix (route.path is a prefix of the
        // request path, separated by a '/' boundary). This allows a single route registered at
        // "/v1/batches" to match requests to "/v1/batches/abc123" or
        // "/v1/batches/abc123/cancel".
        let path_matches = path == route.path
            || (path.starts_with(route.path)
                && path.as_bytes().get(route.path.len()) == Some(&b'/'));
        if path_matches && route.method.to_uppercase() == method {
            if let Some(delay_ms) = route.delay_ms {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            let status =
                StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            if route.is_streaming {
                // Build SSE body: data: <chunk>\n\n ... data: [DONE]\n\n
                // Note: stream_chunks may be empty for an empty-stream fixture; we still
                // emit `data: [DONE]\n\n` with the correct SSE headers so clients do not hang.
                let mut sse = String::new();
                for chunk in &route.stream_chunks {
                    sse.push_str("data: ");
                    sse.push_str(chunk);
                    sse.push_str("\n\n");
                }
                sse.push_str("data: [DONE]\n\n");

                let mut builder = Response::builder()
                    .status(status)
                    .header("content-type", "text/event-stream")
                    .header("cache-control", "no-cache");
                for (name, value) in &route.headers {
                    builder = builder.header(name, value);
                }
                return builder.body(Body::from(sse)).unwrap().into_response();
            }

            let mut builder =
                Response::builder().status(status).header("content-type", "application/json");
            for (name, value) in &route.headers {
                builder = builder.header(name, value);
            }
            return builder.body(Body::from(route.body.clone())).unwrap().into_response();
        }
    }

    // No matching route → 404.
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!("No mock route for {method} {path}")))
        .unwrap()
        .into_response()
}
"#
}

/// Generate the `tests/common.rs` module for Rust e2e tests.
///
/// The module spawns the standalone mock-server binary once per test process
/// and exposes it via `mock_server_url()` which reads the `MOCK_SERVER_URL` env var.
/// This allows tests that use mock_url arguments to access the server dynamically
/// without panicking on unset env vars.
///
/// The module:
/// - Spawns `target/release/mock-server` with the fixtures directory as an argument
/// - Reads stdout lines looking for `MOCK_SERVER_URL=http://...` and `MOCK_SERVERS={...}`
/// - Sets environment variables: `MOCK_SERVER_URL` and `MOCK_SERVER_<FIXTURE_ID>` for each entry
/// - Drains remaining stdout in a background thread to prevent blocking
/// - Uses `OnceLock` to ensure the server is spawned exactly once
pub fn render_common_module() -> String {
    // The module is included via `mod common;` in every integration-test
    // binary, but only fixtures that resolve `mock_url` arguments actually
    // call `mock_server_url()` / touch `MOCK_SERVER_URL`. Each integration
    // test compiles as a separate binary, so the unused-in-some symbols
    // would otherwise trip `-D dead_code` under `cargo test`/`cargo clippy`.
    // The crate-level `#![allow(dead_code)]` mirrors the pattern used in
    // `tests/mock_server.rs`.
    hash::header(CommentStyle::DoubleSlash)
        + r#"//
// Auto-spawned mock server setup for e2e tests.
// This module is auto-generated and should not be edited manually.

#![allow(dead_code)]

use std::sync::OnceLock;

static MOCK_SERVER_URL: OnceLock<String> = OnceLock::new();

/// Get the mock server URL, spawning the server if not already running.
///
/// The server is spawned once per test process and reused by all tests.
/// On first call, this function:
/// - Spawns the `target/release/mock-server` binary
/// - Reads `MOCK_SERVER_URL=http://...` from its stdout
/// - Parses `MOCK_SERVERS={...}` JSON and sets env vars for per-fixture servers
/// - Sets `MOCK_SERVER_URL` env var globally
/// - Drains remaining stdout in a background thread
///
/// Subsequent calls return the cached URL without spawning again.
pub fn mock_server_url() -> &'static str {
    MOCK_SERVER_URL.get_or_init(|| {
        let mock_server_bin = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/target/release/mock-server"
        );
        let fixtures_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures"
        );

        // Spawn the mock-server binary with fixtures directory as argument.
        let mut child = std::process::Command::new(mock_server_bin)
            .arg(fixtures_dir)
            .stdout(std::process::Stdio::piped())
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn mock-server binary");

        let stdout = child.stdout.take().expect("Failed to get stdout");
        let stdin = child.stdin.take().expect("Failed to get stdin");

        let mut url = String::new();
        let mut line_buffer = String::new();
        let mut line_count = 0;

        // Read startup lines from the mock server.
        // Expected: MOCK_SERVER_URL=http://... then MOCK_SERVERS={...json...}
        // The server prints one line per loaded fixture before the markers, so the
        // ceiling has to be high enough to clear hundreds of "loaded route" lines.
        // We bail on the first `MOCK_SERVERS=` line (always emitted last) rather than
        // relying on the line cap.
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stdout);

        while line_count < 2048 {
            line_buffer.clear();
            match reader.read_line(&mut line_buffer) {
                Ok(0) => break,  // EOF
                Ok(_) => {
                    let line = line_buffer.trim();
                    if line.starts_with("MOCK_SERVER_URL=") {
                        url = line.strip_prefix("MOCK_SERVER_URL=")
                            .unwrap_or("")
                            .to_string();
                    } else if line.starts_with("MOCK_SERVERS=") {
                        let json_str = line.strip_prefix("MOCK_SERVERS=")
                            .unwrap_or("{}");
                        // Parse the JSON map and set env vars for each entry.
                        if let Ok(servers) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json_str) {
                            for (fid, furl) in servers {
                                if let serde_json::Value::String(url_str) = furl {
                                    let env_key = format!("MOCK_SERVER_{}", fid.to_uppercase());
                                    std::env::set_var(&env_key, &url_str);
                                }
                            }
                        }
                        std::env::set_var("MOCK_SERVERS", json_str);
                        // We have seen both lines; stop reading.
                        break;
                    }
                    line_count += 1;
                }
                Err(_) => break,
            }
        }

        // Set the main URL env var globally.
        std::env::set_var("MOCK_SERVER_URL", &url);

        // Drain remaining stdout in a background thread to prevent the server from blocking.
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader.into_inner(), &mut std::io::sink());
        });

        // Keep stdin alive for the test process lifetime — the mock-server treats
        // stdin EOF as the parent's shutdown signal, so dropping the handle would
        // make it exit before any test connects.
        Box::leak(Box::new(stdin));

        // Return the URL for this process.
        url
    }).as_str()
}
"#
}

/// Generate the `src/main.rs` for the standalone mock server binary.
///
/// The binary:
/// - Reads all `*.json` fixture files from a fixtures directory (default `../../fixtures`).
/// - For each fixture that has a `mock_response` field, registers a route at
///   `/fixtures/{fixture_id}` returning the configured status/body/SSE chunks.
/// - Binds to `127.0.0.1:0` (random port), prints `MOCK_SERVER_URL=http://...`
///   to stdout, then waits until stdin is closed for clean teardown.
/// - Fixtures that declare host-root paths (`/robots.txt`, `/sitemap.*`, etc.) get their
///   own dedicated listener so the crawler can fetch them from the host root.  A second
///   line `MOCK_SERVERS={...}` (sorted JSON object) maps fixture_id → base URL for those.
///
/// This binary is intended for cross-language e2e suites (WASM, Node) that
/// spawn it as a child process and read the URL from its stdout.
pub fn render_mock_server_binary() -> String {
    hash::header(CommentStyle::DoubleSlash)
        + r#"//
// Standalone mock HTTP server binary for cross-language e2e tests.
// Reads fixture JSON files and serves mock responses on /fixtures/{fixture_id}.
//
// Usage: mock-server [fixtures-dir]
//   fixtures-dir defaults to "../../fixtures"
//
// Prints `MOCK_SERVER_URL=http://127.0.0.1:<port>` to stdout once listening,
// then optionally `MOCK_SERVERS={...}` with per-fixture URLs for host-root fixtures,
// then blocks until stdin is closed (parent process exit triggers cleanup).

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Fixture types (mirrors alef-e2e's fixture.rs for runtime deserialization)
// Supports both schemas:
//   liter-llm: mock_response: { status, body, stream_chunks }
//   spikard:   http.expected_response: { status_code, body, headers }
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MockResponse {
    status: u16,
    #[serde(default)]
    body: Option<serde_json::Value>,
    #[serde(default)]
    stream_chunks: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    headers: HashMap<String, String>,
    /// Optional artificial delay (milliseconds) applied before sending the response.
    /// Used by timeout-error fixtures to force the client request to time out.
    #[serde(default)]
    delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HttpExpectedResponse {
    status_code: u16,
    #[serde(default)]
    body: Option<serde_json::Value>,
    #[serde(default)]
    headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct HttpFixture {
    expected_response: HttpExpectedResponse,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    #[serde(default)]
    mock_response: Option<MockResponse>,
    #[serde(default)]
    http: Option<HttpFixture>,
    /// Array-form fixture schema. `input.mock_responses[i] = { path?, status_code, headers, body_inline | body_file }`.
    /// Used by kreuzcrawl-style fixtures that mock multiple URLs per fixture (e.g. a page +
    /// `/robots.txt` + `/sitemap.xml`).
    #[serde(default)]
    input: Option<serde_json::Value>,
}

/// A single resolved mock response with its serving path and whether it is a host-root path.
struct ResolvedRoute {
    /// The namespaced path under which this route is registered in the shared server,
    /// e.g. `/fixtures/robots_disallow_path` or `/fixtures/robots_disallow_path/assets/style.css`.
    path: String,
    /// The original fixture-declared path (before namespacing), e.g. `/robots.txt` or `/assets/style.css`.
    original_path: String,
    response: MockResponse,
    /// Body bytes (pre-loaded from body_file or body_inline).
    body_bytes: Vec<u8>,
}

/// Returns true for paths that the crawler fetches from the host root rather than
/// under a fixture-namespaced prefix.  These require a dedicated per-fixture listener.
fn is_host_root_path(path: &str) -> bool {
    path.starts_with("/robots") || path.starts_with("/sitemap")
}

impl Fixture {
    /// Bridge both schemas into a unified MockResponse.
    fn as_mock_response(&self) -> Option<MockResponse> {
        if let Some(mock) = &self.mock_response {
            return Some(MockResponse {
                status: mock.status,
                body: mock.body.clone(),
                stream_chunks: mock.stream_chunks.clone(),
                headers: mock.headers.clone(),
                delay_ms: mock.delay_ms,
            });
        }
        if let Some(http) = &self.http {
            return Some(MockResponse {
                status: http.expected_response.status_code,
                body: http.expected_response.body.clone(),
                stream_chunks: None,
                headers: http.expected_response.headers.clone(),
                delay_ms: None,
            });
        }
        None
    }

    /// Resolve every mock response this fixture defines.
    ///
    /// Returns single-element output for the legacy `mock_response` / `http` schemas, and one
    /// element per array entry for the kreuzcrawl-style `input.mock_responses` schema. For the
    /// array schema, each element may declare its own `path` (defaulting to `/fixtures/{id}`),
    /// and the body source can be either `body_inline` (string) or `body_file` (path relative
    /// to the fixtures dir, loaded at startup).
    ///
    /// Paths that are NOT host-root paths (e.g. `/robots.txt`, `/sitemap.xml`) are namespaced
    /// under `/fixtures/{id}` so that fixtures sharing common paths (like `/`, `/a`, `/b`) do
    /// not collide in the shared route table.
    fn as_routes(&self, fixtures_dir: &Path) -> Vec<ResolvedRoute> {
        let mut routes = Vec::new();
        let default_path = format!("/fixtures/{}", self.id);

        if let Some(mock) = self.as_mock_response() {
            let body_bytes = mock
                .body
                .as_ref()
                .map(|b| match b {
                    serde_json::Value::String(s) => s.as_bytes().to_vec(),
                    other => serde_json::to_string(other).unwrap_or_default().into_bytes(),
                })
                .unwrap_or_default();
            routes.push(ResolvedRoute {
                path: default_path.clone(),
                original_path: default_path.clone(),
                response: mock,
                body_bytes,
            });
        }

        if let Some(input) = &self.input {
            if let Some(arr) = input.get("mock_responses").and_then(|v| v.as_array()) {
                for entry in arr {
                    let original_path = entry
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("/")
                        .to_string();

                    // Namespace under /fixtures/<id> unless this is a host-root path.
                    let namespaced_path = if is_host_root_path(&original_path) {
                        original_path.clone()
                    } else if original_path == "/" {
                        format!("/fixtures/{}", self.id)
                    } else {
                        format!("/fixtures/{}{}", self.id, original_path)
                    };

                    let status: u16 = entry.get("status_code").and_then(|v| v.as_u64()).unwrap_or(200) as u16;
                    let headers: HashMap<String, String> = entry
                        .get("headers")
                        .and_then(|v| v.as_object())
                        .map(|h| {
                            h.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();

                    // Load body bytes — use read() not read_to_string() to support binary files.
                    let body_bytes: Vec<u8> = if let Some(inline) = entry.get("body_inline").and_then(|v| v.as_str()) {
                        inline.as_bytes().to_vec()
                    } else if let Some(file) = entry.get("body_file").and_then(|v| v.as_str()) {
                        // body_file is resolved relative to `<fixtures>/responses/` first,
                        // falling back to `<fixtures>/` for projects that store body assets at
                        // the fixtures root rather than under a `responses/` subdir.
                        let candidates = [fixtures_dir.join("responses").join(file), fixtures_dir.join(file)];
                        let mut loaded = None;
                        for abs in &candidates {
                            if let Ok(bytes) = std::fs::read(abs) {
                                loaded = Some(bytes);
                                break;
                            }
                        }
                        match loaded {
                            Some(b) => b,
                            None => {
                                eprintln!(
                                    "warning: cannot read body_file {} (tried {} and {})",
                                    file,
                                    candidates[0].display(),
                                    candidates[1].display()
                                );
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };

                    let delay_ms = entry.get("delay_ms").and_then(|v| v.as_u64());

                    routes.push(ResolvedRoute {
                        path: namespaced_path,
                        original_path,
                        response: MockResponse {
                            status,
                            body: None,
                            stream_chunks: None,
                            headers,
                            delay_ms,
                        },
                        body_bytes,
                    });
                }
            }
        }

        routes
    }
}

// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MockRoute {
    status: u16,
    body: Vec<u8>,
    /// Whether this route serves a streaming SSE response.
    /// True even when `stream_chunks` is empty (e.g. an empty stream still sends
    /// `data: [DONE]\n\n` with the correct `content-type: text/event-stream` header).
    is_streaming: bool,
    stream_chunks: Vec<String>,
    headers: Vec<(String, String)>,
    /// Optional artificial delay applied before the handler returns. When set,
    /// the handler `tokio::time::sleep`s for this many milliseconds before
    /// constructing the response — used by timeout-error fixtures to force
    /// client-side request timeouts.
    delay_ms: Option<u64>,
}

type RouteTable = Arc<HashMap<String, MockRoute>>;

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_request(State(routes): State<RouteTable>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();

    // Try exact match first
    if let Some(route) = routes.get(&path) {
        if let Some(delay_ms) = route.delay_ms {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        return serve_route(route);
    }

    // Try prefix match: find a route that is a prefix of the request path
    // This allows /fixtures/basic_chat/v1/chat/completions to match /fixtures/basic_chat
    for (route_path, route) in routes.iter() {
        if path.starts_with(route_path) && (path.len() == route_path.len() || path.as_bytes()[route_path.len()] == b'/') {
            if let Some(delay_ms) = route.delay_ms {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            return serve_route(route);
        }
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!("No mock route for {path}")))
        .unwrap()
        .into_response()
}

fn serve_route(route: &MockRoute) -> Response {
    let status = StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if route.is_streaming {
        // Note: stream_chunks may be empty for an empty-stream fixture; we still emit
        // `data: [DONE]\n\n` with the correct SSE headers so clients do not hang.
        let mut sse = String::new();
        for chunk in &route.stream_chunks {
            sse.push_str("data: ");
            sse.push_str(chunk);
            sse.push_str("\n\n");
        }
        sse.push_str("data: [DONE]\n\n");

        let mut builder = Response::builder()
            .status(status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache");
        for (name, value) in &route.headers {
            builder = builder.header(name, value);
        }
        return builder.body(Body::from(sse)).unwrap().into_response();
    }

    // Only set the default content-type if the fixture does not override it.
    // Inspect the first non-whitespace byte to detect JSON vs binary vs plain text.
    let has_content_type = route.headers.iter().any(|(k, _)| k.to_lowercase() == "content-type");
    let mut builder = Response::builder().status(status);
    if !has_content_type {
        let first_nonws = route.body.iter().find(|&&b| b != b' ' && b != b'\t' && b != b'\n' && b != b'\r');
        let default_ct = match first_nonws {
            Some(&b'{') | Some(&b'[') => "application/json",
            _ => "text/plain",
        };
        builder = builder.header("content-type", default_ct);
    }
    for (name, value) in &route.headers {
        // Skip content-encoding headers — the mock server returns uncompressed bodies.
        // Sending a content-encoding without actually encoding the body would cause
        // clients to fail decompression.
        if name.to_lowercase() == "content-encoding" {
            continue;
        }
        // The <<absent>> sentinel means this header must NOT be present in the
        // real server response — do not emit it from the mock server either.
        if value == "<<absent>>" {
            continue;
        }
        // Replace the <<uuid>> sentinel with a real UUID v4 so clients can
        // assert the header value matches the UUID pattern.
        if value == "<<uuid>>" {
            let uuid = format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                rand_u32(),
                rand_u16(),
                rand_u16() & 0x0fff,
                (rand_u16() & 0x3fff) | 0x8000,
                rand_u48(),
            );
            builder = builder.header(name, uuid);
            continue;
        }
        builder = builder.header(name, value);
    }
    builder.body(Body::from(route.body.clone())).unwrap().into_response()
}

/// Generate a pseudo-random u32 using the current time nanoseconds.
fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    ns ^ (ns.wrapping_shl(13)) ^ (ns.wrapping_shr(17))
}

fn rand_u16() -> u16 {
    (rand_u32() & 0xffff) as u16
}

fn rand_u48() -> u64 {
    ((rand_u32() as u64) << 16) | (rand_u16() as u64)
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

/// Intermediate fixture-loading result: shared route table plus per-fixture host-root data.
struct LoadedRoutes {
    /// Routes namespaced under /fixtures/<id> for the shared listener.
    shared: HashMap<String, MockRoute>,
    /// For each fixture that has host-root routes: fixture_id → route table at host root.
    per_fixture: HashMap<String, HashMap<String, MockRoute>>,
}

fn load_routes(fixtures_dir: &Path) -> LoadedRoutes {
    let mut shared = HashMap::new();
    let mut per_fixture: HashMap<String, HashMap<String, MockRoute>> = HashMap::new();
    load_routes_recursive(fixtures_dir, fixtures_dir, &mut shared, &mut per_fixture);
    LoadedRoutes { shared, per_fixture }
}

fn load_routes_recursive(
    dir: &Path,
    fixtures_root: &Path,
    shared: &mut HashMap<String, MockRoute>,
    per_fixture: &mut HashMap<String, HashMap<String, MockRoute>>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("warning: cannot read directory {}: {err}", dir.display());
            return;
        }
    };

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_routes_recursive(&path, fixtures_root, shared, per_fixture);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("warning: cannot read {}: {err}", path.display());
                    continue;
                }
            };
            let fixtures: Vec<Fixture> = if content.trim_start().starts_with('[') {
                match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            } else {
                match serde_json::from_str::<Fixture>(&content) {
                    Ok(f) => vec![f],
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            };

            for fixture in fixtures {
                let resolved_routes = fixture.as_routes(fixtures_root);
                // A fixture needs host-root routing if either:
                //  1. it serves a path the crawler fetches at host root (/robots*, /sitemap*), OR
                //  2. it returns a 3xx Location header pointing to a host-root path inside the
                //     same fixture (the engine resolves the Location against the host, not the
                //     /fixtures/<id>/ namespace, so host-root serving is required for the
                //     follow-up GET to hit the correct route).
                let has_intra_fixture_redirect = resolved_routes.iter().any(|r| {
                    // 3xx with relative Location header
                    let location_redirect = (300..400).contains(&r.response.status)
                        && r.response.headers.iter().any(|(name, value)| {
                            name.eq_ignore_ascii_case("location") && value.starts_with('/')
                        });
                    // Refresh header with url=/...
                    let refresh_redirect = r.response.headers.iter().any(|(name, value)| {
                        if !name.eq_ignore_ascii_case("refresh") {
                            return false;
                        }
                        let lower = value.to_ascii_lowercase();
                        lower
                            .find("url=")
                            .map(|idx| value[idx + 4..].trim_start().starts_with('/'))
                            .unwrap_or(false)
                    });
                    // HTML meta-refresh tag pointing to /...
                    let body_lower_lossy = String::from_utf8_lossy(&r.body_bytes).to_ascii_lowercase();
                    let meta_refresh = body_lower_lossy
                        .split("http-equiv=\"refresh\"")
                        .nth(1)
                        .and_then(|s| s.split("content=").nth(1))
                        .map(|s| {
                            let trimmed = s.trim_start_matches(['"', '\'']);
                            trimmed.contains("url=/")
                        })
                        .unwrap_or(false);
                    location_redirect || refresh_redirect || meta_refresh
                });
                let has_host_root = has_intra_fixture_redirect
                    || resolved_routes.iter().any(|r| is_host_root_path(&r.original_path));

                for resolved in resolved_routes {
                    let is_streaming = resolved.response.stream_chunks.is_some();
                    let stream_chunks = resolved.response
                        .stream_chunks
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| match c {
                            serde_json::Value::String(s) => s,
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        })
                        .collect();
                    let mut headers: Vec<(String, String)> = resolved.response.headers.into_iter().collect();
                    headers.sort_by(|a, b| a.0.cmp(&b.0));

                    let mock_route = MockRoute {
                        status: resolved.response.status,
                        body: resolved.body_bytes,
                        is_streaming,
                        stream_chunks,
                        headers,
                        delay_ms: resolved.response.delay_ms,
                    };

                    // Always insert into the shared namespaced table.
                    shared.insert(resolved.path.clone(), mock_route.clone());

                    // For fixtures with host-root routes, also build a per-fixture table
                    // where routes are mounted at their original (un-namespaced) paths.
                    if has_host_root {
                        per_fixture
                            .entry(fixture.id.clone())
                            .or_default()
                            .insert(resolved.original_path.clone(), mock_route);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let fixtures_dir_arg = std::env::args().nth(1).unwrap_or_else(|| "../../fixtures".to_string());
    let fixtures_dir = Path::new(&fixtures_dir_arg);

    let loaded = load_routes(fixtures_dir);
    eprintln!("mock-server: loaded {} shared routes from {}", loaded.shared.len(), fixtures_dir.display());

    // Shared namespaced server.
    let shared_table: RouteTable = Arc::new(loaded.shared);
    let shared_app = Router::new().fallback(handle_request).with_state(shared_table);

    let shared_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock-server: failed to bind shared port");
    let shared_addr: SocketAddr = shared_listener.local_addr().expect("mock-server: failed to get shared local addr");

    // Per-fixture listeners for host-root routes (robots.txt, sitemap.xml, etc.).
    // Sorted by fixture_id for deterministic output.
    let mut fixture_ids: Vec<String> = loaded.per_fixture.keys().cloned().collect();
    fixture_ids.sort();

    let mut fixture_urls: HashMap<String, String> = HashMap::new();
    for fixture_id in &fixture_ids {
        let routes = loaded.per_fixture[fixture_id].clone();
        eprintln!("mock-server: fixture {} has {} host-root routes", fixture_id, routes.len());
        let table: RouteTable = Arc::new(routes);
        let app = Router::new().fallback(handle_request).with_state(table);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock-server: failed to bind per-fixture port");
        let addr: SocketAddr = listener.local_addr().expect("mock-server: failed to get per-fixture local addr");
        fixture_urls.insert(fixture_id.clone(), format!("http://{addr}"));
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock-server: per-fixture server error");
        });
    }

    // Print the shared URL so the parent process can read it.
    println!("MOCK_SERVER_URL=http://{shared_addr}");

    // Always print MOCK_SERVERS=... (empty `{}` when there are no host-root
    // fixtures) so parent parsers — which read until they see this sentinel
    // line — never block on a readline that never comes.
    let mut sorted_pairs: Vec<(&String, &String)> = fixture_urls.iter().collect();
    sorted_pairs.sort_by_key(|(k, _)| k.as_str());
    let json_entries: Vec<String> = sorted_pairs
        .iter()
        .map(|(k, v)| format!("\"{}\":\"{}\"", k, v))
        .collect();
    println!("MOCK_SERVERS={{{}}}", json_entries.join(","));

    // Flush stdout explicitly so the parent does not block waiting.
    use std::io::Write;
    std::io::stdout().flush().expect("mock-server: failed to flush stdout");

    // Spawn the shared server in the background.
    tokio::spawn(async move {
        axum::serve(shared_listener, shared_app).await.expect("mock-server: shared server error");
    });

    // Block until stdin is closed — the parent process controls lifetime.
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while lines.next().is_some() {}
}
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_mock_server_module_contains_struct_definition() {
        let out = render_mock_server_module();
        assert!(out.contains("pub struct MockRoute"));
        assert!(out.contains("pub struct MockServer"));
    }

    #[test]
    fn render_mock_server_binary_contains_main() {
        let out = render_mock_server_binary();
        assert!(out.contains("async fn main()"));
        assert!(out.contains("MOCK_SERVER_URL=http://"));
    }

    #[test]
    fn render_common_module_has_expected_symbols() {
        let src = render_common_module();
        assert!(src.contains("pub fn mock_server_url"), "missing mock_server_url");
        assert!(src.contains("OnceLock"), "missing OnceLock");
        assert!(src.contains("MOCK_SERVER_URL"), "missing MOCK_SERVER_URL");
        assert!(src.contains("MOCK_SERVERS"), "missing MOCK_SERVERS");
        assert!(src.contains("serde_json"), "missing serde_json parsing");
    }
}
