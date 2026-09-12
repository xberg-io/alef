//! Shared HTTP/WebSocket test-codegen abstractions.
//!
//! Per-language e2e codegen (`crates/alef-e2e/src/codegen/<lang>.rs`) was previously
//! a monolithic ~1k-2k-line file per language that duplicated the structural shape
//! of every test (function header, request build, response assert) and only
//! differed in syntax. This module pulls the common shape into a trait + driver
//! pair so each language file becomes thin: implement primitives, delegate to
//! [`http_call::render_http_test`] / [`ws_script::render_ws_test`].
//!
//! The trait targets the **TestClient-driven** test shape — i.e. tests call
//! `client.METHOD(path, body, headers)` against a `TestClient` exposed by the
//! language binding, rather than spinning up a TCP mock server. Languages that
//! cannot expose `TestClient` over FFI (Go/Java/C#) implement the same trait but
//! emit code that spawns the binding's `App.serve()` on a loopback port and
//! drives it with their stdlib HTTP client.

use crate::e2e::fixture::{Fixture, HttpExpectedResponse, HttpRequest, ValidationErrorExpectation};
use std::collections::BTreeMap;

pub mod http_call;
pub mod ws_script;

/// Context for rendering a single TestClient HTTP call.
///
/// `response_var` is the binding-side identifier the renderer should use when
/// emitting subsequent assertions (e.g. `response`, `_resp`, `let response =`).
#[derive(Debug)]
pub struct CallCtx<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a BTreeMap<String, String>,
    pub query_params: &'a BTreeMap<String, serde_json::Value>,
    pub cookies: &'a BTreeMap<String, String>,
    pub body: Option<&'a serde_json::Value>,
    pub content_type: Option<&'a str>,
    pub response_var: &'a str,
}

impl<'a> CallCtx<'a> {
    pub fn from_request(req: &'a HttpRequest, response_var: &'a str) -> Self {
        Self {
            method: req.method.as_str(),
            path: req.path.as_str(),
            headers: &req.headers,
            query_params: &req.query_params,
            cookies: &req.cookies,
            body: req.body.as_ref(),
            content_type: req.content_type.as_deref(),
            response_var,
        }
    }
}

/// The effective Content-Type for a call: the explicit `Content-Type` request
/// header when present (case-insensitive), falling back to `ctx.content_type`.
///
/// Fixtures commonly declare a form/multipart content type only in
/// `request.headers`, leaving `ctx.content_type` unset; consulting only
/// `ctx.content_type` misses that case and a raw string body then gets
/// JSON-encoded (quoted) by a renderer that assumes JSON.
pub fn effective_content_type<'a>(ctx: &CallCtx<'a>) -> Option<&'a str> {
    ctx.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
        .or(ctx.content_type)
}

/// Whether a Content-Type indicates a raw (non-JSON-encoded) text body — i.e.
/// form-urlencoded or multipart, where the fixture body is already
/// pre-encoded wire content and must be sent byte-for-byte rather than
/// serialized as JSON.
pub fn is_raw_text_content_type(content_type: &str) -> bool {
    let ct_lower = content_type.to_ascii_lowercase();
    ct_lower.contains("multipart/form-data") || ct_lower.contains("application/x-www-form-urlencoded")
}

/// Per-language TestClient test renderer.
///
/// Implementations live alongside the per-language codegen module
/// (`crates/alef-e2e/src/codegen/<lang>/client.rs`). The shared driver
/// [`http_call::render_http_test`] calls these primitives in order to assemble
/// a complete test. Methods append to `out`; they MUST NOT clear or seek it.
///
/// Most methods take a `response_var: &str` argument so the renderer can
/// reference the value bound by `render_call`. Default value: `"response"`.
pub trait TestClientRenderer {
    /// Identifier used in fixture skip directives (e.g. `"python"`, `"node"`).
    fn language_name(&self) -> &'static str;

