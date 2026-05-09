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

                routes.push((path, method, status, body_str, headers));
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
            let _ = writeln!(out, "    }};");
            let _ = writeln!(out, "    let mock_server = MockServer::start(vec![mock_route]).await;");
            return;
        }

        routes.push((path.to_string(), method.to_string(), status, body_str, header_tuples));
    } else {
        return;
    }

    // Emit all routes (array schema produces multiple; single schema produces one).
    if routes.len() == 1 {
        let (path, method, status, body_str, header_entries) = routes.pop().unwrap();
        let _ = writeln!(out, "    let mock_route = MockRoute {{");
        let _ = writeln!(out, "        path: \"{path}\",");
        let _ = writeln!(out, "        method: \"{method}\",");
        let _ = writeln!(out, "        status: {status},");
        let _ = writeln!(out, "        body: {body_str}.to_string(),");
        let _ = writeln!(out, "        stream_chunks: vec![],");
        let _ = writeln!(out, "        headers: vec![");
        for (name, value) in &header_entries {
            let n = rust_raw_string(name);
            let v = rust_raw_string(value);
            let _ = writeln!(out, "            ({n}.to_string(), {v}.to_string()),");
        }
        let _ = writeln!(out, "        ],");
        let _ = writeln!(out, "    }};");
        let _ = writeln!(out, "    let mock_server = MockServer::start(vec![mock_route]).await;");
    } else {
        // Multiple routes from array schema.
        let _ = writeln!(out, "    let mut mock_routes = vec![];");
        for (path, method, status, body_str, header_entries) in routes {
            let _ = writeln!(out, "    mock_routes.push(MockRoute {{");
            let _ = writeln!(out, "        path: \"{path}\",");
            let _ = writeln!(out, "        method: \"{method}\",");
            let _ = writeln!(out, "        status: {status},");
            let _ = writeln!(out, "        body: {body_str}.to_string(),");
            let _ = writeln!(out, "        stream_chunks: vec![],");
            let _ = writeln!(out, "        headers: vec![");
            for (name, value) in &header_entries {
                let n = rust_raw_string(name);
                let v = rust_raw_string(value);
                let _ = writeln!(out, "            ({n}.to_string(), {v}.to_string()),");
            }
            let _ = writeln!(out, "        ],");
            let _ = writeln!(out, "    }});");
        }
        let _ = writeln!(out, "    let mock_server = MockServer::start(mock_routes).await;");
    }
}

