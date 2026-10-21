//! Mock-server setup rendering for Rust e2e test functions.

use std::fmt::Write as FmtWrite;

use crate::e2e::config::E2eConfig;
use crate::e2e::escape::rust_raw_string;
use crate::e2e::fixture::Fixture;

/// Emit mock server setup lines into a test function body.
///
/// Builds `MockRoute` objects from the fixture's `mock_response` schema or the
/// `input.mock_responses` route-array schema.
/// The resulting `mock_server` variable is in scope for the rest of the test function.
///
/// `var_name` controls the local binding name (e.g. `"mock_server"` when the rest of
/// the test body references `mock_server.url`, `"_mock_server"` when the server only
/// needs to be kept alive via Drop — typical for error-path fixtures that intentionally
/// never read the URL). The underscore prefix silences `-D unused_variables` without
/// dropping the server early.
pub fn render_mock_server_setup(out: &mut String, fixture: &Fixture, e2e_config: &E2eConfig, var_name: &str) {
    // Prefer the route-array schema when present.
    let mut routes = Vec::new();

    if let Some(mock_responses) = fixture.input.get("mock_responses").and_then(|v| v.as_array()) {
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
                } else if let Some(body_file) = obj.get("body_file").and_then(|v| v.as_str()) {
                    let content = read_mock_body_file(&e2e_config.fixtures, body_file, &fixture.id);
                    rust_raw_string(&content)
                } else {
                    rust_raw_string("{}")
                };

                let delay_ms = obj.get("delay_ms").and_then(|v| v.as_u64());

                routes.push((path, method, status, body_str, headers, delay_ms));
            }
        }
    } else if let Some(mock) = fixture.mock_response.as_ref() {
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
            let _ = writeln!(
                out,
                "        stream_chunk_delay_ms: {},",
                option_u64_literal(mock.stream_chunk_delay_ms)
            );
            let _ = writeln!(out, "    }};");
            let _ = writeln!(out, "    let {var_name} = MockServer::start(vec![mock_route]).await;");
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
        let _ = writeln!(out, "        stream_chunk_delay_ms: None,");
        let _ = writeln!(out, "    }};");
        let _ = writeln!(out, "    let {var_name} = MockServer::start(vec![mock_route]).await;");
    } else {
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
            let _ = writeln!(out, "        stream_chunk_delay_ms: None,");
            let _ = writeln!(out, "    }});");
        }
        let _ = writeln!(out, "    let {var_name} = MockServer::start(mock_routes).await;");
    }
}

/// Rust source for an `Option<u64>` value.
fn option_u64_literal(value: Option<u64>) -> String {
    match value {
        Some(ms) => format!("Some({ms})"),
        None => "None".to_string(),
    }
}

/// Read a `body_file`-referenced fixture response body from disk at generation
/// time, mirroring the standalone runtime binary's own resolution order
/// (`mock_server/binary.rs`'s `as_routes`): `<fixtures_dir>/responses/<file>`
/// first, then `<fixtures_dir>/<file>`.
///
/// ~keep `body_file` used to be silently ignored by the in-process mock server —
/// it always served `{}` regardless of what the fixture declared, while the
/// standalone binary (which resolves and reads the file at its own runtime)
/// served the real content. Any test relying on the real body passed against the
/// standalone binary and silently exercised the wrong response in-process.
/// Codegen already runs with real filesystem access (unlike a deployed test
/// binary), so reading the file here at generation time closes that gap for text
/// bodies. A body that can't be read, or isn't valid UTF-8 (can't be embedded as
/// a Rust string literal), fails generation instead of silently reverting to `{}`.
fn read_mock_body_file(fixtures_dir: &str, body_file: &str, fixture_id: &str) -> String {
    let base = std::path::Path::new(fixtures_dir);
    let candidates = [base.join("responses").join(body_file), base.join(body_file)];
    for candidate in &candidates {
        if let Ok(bytes) = std::fs::read(candidate) {
            return String::from_utf8(bytes).unwrap_or_else(|_| {
                panic!(
                    "Rust e2e generator: fixture `{fixture_id}` declares body_file `{body_file}` \
                     ({}), but its content is not valid UTF-8 — the in-process mock server can \
                     only serve it embedded as a Rust string literal. Use body_inline for binary \
                     responses in this fixture, or exclude `rust` from its languages.",
                    candidate.display()
                )
            });
        }
    }
    panic!(
        "Rust e2e generator: fixture `{fixture_id}` declares body_file `{body_file}`, but it \
         could not be read (tried `{}` and `{}`). The in-process mock server would otherwise \
         silently serve `{{}}` instead of the declared body.",
        candidates[0].display(),
        candidates[1].display()
    );
}

