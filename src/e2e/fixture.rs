//! Fixture loading, validation, and grouping for e2e test generation.

use crate::core::config::e2e::ArgMapping;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Mock HTTP response for testing HTTP clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    /// HTTP status code.
    pub status: u16,
    /// JSON response body (for non-streaming responses).
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// SSE stream chunks (for streaming responses).
    /// Each chunk is a JSON object sent as `data: <chunk>\n\n`.
    #[serde(default)]
    pub stream_chunks: Option<Vec<serde_json::Value>>,
    /// Response headers to apply to the mock response.
    /// Bridged from `http.expected_response.headers` for HTTP fixtures.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Visitor specification for visitor pattern tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitorSpec {
    /// Map of callback method name to action.
    pub callbacks: BTreeMap<String, CallbackAction>,
}

/// Action a visitor callback should take.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum CallbackAction {
    /// Return VisitResult::Skip.
    #[serde(rename = "skip")]
    Skip,
    /// Return VisitResult::Continue.
    #[serde(rename = "continue")]
    Continue,
    /// Return VisitResult::PreserveHtml.
    #[serde(rename = "preserve_html")]
    PreserveHtml,
    /// Return VisitResult::Custom with static output.
    #[serde(rename = "custom")]
    Custom {
        /// The static replacement string.
        output: String,
    },
    /// Return VisitResult::Custom with template interpolation.
    #[serde(rename = "custom_template")]
    CustomTemplate {
        /// Template with placeholders like {text}, {href}.
        template: String,
        /// How the generated visitor returns the rendered template to the host.
        /// `Dict` (default) returns `{"custom": "..."}` (or per-language equivalent)
        /// to hit the structured-result code path; `BareString` returns the raw
        /// rendered string to hit the string-result code path. Both must produce
        /// `VisitResult::Custom`.
        #[serde(default)]
        return_form: TemplateReturnForm,
    },
}

/// How a `CustomTemplate` action returns its rendered value from the visitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateReturnForm {
    /// Return a host-native structured value (e.g. dict, hash, array, object)
    /// carrying the rendered string under a `custom` key.
    #[default]
    Dict,
    /// Return the rendered string directly, with no wrapper.
    BareString,
}

/// Environment variable requirements for a smoke/live test fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEnv {
    /// Name of the env var that holds the API key (e.g. `"OPENAI_API_KEY"`).
    #[serde(default)]
    pub api_key_var: Option<String>,
}

/// Setup call: a mini-call executed before the main fixture call.
/// Used to establish stateful resources like registered backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupCall {
    /// Named call config to use (references `[e2e.calls.<name>]`).
    pub call: String,
    /// Input data passed to the setup call.
    #[serde(default)]
    pub input: serde_json::Value,
}

/// A single e2e test fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    /// Unique identifier (used as test function name).
    pub id: String,
    /// Optional category (defaults to parent directory name).
    #[serde(default)]
    pub category: Option<String>,
    /// Human-readable description.
    pub description: String,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Skip directive.
    #[serde(default)]
    pub skip: Option<SkipDirective>,
    /// Environment variable requirements (used by smoke/live tests).
    #[serde(default)]
    pub env: Option<FixtureEnv>,
    /// Setup calls executed before the main call (used to register backends, etc).
    #[serde(default)]
    pub setup: Vec<SetupCall>,
    /// Named call config to use (references `[e2e.calls.<name>]`).
    /// When omitted, uses the default `[e2e.call]`.
    #[serde(default)]
    pub call: Option<String>,
    /// Input data passed to the function under test.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Optional mock HTTP response for testing HTTP clients.
    #[serde(default)]
    pub mock_response: Option<MockResponse>,
    /// Optional visitor specification for visitor pattern tests.
    #[serde(default)]
    pub visitor: Option<VisitorSpec>,
    /// Fixture-level argument mappings. When non-empty, overrides call_config.args
    /// for this specific fixture (used for trait-bridge stubs and other per-fixture args).
    #[serde(default)]
    pub args: Vec<ArgMapping>,
    /// Assertion recipes this fixture opts into.
    ///
    /// Domain-shaped assertions such as embeddings, keyword extraction,
    /// tree-query helpers, and streaming pseudo-fields require an explicit
    /// recipe opt-in so generic e2e fixtures don't silently inherit
    /// project-specific assumptions.
    #[serde(default)]
    pub assertion_recipes: Vec<String>,
    /// List of assertions to check.
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    /// Source file path (populated during loading).
    #[serde(skip)]
    pub source: String,
    /// HTTP server test specification. When present, this fixture tests
    /// an HTTP handler rather than a function call.
    #[serde(default)]
    pub http: Option<HttpFixture>,
}

