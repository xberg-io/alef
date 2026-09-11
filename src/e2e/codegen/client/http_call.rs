//! Shared HTTP-test driver.
//!
//! Calls trait primitives on a [`TestClientRenderer`] in the canonical order
//! a TestClient-driven test takes:
//!
//! 1. `render_test_open` — doc, signature, opening brace, language-native skip annotation.
//! 2. `render_call` — `let response = client.METHOD(...)`.
//! 3. `render_assert_status` — status code assertion.
//! 4. `render_assert_header` (per header) — header assertions.
//! 5. `render_assert_json_body` / `render_assert_partial_body` — body assertion.
//! 6. `render_assert_validation_errors` — 422 validation errors, if present.
//! 7. `render_test_close` — closing brace / `end`.
//!
//! Steps 3-6 are skipped automatically when the corresponding expectation is empty.

use super::{CallCtx, TestClientRenderer, has_meaningful_body, is_skipped};
use crate::e2e::fixture::Fixture;
use std::collections::BTreeMap;

/// Default name for the response binding inside a generated test.
pub const DEFAULT_RESPONSE_VAR: &str = "response";

/// Render a single HTTP test for `fixture` to `out` using `renderer`.
///
/// Returns `true` if a test was emitted (the fixture has an `http` block),
/// `false` otherwise — caller is responsible for handling non-HTTP fixtures
/// (WebSocket, AsyncAPI spec validation, etc.) via different drivers.
pub fn render_http_test<R: TestClientRenderer + ?Sized>(out: &mut String, renderer: &R, fixture: &Fixture) -> bool {
    let Some(http) = fixture.http.as_ref() else {
        return false;
    };

    let fn_name = renderer.sanitize_test_name(&fixture.id);

    // An encoding this client cannot decode is a skip, not a failure. See
    // `TestClientRenderer::decodable_content_encodings` for why the capability is answered
    // by the language rather than by the fixture.
    let undecodable = undecodable_encoding(http, renderer.decodable_content_encodings());
    let generated_skip = undecodable
        .map(|encoding| format!("{encoding} responses are not decodable by this language's generated test client"));

    let skip_reason = if is_skipped(fixture, renderer.language_name()) {
        Some(
            fixture
                .skip
                .as_ref()
                .and_then(|s| s.reason.as_deref())
                .unwrap_or("skipped"),
        )
    } else {
        generated_skip.as_deref()
    };

    renderer.render_test_open(out, &fn_name, &fixture.description, skip_reason);

    if skip_reason.is_some() {
        // For some languages, render_test_open already emitted a stub body; in
        // those cases render_test_close is still required for symmetry. Calls
        // below are gated on the renderer's expectations.
        renderer.render_test_close(out);
        return true;
    }

    let response_var = DEFAULT_RESPONSE_VAR;
    // For server-pattern e2e tests: build the full path to the fixture handler.
    // Path combines the fixture ID namespace with the actual fixture request path:
    // `/fixtures/{fixture.id}{request.path}` (e.g., `/fixtures/put_create_if_not_exists/items/999`)
    // Using request.path ensures parameterized routes are replaced with actual values.
    let request_path = &http.request.path;
    let namespaced_path = format!("/fixtures/{}{}", fixture.id, request_path);
    let req = &http.request;

    // Synthesize a multipart/form-data body when the fixture declares that
    // content type but carries no explicit request body, mirroring the
    // python/ruby/typescript generators. Without this the TestClient-driven
    // request goes out empty and the core rejects it with 422 (required binary
    // field missing) before the handler is reached. The synthesized body is a
    // raw string; each renderer's `render_call` escapes it for its language.
    let request_plan = plan_request(http);

    let ctx = CallCtx {
        method: req.method.as_str(),
        path: &namespaced_path,
        headers: &request_plan.headers,
        query_params: &req.query_params,
        cookies: &req.cookies,
        body: request_plan.body.as_ref(),
        content_type: request_plan.content_type.as_deref(),
        response_var,
    };
    renderer.render_call(out, &ctx);

    renderer.render_assert_status(out, response_var, http.expected_response.status_code);

    // Emit header assertions in deterministic (sorted) order so generated
    // output is stable across cargo invocations.
    let mut header_names: Vec<&String> = http.expected_response.headers.keys().collect();
    header_names.sort();
    for name in header_names {
        let value = &http.expected_response.headers[name];
        if name.eq_ignore_ascii_case("content-encoding") {
            // Not because the mock layer strips it -- the generated mock server
            // gzip-encodes the body for real. Clients disagree about what
            // survives transparent decompression: `fetch` decodes and leaves the
            // header in place, others remove it once decoded. Asserting the value
            // would test the client's behaviour rather than ours. The decompression
            // path is covered by the server sending a genuinely encoded body. ~keep
            continue;
        }
        renderer.render_assert_header(out, response_var, name, value);
    }

    if has_meaningful_body(&http.expected_response)
        && let Some(body) = http.expected_response.body.as_ref()
    {
        renderer.render_assert_json_body(out, response_var, body);
    }

    if let Some(partial) = http.expected_response.body_partial.as_ref() {
        renderer.render_assert_partial_body(out, response_var, partial);
    }

    if let Some(errors) = http.expected_response.validation_errors.as_ref()
        && !errors.is_empty()
    {
        renderer.render_assert_validation_errors(out, response_var, errors);
    }

    renderer.render_test_close(out);
    true
}

