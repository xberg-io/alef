//! Rendering for the generated standalone Rust mock-server binary.

use crate::core::hash::{self, CommentStyle};

use super::route_loading::render_route_loading_source;
use super::runtime_server::render_runtime_server_source;

const BINARY_INTRO_SOURCE: &str = r####"//
// Standalone mock HTTP server binary for cross-language e2e tests.
// Reads fixture JSON files and serves mock responses on /fixtures/{fixture_id}.
//
// Usage: mock-server [fixtures-dir]
//   fixtures-dir defaults to "../../fixtures"
//
// Prints `MOCK_SERVER_URL=http://127.0.0.1:<port>` to stdout once listening,
// then optionally `MOCK_SERVERS={...}` with per-fixture URLs for origin-root fixtures,
// then blocks until stdin is closed (parent process exit triggers cleanup).

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Fixture types (mirrors alef-e2e's fixture.rs for runtime deserialization)
// Supports both single-response and HTTP expected-response fixture schemas.
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
    /// Route-array fixture schema:
    /// `input.mock_responses[i] = { path?, status_code, headers, body_inline | body_file }`.
    #[serde(default)]
    input: Option<serde_json::Value>,
}

/// A single resolved mock response with its serving path.
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

const ORIGIN_ROOT_ROUTE_PREFIXES: [&str; 2] = ["/robots", "/sitemap"];

/// Returns true for fixture route paths that must be served from the origin root rather
/// than under a fixture-namespaced prefix. These require a dedicated per-fixture listener.
fn is_host_root_path(path: &str) -> bool {
    ORIGIN_ROOT_ROUTE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
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
    /// Returns single-element output for `mock_response` / `http` schemas, and one
    /// element per array entry for the `input.mock_responses` route-array schema. For the
    /// array schema, each element may declare its own `path` (defaulting to `/fixtures/{id}`),
    /// and the body source can be either `body_inline` (string) or `body_file` (path relative
    /// to the fixtures dir, loaded at startup).
    ///
    /// Paths that are not origin-root discovery paths are namespaced
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

                    // Namespace under /fixtures/<id> unless this path must remain at the origin root.
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

"####;

const BINARY_ENTRYPOINT_SOURCE: &str = r####"// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let fixtures_dir_arg = std::env::args().nth(1).unwrap_or_else(|| "../../fixtures".to_string());
    let fixtures_dir = Path::new(&fixtures_dir_arg);

    // Resolve the test-documents corpus served as an HTTP fallback. Prefer the
    // explicit env var; otherwise use the `test_documents` sibling of the fixtures dir.
    let docs_dir = std::env::var_os("ALEF_TEST_DOCUMENTS_DIR")
        .map(PathBuf::from)
        .or_else(|| fixtures_dir.parent().map(|parent| parent.join("test_documents")))
        .filter(|dir| dir.is_dir());
    let _ = DOCS_DIR.set(docs_dir);

    let loaded = load_routes(fixtures_dir);
    eprintln!("mock-server: loaded {} shared routes from {}", loaded.shared.len(), fixtures_dir.display());

    // Shared namespaced server.
    let shared_table: RouteTable = Arc::new(loaded.shared);
    let shared_app = Router::new().fallback(handle_request).with_state(shared_table);

    let shared_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock-server: failed to bind shared port");
    let shared_addr: SocketAddr = shared_listener.local_addr().expect("mock-server: failed to get shared local addr");

    // Per-fixture listeners for origin-root routes.
    // Sorted by fixture_id for deterministic output.
    let mut fixture_ids: Vec<String> = loaded.per_fixture.keys().cloned().collect();
    fixture_ids.sort();

    let mut fixture_urls: HashMap<String, String> = HashMap::new();
    let mut readiness_addrs: Vec<SocketAddr> = Vec::new();
    for fixture_id in &fixture_ids {
        let routes = loaded.per_fixture[fixture_id].clone();
        eprintln!("mock-server: fixture {} has {} origin-root routes", fixture_id, routes.len());
        let table: RouteTable = Arc::new(routes);
        let app = Router::new().fallback(handle_request).with_state(table);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock-server: failed to bind per-fixture port");
        let addr: SocketAddr = listener.local_addr().expect("mock-server: failed to get per-fixture local addr");
        fixture_urls.insert(fixture_id.clone(), format!("http://{addr}"));
        readiness_addrs.push(addr);
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock-server: per-fixture server error");
        });
    }

    // Spawn the shared server before printing — the print is the readiness
    // signal consumers rely on, so the task must be running first.
    readiness_addrs.push(shared_addr);
    tokio::spawn(async move {
        axum::serve(shared_listener, shared_app).await.expect("mock-server: shared server error");
    });

    // Poll each listener with a self-connect until it is actually accepting.
    // This eliminates the race where consumers (e.g. Go's http.Get) attempt a
    // connection in the window between the print and the tokio task being
    // scheduled for the first time.
    for addr in &readiness_addrs {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if TcpStream::connect(addr).await.is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("mock-server: warning: listener {addr} did not become ready within 5s");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    // Print the shared URL so the parent process can read it.
    println!("MOCK_SERVER_URL=http://{shared_addr}");

    // Always print MOCK_SERVERS=... (empty `{}` when there are no origin-root
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

    // Lifetime: by default block until stdin is closed (the typical
    // parent-controlled subprocess pattern used by Rust/Node/Python/etc. test
    // harnesses that spawn this binary directly). When `MOCK_SERVER_NO_STDIN_WATCH=1`
    // is set, block on Ctrl-C / SIGTERM instead — useful for CI launches that
    // background the process across multiple shell steps (the per-step shell
    // exits and the inherited stdin FD closes, which would otherwise kill the
    // server between steps before the test step runs).
    if std::env::var("MOCK_SERVER_NO_STDIN_WATCH").as_deref() == Ok("1") {
        let _ = tokio::signal::ctrl_c().await;
    } else {
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        while lines.next().is_some() {}
    }
}