/// HTTP server test specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFixture {
    /// Handler/route definition.
    pub handler: HttpHandler,
    /// The HTTP request to send.
    pub request: HttpRequest,
    /// Expected response.
    pub expected_response: HttpExpectedResponse,
}

/// Handler/route definition for HTTP server tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHandler {
    /// Route pattern (e.g., "/users/{user_id}").
    pub route: String,
    /// HTTP method (GET, POST, PUT, etc.).
    pub method: String,
    /// JSON Schema for request body validation.
    #[serde(default)]
    pub body_schema: Option<serde_json::Value>,
    /// Parameter schemas by source (path, query, header, cookie).
    #[serde(default)]
    pub parameters: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// Middleware configuration.
    #[serde(default)]
    pub middleware: Option<HttpMiddleware>,
}

/// HTTP request to send in a server test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    #[serde(default)]
    pub form_data: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub content_type: Option<String>,
}

impl HttpRequest {
    /// Encode form_data as a URL-encoded body string (key=value&key=value).
    /// Returns None if form_data is None.
    pub fn url_encoded_body(&self) -> Option<String> {
        self.form_data.as_ref().map(|form| {
            form.iter()
                .map(|(k, v)| {
                    let encoded_k = Self::url_encode(k);
                    let encoded_v = Self::url_encode(v);
                    format!("{}={}", encoded_k, encoded_v)
                })
                .collect::<Vec<_>>()
                .join("&")
        })
    }

    /// Simple URL encoding for form data (RFC 3986).
    fn url_encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{:02X}", b),
            })
            .collect()
    }
}

/// Expected HTTP response specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExpectedResponse {
    pub status_code: u16,
    /// Exact body match.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// Partial body match (only check specified fields).
    #[serde(default)]
    pub body_partial: Option<serde_json::Value>,
    /// Header expectations. Special tokens: `<<uuid>>`, `<<present>>`, `<<absent>>`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Expected validation errors (for 422 responses).
    #[serde(default)]
    pub validation_errors: Option<Vec<ValidationErrorExpectation>>,
}

/// Expected validation error entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationErrorExpectation {
    pub loc: Vec<String>,
    pub msg: String,
    #[serde(rename = "type")]
    pub error_type: String,
}

/// CORS policy configuration for HTTP handler tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Allowed origins (e.g. `["https://example.com"]`). Empty means deny all.
    #[serde(default)]
    pub allow_origins: Vec<String>,
    /// Allowed HTTP methods (e.g. `["GET", "POST"]`). Empty means deny all.
    #[serde(default)]
    pub allow_methods: Vec<String>,
    /// Allowed request headers (e.g. `["Content-Type"]`). Empty means deny all.
    #[serde(default)]
    pub allow_headers: Vec<String>,
    /// Exposed response headers (e.g. `["X-Total-Count"]`).
    #[serde(default)]
    pub expose_headers: Vec<String>,
    /// `Access-Control-Max-Age` value in seconds.
    #[serde(default)]
    pub max_age: Option<u64>,
    /// Whether to allow credentials.
    #[serde(default)]
    pub allow_credentials: bool,
}

/// A single static file entry for the static-files middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticFile {
    /// Relative path within the served directory (e.g. `"hello.txt"`).
    pub path: String,
    /// File content (plain text or HTML string).
    pub content: String,
}