/// Boundary marker shared by every synthesized multipart body.
pub(crate) const MULTIPART_BOUNDARY: &str = "alef-boundary";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedRequest {
    pub headers: BTreeMap<String, String>,
    pub body: Option<serde_json::Value>,
    pub content_type: Option<String>,
}

pub(crate) fn plan_request(http: &crate::e2e::fixture::HttpFixture) -> PlannedRequest {
    let request = &http.request;
    let headers = request.headers.clone();
    let declared_content_type = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
        .or_else(|| request.content_type.clone());

    if request.body.is_some() {
        return PlannedRequest {
            headers,
            body: request.body.clone(),
            content_type: declared_content_type,
        };
    }

    if !is_multipart_content_type(declared_content_type.as_deref()) {
        return PlannedRequest {
            headers,
            body: None,
            content_type: declared_content_type,
        };
    }

    plan_multipart_request(http, headers, declared_content_type)
}

fn plan_multipart_request(
    http: &crate::e2e::fixture::HttpFixture,
    mut headers: BTreeMap<String, String>,
    declared_content_type: Option<String>,
) -> PlannedRequest {
    let request = &http.request;
    if let Some(form_data) = request.form_data.as_ref() {
        remove_content_type_header(&mut headers);
        if form_data.is_empty() {
            return PlannedRequest {
                headers,
                body: None,
                content_type: None,
            };
        }
        return PlannedRequest {
            headers,
            body: Some(serde_json::Value::String(render_form_data(form_data))),
            content_type: Some(multipart_content_type()),
        };
    }

    match synthesize_multipart_request(http) {
        Some(body) => {
            remove_content_type_header(&mut headers);
            PlannedRequest {
                headers,
                body: Some(body),
                content_type: Some(multipart_content_type()),
            }
        }
        None => PlannedRequest {
            headers,
            body: None,
            content_type: declared_content_type,
        },
    }
}

fn multipart_content_type() -> String {
    format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}")
}

fn remove_content_type_header(headers: &mut BTreeMap<String, String>) {
    headers.retain(|name, _| !name.eq_ignore_ascii_case("content-type"));
}

fn is_multipart_content_type(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("multipart/form-data"))
}

/// Synthesize a `multipart/form-data` request body from the handler's body
/// schema when the fixture declares that content type but carries no explicit
/// body. Returns the raw body as a JSON string value (each renderer escapes it
/// for its own language), or `None` when synthesis does not apply.
fn synthesize_multipart_request(http: &crate::e2e::fixture::HttpFixture) -> Option<serde_json::Value> {
    if http.request.body.is_some() || http.request.form_data.is_some() {
        return None;
    }

    let content_type = http
        .request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .or_else(|| http.request.content_type.as_ref().map(|c| c.to_ascii_lowercase()))
        .unwrap_or_default();
    let is_multipart = content_type
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("multipart/form-data"));
    if !is_multipart {
        return None;
    }

    let schema = http.handler.body_schema.as_ref()?;
    if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
        return None;
    }
    let props = schema.get("properties").and_then(|p| p.as_object())?;
    Some(serde_json::Value::String(synthesize_multipart_body_raw(props)))
}

