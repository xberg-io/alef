//! Ruby HTTP rendering for e2e test generation.

use crate::e2e::codegen::client;
use crate::e2e::escape::{escape_ruby_single, ruby_string_literal, sanitize_ident};
use crate::e2e::fixture::{Fixture, ValidationErrorExpectation};

use super::values::json_to_ruby;

/// Thin renderer that emits RSpec `describe` + `it` blocks targeting a mock server
/// via `Net::HTTP`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
pub(super) struct RubyTestClientRenderer;

impl client::TestClientRenderer for RubyTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "ruby"
    }

    /// Emit `describe '{fn_name}' do` + inner `it '{description}' do`.
    ///
    /// `fn_name` is the sanitised fixture id used as the describe label.
    /// When `skip_reason` is `Some`, the inner `it` block gets a `skip` call so
    /// the shared driver short-circuits before emitting any assertions.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let description_literal = ruby_string_literal(description);
        let rendered = crate::e2e::template_env::render(
            "ruby/http_test.jinja",
            minijinja::context! {
                fn_name => fn_name,
                description => description_literal,
                skip_reason => skip_reason,
            },
        );
        out.push_str(&rendered);
    }

    /// Close the inner `it` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let rendered = crate::e2e::template_env::render("ruby/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit a `Net::HTTP` request to the mock server using the path from `ctx`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let method_class = http_method_class(&method);

        let has_body = ctx
            .body
            .is_some_and(|b| !matches!(b, serde_json::Value::String(s) if s.is_empty()));

        let ruby_body = if has_body {
            json_to_ruby(ctx.body.unwrap())
        } else {
            String::new()
        };

        let headers: Vec<minijinja::Value> = ctx
            .headers
            .iter()
            .filter(|(k, _)| {
                // Skip Content-Type when already set from the body above.
                !(has_body && k.to_lowercase() == "content-type")
            })
            .map(|(k, v)| {
                minijinja::context! {
                    key_literal => ruby_string_literal(k),
                    value_literal => ruby_string_literal(v),
                }
            })
            .collect();

        let rendered = crate::e2e::template_env::render(
            "ruby/http_request.jinja",
            minijinja::context! {
                method_class => method_class,
                path => ctx.path,
                has_body => has_body,
                ruby_body => ruby_body,
                headers => headers,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `expect(response.code.to_i).to eq(status)`.
    ///
    /// Net::HTTP returns the HTTP status as a `String`; `.to_i` converts it for
    /// comparison with the integer literal from the fixture.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        out.push_str(&format!("      expect({response_var}.code.to_i).to eq({status})\n"));
    }

    /// Emit a header assertion using `response[header_key]`.
    ///
    /// Handles the three special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_expr = format!("{response_var}[{}]", ruby_string_literal(&header_key));
        let assertion = match expected {
            "<<present>>" => {
                format!("      expect({header_expr}).not_to be_nil\n")
            }
            "<<absent>>" => {
                format!("      expect({header_expr}).to be_nil\n")
            }
            "<<uuid>>" => {
                format!(
                    "      expect({header_expr}).to match(/\\A[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\\z/i)\n"
                )
            }
            literal => {
                let ruby_val = ruby_string_literal(literal);
                format!("      expect({header_expr}).to eq({ruby_val})\n")
            }
        };
        out.push_str(&assertion);
    }

    /// Emit a full JSON body equality assertion.
    ///
    /// Plain string bodies are compared as raw text; structured bodies are parsed
    /// with `JSON.parse` and compared as Ruby Hash/Array values.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::String(s) => {
                let ruby_val = ruby_string_literal(s);
                out.push_str(&format!("      expect({response_var}.body).to eq({ruby_val})\n"));
            }
            _ => {
                let ruby_val = json_to_ruby(expected);
                out.push_str(&format!(
                    "      _body = {response_var}.body && !{response_var}.body.empty? ? JSON.parse({response_var}.body) : nil\n"
                ));
                out.push_str(&format!("      expect(_body).to eq({ruby_val})\n"));
            }
        }
    }

    /// Emit partial body assertions: one `expect(_body[key]).to eq(val)` per field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            out.push_str(&format!("      _body = JSON.parse({response_var}.body)\n"));
            for (key, val) in obj {
                let ruby_key = ruby_string_literal(key);
                let ruby_val = json_to_ruby(val);
                out.push_str(&format!("      expect(_body[{ruby_key}]).to eq({ruby_val})\n"));
            }
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` against the
    /// parsed body's `errors` array.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        for err in errors {
            let msg_lit = ruby_string_literal(&err.msg);
            out.push_str(&format!("      _body = JSON.parse({response_var}.body)\n"));
            out.push_str("      _errors = _body['errors'] || []\n");
            out.push_str(&format!(
                "      expect(_errors.map {{ |e| e['msg'] }}).to include({msg_lit})\n"
            ));
        }
    }
}

/// Render an RSpec example for an HTTP server test fixture via the shared driver.
///
/// Delegates to [`client::http_call::render_http_test`] after handling the one
/// Ruby-specific pre-condition: HTTP 101 (WebSocket upgrade) cannot be exercised
/// via `Net::HTTP` and is emitted as a pending `it` block directly.
pub(super) fn render_http_example(out: &mut String, fixture: &Fixture) {
    // HTTP 101 (WebSocket upgrade) cannot be tested via Net::HTTP.
    // Emit the skip block directly rather than pushing a skip directive through
    // the shared driver, which would require a full `fixture.skip` entry.
    if fixture
        .http
        .as_ref()
        .is_some_and(|h| h.expected_response.status_code == 101)
    {
        if let Some(http) = fixture.http.as_ref() {
            let description_literal = ruby_string_literal(&fixture.description);
            let method = http.request.method.to_uppercase();
            let path = &http.request.path;
            let rendered = crate::e2e::template_env::render(
                "ruby/http_101_skip.jinja",
                minijinja::context! {
                    method => method,
                    path => path,
                    description => description_literal,
                },
            );
            out.push_str(&rendered);
        }
        return;
    }

    client::http_call::render_http_test(out, &RubyTestClientRenderer, fixture);
}

/// Render an RSpec example for an HTTP server-pattern test fixture (SUT harness).
///
/// Uses the server-pattern template to hit the actual SUT harness listening on
/// a configured host:port, rather than the shared mock-server driver.
pub(super) fn render_http_example_sut(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    // HTTP 101 (WebSocket upgrade) cannot be tested via Net::HTTP.
    if http.expected_response.status_code == 101 {
        let description_literal = ruby_string_literal(&fixture.description);
        let method = http.request.method.to_uppercase();
        let path = &http.request.path;
        let rendered = crate::e2e::template_env::render(
            "ruby/http_101_skip.jinja",
            minijinja::context! {
                method => method,
                path => path,
                description => description_literal,
            },
        );
        out.push_str(&rendered);
        return;
    }

    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };
    let description_literal = ruby_string_literal(&desc_with_period);

    // Build request headers dict literal
    let mut header_entries: Vec<String> = http
        .request
        .headers
        .iter()
        .map(|(k, v)| format!("      '{}' => '{}',", k, v))
        .collect();
    header_entries.sort();
    let headers_ruby = if header_entries.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n    }}", header_entries.join("\n"))
    };

    let method = http.request.method.to_uppercase();
    let method_class = http_method_class(&method);
    let path = format!("/fixtures/{}{}", &fixture.id, &http.request.path);

    // Detect content-type so the renderer can decide between JSON-encoded and
    // raw (form-urlencoded / multipart / plain text) body emission.
    let content_type_lower = http
        .request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_else(|| {
            http.request
                .content_type
                .as_ref()
                .map(|ct| ct.to_ascii_lowercase())
                .unwrap_or_default()
        });
    let is_multipart = content_type_lower
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("multipart/form-data"));

    // Synthesize multipart body if content-type is multipart/form-data and there is no explicit body
    let multipart_body_ruby = if is_multipart && http.request.body.is_none() {
        if let Some(schema) = &http.handler.body_schema {
            if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
                if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                    synthesize_multipart_body(props)
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Determine request body.
    // When the fixture body is a JSON string (e.g. URL-encoded form data like
    // "a=1&b=2"), it must be sent as a raw string, NOT wrapped in JSON.dump().
    // Detect this by checking whether the body JSON value is a string.
    // Synthesized multipart bodies are already emitted as raw string literals.
    let (has_body, body_ruby, is_raw_body) = if let Some(body) = &http.request.body {
        let is_raw = body.is_string();
        (true, json_to_ruby(body), is_raw)
    } else if is_multipart && !multipart_body_ruby.is_empty() {
        (true, multipart_body_ruby.clone(), true)
    } else {
        (false, String::new(), false)
    };

    // Determine response body expectations
    let (has_text_body, text_ruby) = if let Some(serde_json::Value::String(s)) = &http.expected_response.body {
        (true, ruby_string_literal(s))
    } else {
        (false, String::new())
    };

    let (has_json_body, json_ruby) = if let Some(body) = &http.expected_response.body {
        if !(body.is_null() || body.is_string() && body.as_str() == Some("")) {
            if !matches!(body, serde_json::Value::String(_)) {
                (true, json_to_ruby(body))
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
                    let ruby_val = json_to_ruby(val);
                    minijinja::context! {
                        key => key,
                        value => ruby_val,
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

    // Build header assertions
    let mut header_assertions: Vec<minijinja::Value> = Vec::new();
    let mut header_names: Vec<String> = http.expected_response.headers.keys().cloned().collect();
    header_names.sort();

    for name in header_names {
        let value = &http.expected_response.headers[&name];
        header_assertions.push(minijinja::context! {
            name => name,
            assertion_type => "eq",
            value => value,
        });
    }

    // Build validation error expectations
    let (has_validation_errors, validation_errors) = if http.expected_response.status_code == 422 {
        if let Some(body) = &http.expected_response.body {
            if let Some(obj) = body.as_object() {
                if let Some(errs) = obj.get("errors").and_then(|v| v.as_array()) {
                    let ve: Vec<minijinja::Value> = errs
                        .iter()
                        .filter_map(|err| {
                            let loc = err.get("loc").and_then(|l| l.as_array())?;
                            let msg = err.get("msg").and_then(|m| m.as_str())?;
                            // Produce comma-separated element literals so the template can
                            // wrap them in `[...]` to form a valid Ruby array literal.
                            // e.g. loc = ["query", "limit"] → loc_ruby = "'query', 'limit'"
                            // Template: `[{{ loc_ruby }}]` → `['query', 'limit']`
                            let loc_ruby = loc.iter().map(json_to_ruby).collect::<Vec<_>>().join(", ");
                            // Escape single quotes for embedding in a Ruby single-quoted string.
                            // `ruby_string_literal` would choose double-quotes, but the template
                            // embeds the value directly inside `'...'`, so we must escape `'` → `\'`.
                            let escaped = escape_ruby_single(msg);
                            Some(minijinja::context! {
                                loc_ruby => loc_ruby,
                                escaped_msg => escaped,
                            })
                        })
                        .collect();
                    (true, ve)
                } else {
                    (false, Vec::new())
                }
            } else {
                (false, Vec::new())
            }
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    let rendered = crate::e2e::template_env::render(
        "ruby/http_test_sut.jinja",
        minijinja::context! {
            fn_name => fn_name,
            description => description_literal,
            method => method,
            method_class => method_class,
            path => path,
            headers_ruby => headers_ruby,
            has_body => has_body,
            body_ruby => body_ruby,
            is_raw_body => is_raw_body,
            expected_status => http.expected_response.status_code,
            has_text_body => has_text_body,
            text_ruby => text_ruby,
            has_json_body => has_json_body,
            json_ruby => json_ruby,
            has_partial_body => has_partial_body,
            partial_body_checks => partial_body_checks,
            header_assertions => header_assertions,
            has_validation_errors => has_validation_errors,
            validation_errors => validation_errors,
        },
    );
    out.push_str(&rendered);
}