// ---------------------------------------------------------------------------
// Test-document HTTP fallback
// ---------------------------------------------------------------------------

/// Directory holding the test-document corpus, resolved once at startup.
static DOCS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Map a file extension to the MIME type the extraction core expects for it.
fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("txt") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("hwpx") => "application/hwp+zip",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Resolve `request_path` against the test-documents directory and serve the file
/// if it exists. Returns `None` when no docs dir is configured or the file is
/// missing, so the caller can fall through to a 404.
///
/// Language suites that ingest documents over HTTP (e.g. Node) request
/// `/<rel>/<file>` against the shared server; these paths are not fixture routes,
/// so the router resolves them here against the test-documents directory and
/// serves the bytes with an extension-derived content-type. Suites that pass
/// local file paths (Python, Go, Rust) never hit this.
fn serve_test_document(request_path: &str) -> Option<Response> {
    let docs_dir = DOCS_DIR.get()?.as_ref()?;
    let relative = request_path.trim_start_matches('/');
    if relative.is_empty() {
        return None;
    }
    // Guard against path traversal: reject any `..` component.
    if relative.split('/').any(|segment| segment == "..") {
        return None;
    }
    let candidate = docs_dir.join(relative);
    let bytes = std::fs::read(&candidate).ok()?;
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type_for(&candidate))
            .body(Body::from(bytes))
            .unwrap()
            .into_response(),
    )
}
"####;

/// Generate the `src/main.rs` for the standalone mock server binary.
///
/// The binary:
/// - Reads all `*.json` fixture files from a fixtures directory (default `../../fixtures`).
/// - For each fixture that has a mock-response schema, registers a route at
///   `/fixtures/{fixture_id}` returning the configured status/body/SSE chunks.
/// - Binds to `127.0.0.1:0` (random port), prints `MOCK_SERVER_URL=http://...`
///   to stdout, then waits until stdin is closed for clean teardown.
/// - Fixtures that need origin-root route resolution get their own dedicated listener. A second
///   line `MOCK_SERVERS={...}` (sorted JSON object) maps fixture_id → base URL for those.
///
/// This binary is intended for cross-language e2e suites (WASM, Node) that
/// spawn it as a child process and read the URL from its stdout.
pub fn render_mock_server_binary() -> String {
    let mut out = hash::header(CommentStyle::DoubleSlash);
    out.push_str(BINARY_INTRO_SOURCE);
    out.push_str(render_runtime_server_source());
    out.push_str(render_route_loading_source());
    out.push_str(BINARY_ENTRYPOINT_SOURCE);
    out
}