/// Static-files middleware configuration for HTTP handler tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticFilesConfig {
    /// URL route prefix (e.g. `"/public"`).
    pub route_prefix: String,
    /// Files to write to the temporary directory.
    #[serde(default)]
    pub files: Vec<StaticFile>,
    /// Whether to serve `index.html` for directory requests.
    #[serde(default)]
    pub index_file: bool,
    /// `Cache-Control` header value to apply.
    #[serde(default)]
    pub cache_control: Option<String>,
}

/// Middleware configuration for HTTP handler tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpMiddleware {
    #[serde(default)]
    pub jwt_auth: Option<serde_json::Value>,
    #[serde(default)]
    pub api_key_auth: Option<serde_json::Value>,
    #[serde(default)]
    pub compression: Option<serde_json::Value>,
    #[serde(default)]
    pub rate_limit: Option<serde_json::Value>,
    #[serde(default)]
    pub request_timeout: Option<serde_json::Value>,
    /// Maximum request-body size policy (e.g. `{"max_bytes": 1024}`). Passed
    /// through opaquely so backends can wire it to their body-limit middleware.
    #[serde(default)]
    pub body_limit: Option<serde_json::Value>,
    #[serde(default)]
    pub request_id: Option<serde_json::Value>,
    /// CORS policy to apply via tower-http `CorsLayer`.
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    /// Static-files configuration to serve via tower-http `ServeDir`.
    #[serde(default)]
    pub static_files: Option<Vec<StaticFilesConfig>>,
    /// GraphQL route configuration (e.g. `{"schema": "...", "response_data": {...}}`).
    /// Passed through opaquely so backends can register a GraphQL endpoint rather
    /// than the generic route+handler pattern used by the other middleware fields.
    #[serde(default)]
    pub graphql: Option<serde_json::Value>,
}

const ORIGIN_ROOT_ROUTE_PREFIXES: [&str; 2] = ["/robots", "/sitemap"];

