use super::*;

pub(in crate::e2e::codegen::typescript::test_file) fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    let test_name = sanitize_ident(&fixture.id);
    // Escape backslashes and double quotes for use in a double-quoted JS string.
    let description = fixture.description.replace('\\', "\\\\").replace('"', "\\\"");

    if http.expected_response.status_code == 101 {
        return;
    }

    let method = http.request.method.to_uppercase();

    // Detect content-type so the renderer can decide between JSON-encoded and
    // raw (form-urlencoded / multipart) body emission.
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
    let is_form_body = content_type_lower
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("application/x-www-form-urlencoded"));
    let is_multipart = content_type_lower
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("multipart/form-data"));

    // If multipart but no request body, synthesize from body_schema
    let effective_body = if is_multipart && http.request.body.is_none() && http.handler.body_schema.is_some() {
        // Synthesize a minimal multipart body from the schema
        Some(synthesize_multipart_body_from_schema(&http.handler.body_schema))
    } else {
        http.request.body.clone()
    };

    // Determine body shape. A raw pre-encoded body (form-urlencoded or a
    // synthesized/pre-encoded multipart string) is sent as UTF-8 bytes verbatim;
    // every other body is JSON.stringify'd and needs an application/json
    // Content-Type. A plain JSON object declared under a multipart param must NOT
    // receive the synthesized multipart boundary Content-Type — that makes the
    // server's multipart parser reject the JSON body with 400 before the handler
    // runs. Mirrors the Python generator's body_is_bytes_literal/body_is_string
    // branch logic.
    let has_body = effective_body.is_some();
    let body_is_string = effective_body
        .as_ref()
        .is_some_and(|b| matches!(b, serde_json::Value::String(_)));
    let body_is_raw = (is_form_body || is_multipart) && body_is_string;
    let emit_multipart_boundary = is_multipart && body_is_string;
    let has_content_type_header = http
        .request
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    let needs_json_content_type = has_body && !body_is_raw && !has_content_type_header;

    let has_headers = !http.request.headers.is_empty() || needs_json_content_type || emit_multipart_boundary;

    // Build the body entry if present.
    let body_entry: Option<String> = effective_body.as_ref().map(|body| {
        let js_body = json_to_js(body);
        // Raw form-urlencoded/multipart string bodies are sent as UTF-8 bytes
        // as-is; everything else is JSON.stringify'd.
        if body_is_raw {
            format!("body: Buffer.from({js_body}, 'utf-8')")
        } else {
            format!("body: JSON.stringify({js_body})")
        }
    });

    // Build the fetch init object. Use multi-line form when headers or body
    // are present so the output matches what oxfmt would produce; use inline
    // form for the simple method+redirect-only case.
    let fetch_init: String = if has_headers || body_entry.is_some() {
        // Multi-line object: each entry on its own line, trailing commas.
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("      method: \"{method}\","));
        lines.push("      redirect: \"manual\",".to_string());
        if has_headers {
            let mut header_lines: Vec<String> = http
                .request
                .headers
                .iter()
                // Skip the fixture Content-Type only when we replace it with the
                // synthesized multipart boundary header below.
                .filter(|(k, _)| !(emit_multipart_boundary && k.eq_ignore_ascii_case("content-type")))
                .map(|(k, v)| {
                    let expanded_v = expand_fixture_templates(v);
                    format!("        \"{}\": \"{}\",", escape_js(k), escape_js(&expanded_v))
                })
                .collect();
            if needs_json_content_type {
                header_lines.push("        \"Content-Type\": \"application/json\",".to_string());
            }
            if emit_multipart_boundary {
                // Real multipart string body: add the Content-Type with our boundary.
                header_lines
                    .push("        \"Content-Type\": \"multipart/form-data; boundary=alef-boundary\",".to_string());
            }
            lines.push("      headers: {".to_string());
            lines.extend(header_lines);
            lines.push("      },".to_string());
        }
        if let Some(body) = body_entry {
            lines.push(format!("      {body},"));
        }
        format!("{{\n{}\n    }}", lines.join("\n"))
    } else {
        // Inline: no headers, no body — only method and redirect.
        format!("{{ method: \"{method}\", redirect: \"manual\" }}")
    };

    let init_str = fetch_init;
    // Server-pattern: construct path as /fixtures/{fixture_id}{request_path}
    let path = format!("/fixtures/{}{}", fixture.id, http.request.path);

    let status = http.expected_response.status_code;

    // Determine body type and prepare context
    let (has_text_body, text_body) = if let Some(expected_body) = &http.expected_response.body {
        if !(expected_body.is_null() || expected_body.is_string() && expected_body.as_str() == Some("")) {
            if let serde_json::Value::String(s) = expected_body {
                (true, escape_js(s))
            } else {
                (false, String::new())
            }
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let (has_json_body, json_val) = if let Some(expected_body) = &http.expected_response.body {
        if !(expected_body.is_null() || expected_body.is_string() && expected_body.as_str() == Some("")) {
            if let serde_json::Value::String(_) = expected_body {
                (false, String::new())
            } else {
                // Use multi-line form for objects so the output is stable under
                // oxfmt (formatters leave properly-indented multi-line objects
                // unchanged). Scalar and array values stay inline.
                (true, json_to_js_multiline(expected_body, 4))
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
                    crate::alef_context! {
                        key => escape_js(key),
                        js_val => json_to_js(val),
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
    for (header_name, header_value) in &http.expected_response.headers {
        let lower_name = header_name.to_lowercase();
        if lower_name == "content-encoding" {
            continue;
        }
        let escaped_name = escape_js(&lower_name);
        let (assertion_type, value) = match header_value.as_str() {
            "<<present>>" => ("present", String::new()),
            "<<absent>>" => ("absent", String::new()),
            "<<uuid>>" => ("uuid", String::new()),
            exact => ("exact", escape_js(exact)),
        };
        header_assertions.push(crate::alef_context! {
            name => escaped_name,
            assertion_type => assertion_type,
            value => value,
        });
    }

    // Build validation error assertions
    let body_has_content = matches!(&http.expected_response.body, Some(v)
        if !(v.is_null() || (v.is_string() && v.as_str() == Some(""))));
    let (has_validation_errors, validation_errors) =
        if let Some(validation_errors) = &http.expected_response.validation_errors {
            if !validation_errors.is_empty() && !body_has_content {
                let errors: Vec<minijinja::Value> = validation_errors
                    .iter()
                    .map(|ve| {
                        let loc_js: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_js(s))).collect();
                        let loc_str = loc_js.join(", ");
                        let expanded_msg = expand_fixture_templates(&ve.msg);
                        let escaped_msg = escape_js(&expanded_msg);
                        crate::alef_context! {
                            loc_js => loc_str,
                            escaped_msg => escaped_msg,
                        }
                    })
                    .collect();
                (true, errors)
            } else {
                (false, Vec::new())
            }
        } else {
            (false, Vec::new())
        };

    let ctx = crate::alef_context! {
        test_name => test_name,
        description => description,
        method => method,
        init_str => init_str,
        path => path,
        expected_status => status,
        has_text_body => has_text_body,
        text_body => text_body,
        has_json_body => has_json_body,
        json_val => json_val,
        has_partial_body => has_partial_body,
        partial_body_checks => partial_body_checks,
        header_assertions => header_assertions,
        has_validation_errors => has_validation_errors,
        validation_errors => validation_errors,
        is_multipart => is_multipart,
    };
    let rendered = crate::e2e::template_env::render("typescript/http_test.jinja", ctx);
    out.push_str(&rendered);
}

/// Synthesize a minimal multipart/form-data body from a JSON schema.
/// RFC 2388 requires boundaries to be prefixed with CRLF and the final boundary
/// to end with CRLF followed by `--` (i.e., `\r\n--boundary--\r\n`).
fn synthesize_multipart_body_from_schema(schema: &Option<serde_json::Value>) -> serde_json::Value {
    let Some(schema_val) = schema else {
        return serde_json::Value::String(String::new());
    };

    let mut body = String::new();
    let boundary = "alef-boundary";

    if let Some(props) = schema_val.get("properties").and_then(|p| p.as_object()) {
        for (key, prop_schema) in props {
            // Check if this is a binary/file field
            let is_binary = prop_schema
                .get("format")
                .and_then(|f| f.as_str())
                .map(|f| f == "binary")
                .unwrap_or(false);

            body.push_str(&format!("--{}\r\n", boundary));

            if is_binary {
                body.push_str(&format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}.txt\"\r\nContent-Type: text/plain\r\n\r\n<file content>",
                    escape_js(key),
                    escape_js(key)
                ));
            } else {
                body.push_str(&format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\ntest_value",
                    escape_js(key)
                ));
            }

            body.push_str("\r\n");
        }
    }

    // RFC 2388: final boundary must be terminated with `--` and CRLF
    body.push_str(&format!("--{}--\r\n", boundary));
    serde_json::Value::String(body)
}