/// Synthesize a multipart body from the handler's body schema properties.
///
/// For each property, generate a form-data part with placeholder content.
/// Binary (format="binary") properties get a filename and Content-Type header.
/// Text properties are simple form fields.
fn synthesize_multipart_body(props: &serde_json::Map<String, serde_json::Value>) -> String {
    const BOUNDARY: &str = "alef-boundary";
    let mut body = String::new();

    for (prop_name, prop_schema) in props {
        let is_binary = prop_schema
            .get("format")
            .and_then(|f| f.as_str())
            .is_some_and(|f| f == "binary");

        body.push_str(&format!(
            "--{}\r\nContent-Disposition: form-data; name=\"{}\"",
            BOUNDARY, prop_name
        ));

        if is_binary {
            body.push_str(&format!(
                "; filename=\"{}.txt\"\r\nContent-Type: text/plain\r\n\r\n",
                prop_name
            ));
            body.push_str("placeholder content");
        } else {
            body.push_str("\r\n\r\nsample");
        }

        body.push_str("\r\n");
    }

    body.push_str(&format!("--{}--\r\n", BOUNDARY));

    // Use ruby_string_literal to properly escape the multipart body.
    // This converts actual \r\n characters to escaped \\r\\n in the string literal.
    ruby_string_literal(&body)
}

/// Convert an uppercase HTTP method string to Ruby's Net::HTTP class name.
/// Ruby uses title-cased names: Get, Post, Put, Delete, Patch, Head, Options, Trace.
pub(super) fn http_method_class(method: &str) -> String {
    let mut chars = method.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}