fn render_form_data(form_data: &BTreeMap<String, String>) -> String {
    let mut body = String::new();
    for (name, value) in form_data {
        body.push_str(&format!(
            "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str(&format!("--{MULTIPART_BOUNDARY}--\r\n"));
    body
}

/// Build the raw multipart body: one part per schema property, with a filename
/// and `text/plain` part for `format: binary` fields and a plain value
/// otherwise.
fn synthesize_multipart_body_raw(props: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut body = String::new();
    for (prop_name, prop_schema) in props {
        let is_binary = prop_schema
            .get("format")
            .and_then(|f| f.as_str())
            .is_some_and(|f| f == "binary");
        body.push_str(&format!(
            "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"{prop_name}\""
        ));
        if is_binary {
            body.push_str(&format!(
                "; filename=\"{prop_name}.txt\"\r\nContent-Type: text/plain\r\n\r\nplaceholder content"
            ));
        } else {
            body.push_str("\r\n\r\nsample");
        }
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{MULTIPART_BOUNDARY}--\r\n"));
    body
}

/// The content-encoding this exchange requires the client to decode, when it is one the
/// client cannot.
///
/// Both halves matter. The response header is what actually arrives encoded, but a request
/// advertising `Accept-Encoding` for an encoding the client cannot read is itself the defect
/// — it asks a server for bytes it has no way to interpret — so a fixture is skipped on
/// either. `<<absent>>` is the sentinel for "this header must not be present" and never
/// names an encoding to decode.
fn undecodable_encoding<'a>(http: &'a crate::e2e::fixture::HttpFixture, decodable: &[&str]) -> Option<&'a str> {
    let is_undecodable = |value: &str| {
        !value.is_empty() && value != "<<absent>>" && !decodable.iter().any(|known| known.eq_ignore_ascii_case(value))
    };

    http.expected_response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, value)| value.as_str())
        .filter(|value| is_undecodable(value))
        .or_else(|| {
            http.request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
                .map(|(_, value)| value.as_str())
                .filter(|value| is_undecodable(value))
        })
}

#[cfg(test)]
mod tests {
    use super::super::{CallCtx, TestClientRenderer};
    use super::render_http_test;
    use crate::e2e::fixture::{Fixture, HttpExpectedResponse, HttpFixture, HttpRequest, ValidationErrorExpectation};
    use std::collections::BTreeMap;

