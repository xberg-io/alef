//! HTTP server test function rendering for Python e2e tests.

use crate::e2e::escape::{escape_python, sanitize_ident};
use crate::e2e::fixture::Fixture;

use super::json::json_to_python_literal;

/// Render a pytest test function for an HTTP server fixture.
///
/// HTTP 101 (WebSocket upgrade) is handled specially: urllib cannot drive
/// upgrade responses, so those fixtures are omitted by the Python driver.
/// Other fixtures use the template-based generator.
pub(super) fn render_http_test_function(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };

    // HTTP 101 (WebSocket upgrade) — urllib cannot handle upgrade responses.
    // The Python driver filters these out before rendering; keep this guard for
    // direct callers and future renderer reuse.
    if http.expected_response.status_code == 101 {
        return;
    }

    // Build headers dict literal.
    let mut header_entries: Vec<String> = http
        .request
        .headers
        .iter()
        .map(|(k, v)| format!("        \"{}\": \"{}\",", escape_python(k), escape_python(v)))
        .collect();
    header_entries.sort();
    let headers_py = if header_entries.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n    }}", header_entries.join("\n"))
    };

    let method = http.request.method.to_uppercase();
    let path = format!("/fixtures/{}{}", &fixture.id, &http.request.path);

    // Determine body context
    let (has_body, body_py) = if let Some(body) = &http.request.body {
        let py_body = json_to_python_literal(body);
        (true, py_body)
    } else {
        (false, String::new())
    };

    // Determine body assertions
    let (has_text_body, text_py) = if let Some(serde_json::Value::String(s)) = &http.expected_response.body {
        (true, format!("\"{}\"", escape_python(s)))
    } else {
        (false, String::new())
    };

    let (has_json_body, json_py) = if let Some(body) = &http.expected_response.body {
        if !(body.is_null() || body.is_string() && body.as_str() == Some("")) {
            if !matches!(body, serde_json::Value::String(_)) {
                (true, json_to_python_literal(body))
            } else {
                (false, String::new())
            }
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let (has_partial_body, partial_body_checks) = if let Some(partial) = &http.expected_response.body_partial {
        if let Some(obj) = partial.as_object() {
            let checks: Vec<minijinja::Value> = obj
                .iter()
                .map(|(key, val)| {
                    let py_val = json_to_python_literal(val);
                    minijinja::context! {
                        key => escape_python(key),
                        py_val => py_val,
                    }
                })
                .collect();
            (true, checks)
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    // Build header assertions (deterministic order)
    let mut header_assertions: Vec<minijinja::Value> = Vec::new();
    let mut header_names: Vec<&String> = http.expected_response.headers.keys().collect();
    header_names.sort();
    for name in header_names {
        let value = &http.expected_response.headers[name];
        if name.eq_ignore_ascii_case("content-encoding") {
            continue;
        }
        let escaped_name = escape_python(&name.to_lowercase());
        let (assertion_type, val) = match value.as_str() {
            "<<present>>" => ("present", String::new()),
            "<<absent>>" => ("absent", String::new()),
            "<<uuid>>" => ("uuid", String::new()),
            exact => ("exact", escape_python(exact)),
        };
        header_assertions.push(minijinja::context! {
            name => escaped_name,
            assertion_type => assertion_type,
            value => val,
        });
    }

    // Build validation error assertions
    let body_has_content = matches!(&http.expected_response.body, Some(v)
        if !(v.is_null() || (v.is_string() && v.as_str() == Some(""))));
    let (has_validation_errors, validation_errors) = if let Some(errors) = &http.expected_response.validation_errors {
        if !errors.is_empty() && !body_has_content {
            let ve_list: Vec<minijinja::Value> = errors
                .iter()
                .map(|ve| {
                    let loc_py: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_python(s))).collect();
                    let loc_str = loc_py.join(", ");
                    let escaped_msg = escape_python(&ve.msg);
                    minijinja::context! {
                        loc_py => loc_str,
                        escaped_msg => escaped_msg,
                    }
                })
                .collect();
            (true, ve_list)
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    let ctx = minijinja::context! {
        fn_name => fn_name,
        description => desc_with_period,
        method => method,
        path => path,
        headers_py => headers_py,
        has_body => has_body,
        body_py => body_py,
        expected_status => http.expected_response.status_code,
        has_text_body => has_text_body,
        text_py => text_py,
        has_json_body => has_json_body,
        json_py => json_py,
        has_partial_body => has_partial_body,
        partial_body_checks => partial_body_checks,
        header_assertions => header_assertions,
        has_validation_errors => has_validation_errors,
        validation_errors => validation_errors,
    };
    let rendered = crate::e2e::template_env::render("python/http_test.jinja", ctx);
    out.push_str(&rendered);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    fn fixture_with_body(body: Option<serde_json::Value>) -> Fixture {
        Fixture {
            id: "basic_http".to_string(),
            description: "A basic HTTP test".to_string(),
            input: serde_json::Value::Null,
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/basic".to_string(),
                    method: "GET".to_string(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "GET".to_string(),
                    path: "/basic".to_string(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: None,
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            visitor: None,
            args: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn render_http_test_function_no_http_field_emits_nothing() {
        let fixture = crate::e2e::fixture::Fixture {
            id: "test_fixture".to_string(),
            description: "A test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            visitor: None,
            args: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture);
        assert!(out.is_empty(), "got: {out}");
    }

    #[test]
    fn render_http_test_function_preserves_statement_newlines_without_body() {
        let fixture = fixture_with_body(None);
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture);

        assert!(
            out.contains("_headers = {}\n    _req = urllib.request.Request"),
            "got: {out}"
        );
        assert!(out.contains("method=\"GET\")\n    class _NoRedirect"), "got: {out}");
        assert!(!out.contains("E704"), "got: {out}");
    }

    #[test]
    fn render_http_test_function_preserves_statement_newlines_with_body() {
        let fixture = fixture_with_body(Some(serde_json::json!({ "name": "alef" })));
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture);

        assert!(out.contains("_headers = {}\n    import json"), "got: {out}");
        assert!(out.contains("method=\"GET\")\n    class _NoRedirect"), "got: {out}");
        assert!(!out.contains("E704"), "got: {out}");
    }
}