#[cfg(test)]
mod body_file_tests {
    use super::render_mock_server_setup;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::Fixture;

    fn fixture_with_body_file(id: &str, path: &str, body_file: &str) -> Fixture {
        let input = serde_json::json!({
            "mock_responses": [
                { "path": path, "method": "GET", "status_code": 200, "body_file": body_file }
            ]
        });
        Fixture {
            id: id.to_string(),
            input,
            ..Fixture::default()
        }
    }

    /// Regression test for alef task #81: a `body_file` route must embed the
    /// real file content, not the `{}` placeholder the in-process mock server
    /// used to always emit regardless of what the fixture declared.
    #[test]
    fn body_file_content_is_read_and_embedded() {
        let dir = tempfile::tempdir().expect("temp fixtures dir");
        std::fs::write(dir.path().join("payload.json"), r#"{"greeting":"hello"}"#).expect("write body file");
        let fixture = fixture_with_body_file("serves_payload", "/fixtures/serves_payload", "payload.json");
        let e2e_config = E2eConfig {
            fixtures: dir.path().to_string_lossy().to_string(),
            ..E2eConfig::default()
        };

        let mut out = String::new();
        render_mock_server_setup(&mut out, &fixture, &e2e_config, "mock_server");

        assert!(
            out.contains(r#"{"greeting":"hello"}"#),
            "expected the real body_file content embedded, got:\n{out}"
        );
        assert!(
            !out.contains("body: r#\"{}\"#"),
            "must not fall back to the `{{}}` placeholder, got:\n{out}"
        );
    }

    /// The `responses/` subdirectory takes precedence over the fixtures root,
    /// matching the standalone binary's own resolution order.
    #[test]
    fn body_file_prefers_the_responses_subdirectory() {
        let dir = tempfile::tempdir().expect("temp fixtures dir");
        std::fs::create_dir(dir.path().join("responses")).expect("mkdir responses");
        std::fs::write(dir.path().join("responses").join("payload.json"), "from-responses-dir")
            .expect("write nested body file");
        std::fs::write(dir.path().join("payload.json"), "from-fixtures-root").expect("write root body file");
        let fixture = fixture_with_body_file("prefers_nested", "/fixtures/prefers_nested", "payload.json");
        let e2e_config = E2eConfig {
            fixtures: dir.path().to_string_lossy().to_string(),
            ..E2eConfig::default()
        };

        let mut out = String::new();
        render_mock_server_setup(&mut out, &fixture, &e2e_config, "mock_server");

        assert!(out.contains("from-responses-dir"), "got:\n{out}");
        assert!(!out.contains("from-fixtures-root"), "got:\n{out}");
    }

    /// Positive control: `body_inline` is unaffected by this change and keeps
    /// rendering exactly as before.
    #[test]
    fn body_inline_is_unaffected() {
        let input = serde_json::json!({
            "mock_responses": [
                { "path": "/fixtures/inline_smoke", "method": "GET", "status_code": 200, "body_inline": "hi" }
            ]
        });
        let fixture = Fixture {
            id: "inline_smoke".to_string(),
            input,
            ..Fixture::default()
        };
        let e2e_config = E2eConfig::default();

        let mut out = String::new();
        render_mock_server_setup(&mut out, &fixture, &e2e_config, "mock_server");

        assert!(out.contains(r##"body: r#"hi"#.to_string(),"##), "got:\n{out}");
    }

    #[test]
    #[should_panic(expected = "fixture `missing_body_file`")]
    fn unreadable_body_file_fails_loudly_instead_of_falling_back() {
        let dir = tempfile::tempdir().expect("temp fixtures dir");
        let fixture = fixture_with_body_file("missing_body_file", "/fixtures/missing_body_file", "nope.json");
        let e2e_config = E2eConfig {
            fixtures: dir.path().to_string_lossy().to_string(),
            ..E2eConfig::default()
        };

        let mut out = String::new();
        render_mock_server_setup(&mut out, &fixture, &e2e_config, "mock_server");
    }
}
