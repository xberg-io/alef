use super::*;

struct ZigTestClientRenderer;

impl client::TestClientRenderer for ZigTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "zig"
    }

    /// `std.http.Client` handles gzip but rejects an encoding it does not know while parsing
    /// the response head, so an unsupported encoding fails inside `receiveHead` before a body
    /// exists to assert on. No brotli decoder exists in the standard library.
    fn decodable_content_encodings(&self) -> &'static [&'static str] {
        &["gzip"]
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        if let Some(reason) = skip_reason {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
            let _ = writeln!(out, "    // skipped: {reason}");
            let _ = writeln!(out, "    return error.SkipZigTest;");
        } else {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_id = ctx.path.trim_start_matches("/fixtures/");
        // Escape curly braces in fixture_id so they don't get interpreted as format specs by bufPrint.
        let escaped_fixture_id = fixture_id.replace('{', "{{").replace('}', "}}");

        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");

        let _ = writeln!(out, "    var url_buf: [512]u8 = undefined;");
        let _ = writeln!(
            out,
            "    const url = try std.fmt.bufPrint(&url_buf, \"{{s}}/fixtures/{escaped_fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
        );

        // Headers
        if !ctx.headers.is_empty() {
            let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
            header_pairs.sort_by_key(|(k, _)| k.as_str());
            let _ = writeln!(out, "    const headers = [_]std.http.Header{{");
            for (k, v) in &header_pairs {
                let ek = escape_zig(k);
                let ev = escape_zig(v);
                let _ = writeln!(out, "        .{{ .name = \"{ek}\", .value = \"{ev}\" }},");
            }
            let _ = writeln!(out, "    }};");
        }

        let headers_arg = if ctx.headers.is_empty() { "&.{}" } else { "&headers" };
        let has_body = ctx.body.is_some();
        // zig 0.16's std.http.Client.fetch asserts in `sendBodilessUnflushed` when a
        // body-requiring method (POST/PUT/PATCH) is sent without a `.payload`. The mock server
        // replays by fixture id and ignores the request body, so emit an empty payload for such
        // methods when the fixture itself carries no body, avoiding the `reached unreachable` panic.
        let method_requires_body = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
        let emit_payload = has_body || method_requires_body;

        // Body
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_zig(&json_str);
            let _ = writeln!(out, "    const body_bytes: []const u8 = \"{escaped}\";");
        } else if emit_payload {
            let _ = writeln!(out, "    const body_bytes: []const u8 = \"\";");
        }

        // zig 0.16: std.http.Client requires an `io: Io` (the new std.Io abstraction), and
        // the response body is captured through a std.Io.Writer rather than the removed
        // `response_storage`/ArrayList API. A blocking `Io.Threaded` instance backs the client.
        let _ = writeln!(out, "    var threaded = std.Io.Threaded.init(allocator, .{{}});");
        let _ = writeln!(out, "    defer threaded.deinit();");
        let _ = writeln!(out, "    const io = threaded.io();");
        let _ = writeln!(
            out,
            "    var http_client = std.http.Client{{ .allocator = allocator, .io = io }};"
        );
        let _ = writeln!(out, "    defer http_client.deinit();");
        let _ = writeln!(out, "    var response_body = std.Io.Writer.Allocating.init(allocator);");
        let _ = writeln!(out, "    defer response_body.deinit();");

        let method_zig = match method.as_str() {
            "GET" => ".GET",
            "POST" => ".POST",
            "PUT" => ".PUT",
            "DELETE" => ".DELETE",
            "PATCH" => ".PATCH",
            "HEAD" => ".HEAD",
            "OPTIONS" => ".OPTIONS",
            _ => ".GET",
        };

        let payload_field = if emit_payload { ", .payload = body_bytes" } else { "" };
        // `.keep_alive = false` sends `Connection: close` so the server closes the socket after
        // the response. Without it, the std.http.Client blocks reading a kept-alive connection
        // waiting for data/EOF that never arrives — under the e2e load this deadlocks the test
        // binaries (0% CPU, hundreds of lingering connections). Each test uses a fresh client,
        // so there is no keep-alive reuse benefit to preserve.
        let _ = writeln!(
            out,
            "    const {rv} = try http_client.fetch(.{{ .location = .{{ .url = url }}, .method = {method_zig}, .extra_headers = {headers_arg}{payload_field}, .keep_alive = false, .redirect_behavior = .unhandled, .response_writer = &response_body.writer }});",
            rv = ctx.response_var,
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "    try testing.expectEqual(@as(u10, {status}), @intFromEnum({response_var}.status));"
        );
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let ename = escape_zig(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is present (header inspection not yet implemented)"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is absent (header inspection not yet implemented)"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' matches UUID pattern (header inspection not yet implemented)"
                );
            }
            exact => {
                let evalue = escape_zig(exact);
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' == \"{evalue}\" (header inspection not yet implemented)"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        // A string-valued expected body is a plain-text response (e.g. `text/plain` "foo bar 10"),
        // so compare the raw string contents — JSON-serializing it would wrap it in quotes and
        // never match the unquoted response bytes. Structured bodies keep their serialized form.
        let escaped = match expected {
            serde_json::Value::String(s) => escape_zig(s),
            other => escape_zig(&serde_json::to_string(other).unwrap_or_default()),
        };
        let _ = writeln!(
            out,
            "    try testing.expectEqualStrings(\"{escaped}\", response_body.written());"
        );
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let ekey = escape_zig(key);
                let eval = escape_zig(&serde_json::to_string(val).unwrap_or_default());
                let _ = writeln!(
                    out,
                    "    // assert body contains field \"{ekey}\" = \"{eval}\" (partial JSON not yet implemented)"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
    ) {
        for ve in errors {
            let loc = ve.loc.join(".");
            let escaped_loc = escape_zig(&loc);
            let escaped_msg = escape_zig(&ve.msg);
            let _ = writeln!(
                out,
                "    // assert validation error at \"{escaped_loc}\": \"{escaped_msg}\" (not yet implemented)"
            );
        }
    }
}

/// Render a Zig `test "..." { ... }` block for an HTTP server fixture.
///
/// Delegates to the shared [`client::http_call::render_http_test`] driver via
/// [`ZigTestClientRenderer`].
pub(super) fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &ZigTestClientRenderer, fixture);
}