/// Returns true for fixture route paths that must be served from the origin root rather than
/// under a fixture-namespaced prefix. Mirrors the identical predicate in the standalone
/// mock-server binary (`codegen/rust/mock_server.rs`).
fn is_host_root_path(path: &str) -> bool {
    ORIGIN_ROOT_ROUTE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

impl Default for Fixture {
    fn default() -> Self {
        Fixture {
            id: String::new(),
            category: None,
            description: String::new(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: Vec::new(),
            source: String::new(),
            http: None,
        }
    }
}

impl Fixture {
    /// Resolve the effective args for this fixture, preferring fixture-level args when present.
    ///
    /// When `self.args` is non-empty, returns a reference to it. Otherwise, returns
    /// a reference to `call_config.args`. This allows fixtures to override the call's
    /// default args (e.g., for trait-bridge stubs that need per-fixture test backend setup).
    pub fn resolved_args<'a>(&'a self, call_config: &'a crate::core::config::e2e::CallConfig) -> &'a [ArgMapping] {
        if !self.args.is_empty() {
            &self.args
        } else {
            &call_config.args
        }
    }

    /// Returns true if this is an HTTP server test fixture.
    pub fn is_http_test(&self) -> bool {
        self.http.is_some()
    }

    /// Returns true if this fixture requires a mock HTTP server.
    /// This is true when the fixture declares a single mock response, an HTTP expected
    /// response, or one or more entries in the generic `input.mock_responses` route array.
    pub fn needs_mock_server(&self) -> bool {
        if self.mock_response.is_some() || self.http.is_some() {
            return true;
        }
        self.input
            .get("mock_responses")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false)
    }

    /// Returns the effective mock response for this fixture, bridging both schemas:
    /// - call fixture schema: `mock_response: { status, body, stream_chunks }`
    /// - HTTP fixture schema: `http.expected_response: { status_code, body, headers }`
    ///
    /// Returns `None` if neither schema is present.
    pub fn as_mock_response(&self) -> Option<MockResponse> {
        if let Some(mock) = &self.mock_response {
            return Some(mock.clone());
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

    /// Returns true if the mock response uses streaming (SSE).
    pub fn is_streaming_mock(&self) -> bool {
        self.mock_response
            .as_ref()
            .and_then(|m| m.stream_chunks.as_ref())
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    /// Returns true if this fixture needs a dedicated origin-root listener.
    ///
    /// Route-array fixtures are normally mounted under `/fixtures/<id>` in the shared
    /// mock server. A dedicated listener is required when a route path or fixture body
    /// makes the client under test resolve follow-up requests from the origin root rather
    /// than the fixture namespace. Mirrors the `is_host_root_path` predicate in the
    /// standalone mock-server binary (`codegen/rust/mock_server.rs`).
    ///
    /// Origin-root fixtures get a dedicated per-fixture listener and their base URL is
    /// published in the `MOCK_SERVERS={"fixture_id":"http://..."}` JSON line.
    pub fn has_host_root_route(&self) -> bool {
        if let Some(arr) = self.input.get("mock_responses").and_then(|v| v.as_array()) {
            if arr.iter().any(|entry| {
                entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(is_host_root_path)
                    .unwrap_or(false)
            }) {
                return true;
            }
            // A response can trigger a follow-up request to an origin-root path. In that
            // case, the fixture must be served on a dedicated listener so the next request
            // resolves against the same route table. Three trigger shapes are detected:
            //   - 3xx with Location: /...
            //   - any status with Refresh: <s>;url=/...
            //   - 200 HTML with <meta http-equiv="refresh" content="...url=/...">
            return arr.iter().any(|entry| {
                let status = entry.get("status_code").and_then(|v| v.as_u64()).unwrap_or(0);
                let headers = entry.get("headers").and_then(|v| v.as_object());
                let location_redirect = (300..400).contains(&status)
                    && headers
                        .map(|hdrs| {
                            hdrs.iter().any(|(name, value)| {
                                name.eq_ignore_ascii_case("location")
                                    && value.as_str().is_some_and(|s| s.starts_with('/'))
                            })
                        })
                        .unwrap_or(false);
                let refresh_redirect = headers
                    .map(|hdrs| {
                        hdrs.iter().any(|(name, value)| {
                            if !name.eq_ignore_ascii_case("refresh") {
                                return false;
                            }
                            value
                                .as_str()
                                .and_then(|s| s.to_ascii_lowercase().find("url=").map(|i| (s.to_owned(), i)))
                                .map(|(s, idx)| s[idx + 4..].trim_start().starts_with('/'))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                let meta_refresh = entry
                    .get("body_inline")
                    .and_then(|v| v.as_str())
                    .map(|body| {
                        let lower = body.to_ascii_lowercase();
                        lower
                            .split("http-equiv=\"refresh\"")
                            .nth(1)
                            .and_then(|s| s.split("content=").nth(1))
                            .map(|s| s.trim_start_matches(['"', '\'']).contains("url=/"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                // Inline HTML anchor with host-absolute target (`<a href="/page1">`) uses
                // the same trigger as the runtime mock-server `has_inline_host_link`
                // detection. Generated tests for multi-page fixtures use the shared
                // `/fixtures/<id>/` URL while clients resolve linked `/page` paths against
                // the host root.
                let inline_host_link = entry
                    .get("body_inline")
                    .and_then(|v| v.as_str())
                    .map(|body| body.contains("href=\"/") || body.contains("href='/"))
                    .unwrap_or(false);
                location_redirect || refresh_redirect || meta_refresh || inline_host_link
            });
        }
        false
    }

    /// Get the resolved category (explicit or from source directory).
    pub fn resolved_category(&self) -> String {
        self.category.clone().unwrap_or_else(|| {
            Path::new(&self.source)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string()
        })
    }
}

/// Skip directive for conditionally excluding fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipDirective {
    /// Languages to skip (empty means skip all).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Human-readable reason for skipping.
    #[serde(default)]
    pub reason: Option<String>,
}

impl SkipDirective {
    /// Check if this fixture should be skipped for a given language.
    pub fn should_skip(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.iter().any(|l| l == language)
    }
}

/// A single assertion in a fixture.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Assertion {
    /// Assertion type (equals, contains, not_empty, error, etc.).
    #[serde(rename = "type")]
    pub assertion_type: String,
    /// Field path to access on the result (dot-separated).
    #[serde(default)]
    pub field: Option<String>,
    /// Expected value (string, number, bool, or array depending on type).
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Expected values (for contains_all, contains_any).
    #[serde(default)]
    pub values: Option<Vec<serde_json::Value>>,
    /// Method name to call on the result (for method_result assertions).
    #[serde(default)]
    pub method: Option<String>,
    /// Assertion check type for the method result (equals, is_true, is_false, greater_than_or_equal, count_min).
    #[serde(default)]
    pub check: Option<String>,
    /// Arguments to pass to the method call (for method_result assertions).
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// Return type hint for C method_result codegen.
    ///
    /// Supported values:
    /// - `"string"` — the method returns a heap-allocated `char*` that must be
    ///   freed with `free()` after the assertion.  The generator emits
    ///   `char* _r = call(); assert(...); free(_r);`.
    ///
    /// Defaults to primitive integer dispatch when absent.
    #[serde(default)]
    pub return_type: Option<String>,
}

/// A group of fixtures sharing the same category.
#[derive(Debug, Clone)]
pub struct FixtureGroup {
    pub category: String,
    pub fixtures: Vec<Fixture>,
}

/// Load all fixtures from a directory recursively.
pub fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>> {
    let mut fixtures = Vec::new();
    load_fixtures_recursive(dir, dir, &mut fixtures)?;

    // Validate: check for duplicate IDs
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in &fixtures {
        if let Some(prev_source) = seen.get(&f.id) {
            bail!(
                "duplicate fixture ID '{}': found in '{}' and '{}'",
                f.id,
                prev_source,
                f.source
            );
        }
        seen.insert(f.id.clone(), f.source.clone());
    }

    // Sort by (category, id) for deterministic output
    fixtures.sort_by(|a, b| {
        let cat_cmp = a.resolved_category().cmp(&b.resolved_category());
        cat_cmp.then_with(|| a.id.cmp(&b.id))
    });

    Ok(fixtures)
}

fn load_fixtures_recursive(base: &Path, dir: &Path, fixtures: &mut Vec<Fixture>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_fixtures_recursive(base, &path, fixtures)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read fixture: {}", path.display()))?;
            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            // Try parsing as array first, then as single fixture. Normalize at the
            // raw JSON level so fixture-level helper fields that are not stored on
            // `Fixture` can still influence generated argument input.
            let parsed: Vec<Fixture> = if content.trim_start().starts_with('[') {
                let values: Vec<serde_json::Value> = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?;
                values
                    .into_iter()
                    .map(normalize_fixture_value)
                    .map(serde_json::from_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?
            } else {
                let value: serde_json::Value = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                let single: Fixture = serde_json::from_value(normalize_fixture_value(value))
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                vec![single]
            };

            for mut fixture in parsed {
                fixture.source = relative.clone();
                // Expand template expressions (e.g. `{{ repeat 'x' 10000 times }}`)
                // in all JSON string values so generators emit the expanded values.
                expand_json_templates(&mut fixture.input);
                if let Some(ref mut http) = fixture.http {
                    for (_, v) in http.request.headers.iter_mut() {
                        *v = crate::e2e::escape::expand_fixture_templates(v);
                    }
                    if let Some(ref mut body) = http.request.body {
                        expand_json_templates(body);
                    }
                }
                fixtures.push(fixture);
            }
        }
    }
    Ok(())
}

fn normalize_fixture_value(mut value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };

    if let Some(config) = object.get("config").cloned() {
        let input = object
            .entry("input")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(input_object) = input.as_object_mut() {
            input_object.entry("config".to_string()).or_insert(config);
        }
    }

    value
}

/// Group fixtures by their resolved category.
pub fn group_fixtures(fixtures: &[Fixture]) -> Vec<FixtureGroup> {
    let mut groups: HashMap<String, Vec<Fixture>> = HashMap::new();
    for f in fixtures {
        groups.entry(f.resolved_category()).or_default().push(f.clone());
    }
    let mut result: Vec<FixtureGroup> = groups
        .into_iter()
        .map(|(category, fixtures)| FixtureGroup { category, fixtures })
        .collect();
    result.sort_by(|a, b| a.category.cmp(&b.category));
    result
}

/// Recursively expand fixture template expressions in all string values of a JSON tree.
fn expand_json_templates(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            let expanded = crate::e2e::escape::expand_fixture_templates(s);
            if expanded != *s {
                *s = expanded;
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                expand_json_templates(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                expand_json_templates(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_with_mock_response() {
        let json = r#"{
            "id": "test_chat",
            "description": "Test chat",
            "call": "chat",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}]},
            "mock_response": {
                "status": 200,
                "body": {"choices": [{"message": {"content": "hello"}}]}
            },
            "assertions": [{"type": "not_error"}]
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
        assert_eq!(fixture.mock_response.unwrap().status, 200);
    }

    #[test]
    fn test_fixture_with_streaming_mock_response() {
        let json = r#"{
            "id": "test_stream",
            "description": "Test streaming",
            "input": {},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}, {"delta": " world"}]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(fixture.is_streaming_mock());
    }

    #[test]
    fn test_fixture_without_mock_response() {
        let json = r#"{
            "id": "test_no_mock",
            "description": "No mock",
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
    }

    #[test]
    fn normalize_fixture_value_copies_top_level_config_into_input() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {"kind": "uri", "uri": "doc.txt"},
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("markdown"))
        );
    }

    #[test]
    fn normalize_fixture_value_preserves_explicit_input_config() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {
                "kind": "uri",
                "uri": "doc.txt",
                "config": {"output_format": "html"}
            },
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("html"))
        );
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_robot_route_path() {
        let json = r#"{
            "id": "robots_disallow_path",
            "description": "Robots fixture",
            "input": {
                "mock_responses": [
                    {"path": "/robots.txt", "status_code": 200, "body_inline": "User-agent: *\nDisallow: /"},
                    {"path": "/", "status_code": 200, "body_inline": "<html/>"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.has_host_root_route(), "expected true for /robots.txt path");
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_sitemap_route_path() {
        let json = r#"{
            "id": "sitemap_index",
            "description": "Sitemap fixture",
            "input": {
                "mock_responses": [
                    {"path": "/sitemap.xml", "status_code": 200, "body_inline": "<?xml version='1.0'?>"},
                    {"path": "/", "status_code": 200, "body_inline": "<html/>"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.has_host_root_route(), "expected true for /sitemap.xml path");
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_redirect_target() {
        let json = r#"{
            "id": "redirect_fixture",
            "description": "Redirect fixture",
            "input": {
                "mock_responses": [
                    {
                        "path": "/",
                        "status_code": 302,
                        "headers": {"Location": "/final"},
                        "body_inline": ""
                    },
                    {"path": "/final", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            fixture.has_host_root_route(),
            "expected origin-root listener for origin-root redirect target"
        );
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_link_target() {
        let json = r#"{
            "id": "linked_pages",
            "description": "Linked pages",
            "input": {
                "mock_responses": [
                    {
                        "path": "/",
                        "status_code": 200,
                        "body_inline": "<html><a href='/page'>Page</a></html>"
                    },
                    {"path": "/page", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            fixture.has_host_root_route(),
            "expected origin-root listener for origin-root link target"
        );
    }

    #[test]
    fn has_host_root_route_false_for_data_json_path() {
        let json = r#"{
            "id": "data_endpoint",
            "description": "Namespaced route fixture",
            "input": {
                "mock_responses": [
                    {"path": "/data.json", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.has_host_root_route(), "expected false for /data.json path");
    }

    #[test]
    fn has_host_root_route_false_for_single_mock_response_schema() {
        let json = r#"{
            "id": "basic_chat",
            "description": "Basic chat",
            "mock_response": {"status": 200, "body": {}},
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            !fixture.has_host_root_route(),
            "expected false for single mock_response schema"
        );
    }

    #[test]
    fn has_host_root_route_false_for_empty_mock_responses() {
        let json = r#"{
            "id": "empty_responses",
            "description": "No mock_responses",
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.has_host_root_route(), "expected false when no mock_responses");
    }
}