    /// Mock renderer that records every call as a tag in `out`. Lets us assert
    /// the exact sequence of trait calls the shared driver makes for each
    /// expected-response shape.
    /// The tuple field is the set the mock client can decode, so the capability gate can be
    /// driven from a test without a second full renderer impl.
    struct TagRenderer(&'static [&'static str]);

    impl TestClientRenderer for TagRenderer {
        fn decodable_content_encodings(&self) -> &'static [&'static str] {
            self.0
        }

        fn language_name(&self) -> &'static str {
            "mock"
        }
        fn render_test_open(&self, out: &mut String, fn_name: &str, _: &str, skip: Option<&str>) {
            let skip_marker = skip.map(|r| format!("|skip={r}")).unwrap_or_default();
            out.push_str(&format!("OPEN({fn_name}{skip_marker})\n"));
        }
        fn render_test_close(&self, out: &mut String) {
            out.push_str("CLOSE\n");
        }
        fn render_call(&self, out: &mut String, ctx: &CallCtx<'_>) {
            out.push_str(&format!("CALL({} {} -> {})\n", ctx.method, ctx.path, ctx.response_var));
        }
        fn render_assert_status(&self, out: &mut String, _: &str, status: u16) {
            out.push_str(&format!("STATUS={status}\n"));
        }
        fn render_assert_header(&self, out: &mut String, _: &str, name: &str, value: &str) {
            out.push_str(&format!("HEADER({name}={value})\n"));
        }
        fn render_assert_json_body(&self, out: &mut String, _: &str, expected: &serde_json::Value) {
            out.push_str(&format!("JSON_BODY({expected})\n"));
        }
        fn render_assert_partial_body(&self, out: &mut String, _: &str, expected: &serde_json::Value) {
            out.push_str(&format!("PARTIAL_BODY({expected})\n"));
        }
        fn render_assert_validation_errors(&self, out: &mut String, _: &str, errors: &[ValidationErrorExpectation]) {
            out.push_str(&format!("VALIDATION({})\n", errors.len()));
        }
    }

    fn http_fixture(id: &str, expected: HttpExpectedResponse) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.into(),
            description: "test".into(),
            category: Some("smoke".into()),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: String::new(),
            http: Some(HttpFixture {
                handler: crate::e2e::fixture::HttpHandler {
                    route: String::new(),
                    method: "GET".into(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "GET".into(),
                    path: String::new(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: expected,
            }),
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }

    fn empty_expected(status: u16) -> HttpExpectedResponse {
        HttpExpectedResponse {
            status_code: status,
            body: None,
            body_partial: None,
            headers: BTreeMap::new(),
            validation_errors: None,
        }
    }

    #[test]
    fn driver_emits_open_call_status_close_in_order() {
        let fixture = http_fixture("simple", empty_expected(200));
        let mut out = String::new();
        let emitted = render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(emitted);
        assert_eq!(
            out,
            "OPEN(simple)\nCALL(GET /fixtures/simple -> response)\nSTATUS=200\nCLOSE\n"
        );
    }

    #[test]
    fn driver_skips_when_no_http_block() {
        let mut fixture = http_fixture("noop", empty_expected(200));
        fixture.http = None;
        let mut out = String::new();
        let emitted = render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(!emitted);
        assert!(out.is_empty());
    }

    #[test]
    fn driver_emits_skip_marker_and_short_circuits_assertions() {
        let mut fixture = http_fixture("skipme", empty_expected(200));
        fixture.skip = Some(crate::e2e::fixture::SkipDirective {
            languages: vec!["mock".into()],
            reason: Some("not yet".into()),
        });
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(out.contains("OPEN(skipme|skip=not yet)"));
        assert!(out.contains("CLOSE"));
        assert!(!out.contains("CALL"));
        assert!(!out.contains("STATUS"));
    }

    #[test]
    fn an_undecodable_response_encoding_becomes_a_named_skip() {
        let mut expected = empty_expected(200);
        expected.headers.insert("content-encoding".into(), "br".into());
        let fixture = http_fixture("brotli_only", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip"]), &fixture);
        assert!(out.contains("skip=br responses are not decodable"), "{out}");
        // The point of the skip is that no assertion runs: an undecodable body fails on the
        // bytes, not on a comparison, so emitting the call would be red rather than absent.
        assert!(!out.contains("CALL"), "{out}");
        assert!(!out.contains("STATUS"), "{out}");
    }

    #[test]
    fn an_undecodable_accept_encoding_request_header_becomes_a_skip() {
        let mut fixture = http_fixture("asks_for_brotli", empty_expected(200));
        fixture
            .http
            .as_mut()
            .expect("http fixture")
            .request
            .headers
            .insert("Accept-Encoding".into(), "br".into());
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip"]), &fixture);
        assert!(out.contains("skip=br responses are not decodable"), "{out}");
    }

    /// Negative control for both tests above: the gate must skip only what the client
    /// cannot read. A decodable encoding, and the `<<absent>>` sentinel that asserts a
    /// header is *missing*, must both still render a real test.
    #[test]
    fn decodable_and_absent_encodings_still_render_assertions() {
        let mut gzip = empty_expected(200);
        gzip.headers.insert("content-encoding".into(), "gzip".into());
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip"]), &http_fixture("gzipped", gzip));
        assert!(!out.contains("skip="), "{out}");
        assert!(out.contains("STATUS"), "{out}");

        let mut absent = empty_expected(200);
        absent.headers.insert("content-encoding".into(), "<<absent>>".into());
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip"]), &http_fixture("plain", absent));
        assert!(!out.contains("skip="), "{out}");
        assert!(out.contains("STATUS"), "{out}");
    }

    #[test]
    fn driver_strips_content_encoding_header_assertion() {
        let mut expected = empty_expected(200);
        expected.headers.insert("Content-Encoding".into(), "gzip".into());
        expected.headers.insert("X-Foo".into(), "bar".into());
        let fixture = http_fixture("hdr", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(!out.contains("HEADER(Content-Encoding"));
        assert!(out.contains("HEADER(X-Foo=bar)"));
    }

    #[test]
    fn driver_emits_headers_in_sorted_order() {
        let mut expected = empty_expected(200);
        expected.headers.insert("Z-Header".into(), "z".into());
        expected.headers.insert("A-Header".into(), "a".into());
        expected.headers.insert("M-Header".into(), "m".into());
        let fixture = http_fixture("hdr", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        let a_pos = out.find("HEADER(A-Header").unwrap();
        let m_pos = out.find("HEADER(M-Header").unwrap();
        let z_pos = out.find("HEADER(Z-Header").unwrap();
        assert!(a_pos < m_pos);
        assert!(m_pos < z_pos);
    }

    #[test]
    fn driver_skips_body_assert_for_null_and_empty_string_sentinels() {
        let mut expected = empty_expected(200);
        expected.body = Some(serde_json::Value::Null);
        let fixture = http_fixture("nullbody", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(!out.contains("JSON_BODY"));

        let mut expected = empty_expected(200);
        expected.body = Some(serde_json::Value::String(String::new()));
        let fixture = http_fixture("emptybody", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(!out.contains("JSON_BODY"));
    }

    #[test]
    fn driver_emits_body_partial_assertion_independently_of_body() {
        let mut expected = empty_expected(200);
        expected.body_partial = Some(serde_json::json!({"k": "v"}));
        let fixture = http_fixture("partial", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(out.contains("PARTIAL_BODY"));
    }

    #[test]
    fn driver_emits_validation_errors_assertion_when_present_and_nonempty() {
        let mut expected = empty_expected(422);
        expected.validation_errors = Some(vec![ValidationErrorExpectation {
            loc: vec!["name".into()],
            msg: "field required".into(),
            error_type: "missing".into(),
        }]);
        let fixture = http_fixture("ve", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(out.contains("VALIDATION(1)"));

        // Empty vec → no assertion
        let mut expected = empty_expected(422);
        expected.validation_errors = Some(vec![]);
        let fixture = http_fixture("ve_empty", expected);
        let mut out = String::new();
        render_http_test(&mut out, &TagRenderer(&["gzip", "br"]), &fixture);
        assert!(!out.contains("VALIDATION"));
    }

    #[test]
    fn synthesizes_multipart_body_for_schema_only_fixture() {
        let mut fixture = http_fixture("upload", empty_expected(200));
        {
            let http = fixture.http.as_mut().unwrap();
            http.request.content_type = Some("multipart/form-data".into());
            http.handler.body_schema = Some(serde_json::json!({
                "type": "object",
                "properties": { "file": { "type": "string", "format": "binary" } },
                "required": ["file"],
            }));
        }
        let body = super::synthesize_multipart_request(fixture.http.as_ref().unwrap())
            .expect("multipart body should be synthesized");
        let raw = body.as_str().unwrap();
        assert!(raw.contains("--alef-boundary"));
        assert!(raw.contains("name=\"file\""));
        assert!(raw.contains("filename=\"file.txt\""));
        assert!(raw.contains("placeholder content"));
        assert!(raw.ends_with("--alef-boundary--\r\n"));
    }

    #[test]
    fn no_multipart_synthesis_when_body_present_or_not_multipart() {
        // An explicit request body is used verbatim — never overridden.
        let mut with_body = http_fixture("explicit", empty_expected(200));
        {
            let http = with_body.http.as_mut().unwrap();
            http.request.content_type = Some("multipart/form-data".into());
            http.request.body = Some(serde_json::json!("verbatim"));
            http.handler.body_schema = Some(serde_json::json!({
                "type": "object",
                "properties": { "f": { "type": "string", "format": "binary" } },
            }));
        }
        assert!(super::synthesize_multipart_request(with_body.http.as_ref().unwrap()).is_none());

        // A non-multipart content type is left alone.
        let mut json_req = http_fixture("json", empty_expected(200));
        {
            let http = json_req.http.as_mut().unwrap();
            http.request.content_type = Some("application/json".into());
            http.handler.body_schema = Some(serde_json::json!({
                "type": "object",
                "properties": { "f": { "type": "string" } },
            }));
        }
        assert!(super::synthesize_multipart_request(json_req.http.as_ref().unwrap()).is_none());
    }

    #[test]
    fn explicit_empty_multipart_data_suppresses_body_and_content_type() {
        let mut fixture = http_fixture("empty_upload", empty_expected(200));
        let http = fixture.http.as_mut().unwrap();
        http.request.content_type = Some("multipart/form-data".into());
        http.request.form_data = Some(BTreeMap::new());
        http.handler.body_schema = Some(serde_json::json!({
            "type": "object",
            "properties": { "file": { "type": "string", "format": "binary" } },
        }));

        let plan = super::plan_request(http);
        assert_eq!(plan.body, None);
        assert_eq!(plan.content_type, None);
        assert!(
            !plan
                .headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("content-type"))
        );
    }

    #[test]
    fn explicit_multipart_form_data_uses_declared_values() {
        let mut fixture = http_fixture("form_upload", empty_expected(200));
        let http = fixture.http.as_mut().unwrap();
        http.request.content_type = Some("multipart/form-data".into());
        http.request.form_data = Some(BTreeMap::from([("caption".into(), "sample".into())]));

        let plan = super::plan_request(http);
        assert_eq!(
            plan.content_type.as_deref(),
            Some("multipart/form-data; boundary=alef-boundary")
        );
        let body = plan.body.and_then(|value| value.as_str().map(str::to_owned)).unwrap();
        assert!(body.contains("name=\"caption\""));
        assert!(body.contains("sample"));
    }
}