    /// Content-encodings this language's generated test client can decode.
    ///
    /// A fixture may legitimately advertise `Accept-Encoding: br` and expect a
    /// `content-encoding: br` response — that is a realistic exchange and the mock server
    /// serves it for real. Whether the *generated client* can read the result is a property
    /// of the language and its HTTP stack, not of the fixture, so it is answered here rather
    /// than duplicated into every consumer's fixture corpus.
    ///
    /// Overriding this to omit an encoding does not weaken an existing assertion: without it
    /// the test does not fail on a comparison, it fails on undecoded bytes reaching a JSON
    /// reader, or — for stacks that reject an encoding they cannot handle at the protocol
    /// layer — before a body exists to compare at all. A named skip states the limitation;
    /// the alternative states nothing and is red.
    ///
    /// The default claims gzip and brotli, matching the stacks that decode both
    /// transparently. ~keep
    fn decodable_content_encodings(&self) -> &'static [&'static str] {
        &["gzip", "br"]
    }

    /// Convert a fixture id (`my_test_id`) to a language-valid identifier.
    /// Default implementation lower-cases and replaces non-alphanumerics with `_`.
    fn sanitize_test_name(&self, id: &str) -> String {
        crate::e2e::escape::sanitize_ident(id)
    }

    /// Render the test-function opening: doc, signature, opening brace.
    /// `skip_reason: Some(...)` means the fixture is skipped for this language;
    /// the renderer should emit the language-native skip annotation.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>);

    /// Render the test-function closing brace / `end` / etc.
    fn render_test_close(&self, out: &mut String);

    /// Render `let <response_var> = client.METHOD(path, body, query, headers, cookies)`
    /// (or per-language equivalent). Including the trailing newline.
    fn render_call(&self, out: &mut String, ctx: &CallCtx<'_>);

    /// Render `assert <response_var>.status == status`.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16);

    /// Render `assert <response_var>.headers[name] == expected`.
    /// `expected` may be a literal value or one of the special tokens `<<uuid>>`,
    /// `<<present>>`, `<<absent>>` per the fixture schema; the renderer is
    /// responsible for handling those.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str);

    /// Render an exact-equality JSON body assertion. The renderer is responsible
    /// for parsing the response body as JSON (or the appropriate language-native
    /// equivalent) and comparing it to `expected`.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value);

    /// Render a partial JSON body assertion: every field in `expected` must be
    /// present in the response with the same value, but the response may have
    /// additional fields.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value);

    /// Render a validation-errors assertion for 422 responses.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    );
}

/// Whether a fixture is skipped for the given language.
///
/// Pulled into the shared driver layer so individual renderers don't reimplement.
pub fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture
        .skip
        .as_ref()
        .map(|s| s.languages.iter().any(|l| l == language))
        .unwrap_or(false)
}

/// Whether the expected-response carries any header expectations beyond
/// `content-encoding`.
///
/// `content-encoding` stays excluded, but no longer because the mock layer strips it --
/// the server encodes the body for real. It is excluded because
/// HTTP clients disagree about what survives transparent decompression: `fetch` decodes
/// and leaves the header in place, while other clients remove it once the body is decoded.
/// Asserting the value would therefore pass or fail on client behaviour rather than on
/// ours, across fourteen languages. The coverage that matters -- the client actually
/// decompressing a real gzip body -- comes from the server encoding it, not from asserting
/// the header. ~keep
pub fn has_meaningful_headers(expected: &HttpExpectedResponse) -> bool {
    expected
        .headers
        .iter()
        .any(|(k, _)| !k.eq_ignore_ascii_case("content-encoding"))
}

/// Whether the expected-response carries a non-empty body.
///
/// The fixture schema uses `null` and `""` as "no body" sentinels — neither
/// triggers a body assertion.
pub fn has_meaningful_body(expected: &HttpExpectedResponse) -> bool {
    match &expected.body {
        Some(v) if v.is_null() => false,
        Some(v) if v.as_str() == Some("") => false,
        Some(_) => true,
        None => false,
    }
}