/// Generate the complete `mock_server.rs` module source.
pub fn render_mock_server_module() -> String {
    // This is parameterized Axum mock server code identical in structure to
    // liter-llm's mock_server.rs but without any project-specific imports.
    hash::header(CommentStyle::DoubleSlash)
        + r#"//
// Minimal axum-based mock HTTP server for e2e tests.

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
    /// Response body JSON string (used when `stream_chunks` is empty).
    pub body: String,
    /// Ordered SSE data payloads for streaming responses.
    /// Each entry becomes `data: <chunk>\n\n` in the response.
    /// A final `data: [DONE]\n\n` is always appended.
    pub stream_chunks: Vec<String>,
    /// Response headers to apply (name, value) pairs.
    /// Multiple entries with the same name produce multiple header lines.
    pub headers: Vec<(String, String)>,
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
            let status =
                StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            if !route.stream_chunks.is_empty() {
                // Build SSE body: data: <chunk>\n\n ... data: [DONE]\n\n
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

/// Generate the `src/main.rs` for the standalone mock server binary.
///
/// The binary:
/// - Reads all `*.json` fixture files from a fixtures directory (default `../../fixtures`).
/// - For each fixture that has a `mock_response` field, registers a route at
///   `/fixtures/{fixture_id}` returning the configured status/body/SSE chunks.
/// - Binds to `127.0.0.1:0` (random port), prints `MOCK_SERVER_URL=http://...`
///   to stdout, then waits until stdin is closed for clean teardown.
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

/// A single resolved mock response with its serving path.
struct ResolvedRoute {
    path: String,
    response: MockResponse,
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
            });
        }
        if let Some(http) = &self.http {
            return Some(MockResponse {
                status: http.expected_response.status_code,
                body: http.expected_response.body.clone(),
                stream_chunks: None,
                headers: http.expected_response.headers.clone(),
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
    fn as_routes(&self, fixtures_dir: &Path) -> Vec<ResolvedRoute> {
        let mut routes = Vec::new();
        let default_path = format!("/fixtures/{}", self.id);

        if let Some(mock) = self.as_mock_response() {
            routes.push(ResolvedRoute {
                path: default_path.clone(),
                response: mock,
            });
        }

        if let Some(input) = &self.input {
            if let Some(arr) = input.get("mock_responses").and_then(|v| v.as_array()) {
                for entry in arr {
                    let path = entry
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| default_path.clone());
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
                    let body: Option<serde_json::Value> = if let Some(inline) = entry.get("body_inline") {
                        Some(inline.clone())
                    } else if let Some(file) = entry.get("body_file").and_then(|v| v.as_str()) {
                        // body_file is resolved relative to `<fixtures>/responses/` first,
                        // falling back to `<fixtures>/` for projects that store body assets at
                        // the fixtures root rather than under a `responses/` subdir.
                        let candidates = [fixtures_dir.join("responses").join(file), fixtures_dir.join(file)];
                        let mut loaded = None;
                        for abs in &candidates {
                            if let Ok(s) = std::fs::read_to_string(abs) {
                                loaded = Some(s);
                                break;
                            }
                        }
                        match loaded {
                            Some(s) => Some(serde_json::Value::String(s)),
                            None => {
                                eprintln!(
                                    "warning: cannot read body_file {} (tried {} and {})",
                                    file,
                                    candidates[0].display(),
                                    candidates[1].display()
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };
                    routes.push(ResolvedRoute {
                        path,
                        response: MockResponse {
                            status,
                            body,
                            stream_chunks: None,
                            headers,
                        },
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
    body: String,
    stream_chunks: Vec<String>,
    headers: Vec<(String, String)>,
}

type RouteTable = Arc<HashMap<String, MockRoute>>;

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_request(State(routes): State<RouteTable>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();

    // Try exact match first
    if let Some(route) = routes.get(&path) {
        return serve_route(route);
    }

    // Try prefix match: find a route that is a prefix of the request path
    // This allows /fixtures/basic_chat/v1/chat/completions to match /fixtures/basic_chat
    for (route_path, route) in routes.iter() {
        if path.starts_with(route_path) && (path.len() == route_path.len() || path.as_bytes()[route_path.len()] == b'/') {
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

    if !route.stream_chunks.is_empty() {
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
    // Use application/json when the body looks like JSON (starts with { or [),
    // otherwise fall back to text/plain to avoid clients failing JSON-decode.
    let has_content_type = route.headers.iter().any(|(k, _)| k.to_lowercase() == "content-type");
    let mut builder = Response::builder().status(status);
    if !has_content_type {
        let trimmed = route.body.trim_start();
        let default_ct = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            "application/json"
        } else {
            "text/plain"
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

fn load_routes(fixtures_dir: &Path) -> HashMap<String, MockRoute> {
    let mut routes = HashMap::new();
    load_routes_recursive(fixtures_dir, fixtures_dir, &mut routes);
    routes
}

fn load_routes_recursive(dir: &Path, fixtures_root: &Path, routes: &mut HashMap<String, MockRoute>) {
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
            load_routes_recursive(&path, fixtures_root, routes);
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
                for resolved in fixture.as_routes(fixtures_root) {
                    let mock = resolved.response;
                    let body = mock
                        .body
                        .as_ref()
                        .map(|b| match b {
                            // Plain strings (e.g. text/plain bodies) are stored as JSON strings in
                            // fixtures. Return the raw value so clients receive the string itself,
                            // not its JSON-encoded form with extra surrounding quotes.
                            serde_json::Value::String(s) => s.clone(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        })
                        .unwrap_or_default();
                    let stream_chunks = mock
                        .stream_chunks
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| match c {
                            serde_json::Value::String(s) => s,
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        })
                        .collect();
                    let mut headers: Vec<(String, String)> = mock.headers.into_iter().collect();
                    headers.sort_by(|a, b| a.0.cmp(&b.0));
                    routes.insert(
                        resolved.path,
                        MockRoute {
                            status: mock.status,
                            body,
                            stream_chunks,
                            headers,
                        },
                    );
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

    let routes = load_routes(fixtures_dir);
    eprintln!("mock-server: loaded {} routes from {}", routes.len(), fixtures_dir.display());

    let route_table: RouteTable = Arc::new(routes);
    let app = Router::new().fallback(handle_request).with_state(route_table);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock-server: failed to bind port");
    let addr: SocketAddr = listener.local_addr().expect("mock-server: failed to get local addr");

    // Print the URL so the parent process can read it.
    println!("MOCK_SERVER_URL=http://{addr}");
    // Flush stdout explicitly so the parent does not block waiting.
    use std::io::Write;
    std::io::stdout().flush().expect("mock-server: failed to flush stdout");

    // Spawn the server in the background.
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock-server: server error");
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
}
