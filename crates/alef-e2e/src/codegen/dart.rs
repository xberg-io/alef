//! Dart e2e test generator using package:test and package:http.
//!
//! Generates `e2e/dart/test/<category>_test.dart` files from JSON fixtures.
//! HTTP fixtures hit the mock server at `MOCK_SERVER_URL/fixtures/<id>`.
//! Non-HTTP fixtures without a dart-specific call override emit a skip stub.

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::fixture::{Fixture, FixtureGroup, HttpFixture};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::pub_dev;
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Dart e2e code generator.
pub struct DartE2eCodegen;

impl E2eCodegen for DartE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve package config.
        let dart_pkg = e2e_config.resolve_package("dart");
        let pkg_name = dart_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.dart_pubspec_name());
        let pkg_path = dart_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/dart".to_string());
        let pkg_version = dart_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate pubspec.yaml with http dependency for HTTP client tests.
        files.push(GeneratedFile {
            path: output_base.join("pubspec.yaml"),
            content: render_pubspec(&pkg_name, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        let test_base = output_base.join("test");

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.dart", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &active, e2e_config, lang);
            files.push(GeneratedFile {
                path: test_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "dart"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_pubspec(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let test_ver = pub_dev::TEST_PACKAGE;
    let http_ver = pub_dev::HTTP_PACKAGE;

    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!("  {pkg_name}: ^{pkg_version}")
        }
        crate::config::DependencyMode::Local => {
            format!("  {pkg_name}:\n    path: {pkg_path}")
        }
    };

    format!(
        r#"name: e2e_dart
version: 0.1.0
publish_to: none

environment:
  sdk: ">=3.0.0 <4.0.0"

dependencies:
{dep_block}

dev_dependencies:
  test: {test_ver}
  http: {http_ver}
"#
    )
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    lang: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));

    // Check if any fixture needs the http package (HTTP server tests).
    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());

    let _ = writeln!(out, "import 'package:test/test.dart';");
    let _ = writeln!(out, "import 'dart:io';");
    if has_http_fixtures {
        let _ = writeln!(out, "import 'package:http/http.dart' as http;");
        let _ = writeln!(out, "import 'dart:convert';");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "void main() {{");

    for fixture in fixtures {
        render_test_case(&mut out, fixture, e2e_config, lang);
    }

    let _ = writeln!(out, "}}");
    out
}

fn render_test_case(out: &mut String, fixture: &Fixture, e2e_config: &E2eConfig, lang: &str) {
    // HTTP fixtures: hit the mock server.
    if let Some(http) = &fixture.http {
        render_http_test_case(out, fixture, http);
        return;
    }

    // Non-HTTP fixtures: check if there is a dart-specific call override.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let call_overrides = call_config.overrides.get(lang);

    if call_overrides.is_none() {
        // No dart-specific call override — emit a skip stub.
        render_skip_stub(out, fixture);
        return;
    }

    // Has a dart call override — render a call-based test.
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let description = escape_dart(&fixture.description);
    let is_async = call_config.r#async;

    if is_async {
        let _ = writeln!(out, "  test('{description}', () async {{");
    } else {
        let _ = writeln!(out, "  test('{description}', () {{");
    }

    if is_async {
        let _ = writeln!(out, "    final {result_var} = await {function_name}();");
    } else {
        let _ = writeln!(out, "    final {result_var} = {function_name}();");
    }

    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
}

/// Render an HTTP server test using package:http against MOCK_SERVER_URL.
///
/// The mock server registers each fixture at `/fixtures/<fixture_id>` and returns
/// the pre-canned response. Tests send the correct HTTP method and headers to that
/// endpoint.
fn render_http_test_case(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    let description = escape_dart(&fixture.description);
    let request = &http.request;
    let expected = &http.expected_response;
    let method = request.method.to_uppercase();
    let fixture_id = &fixture.id;
    let expected_status = expected.status_code;

    // Skip 101 Switching Protocols — Dart's http client cannot handle protocol-switch responses.
    if expected_status == 101 {
        let _ = writeln!(out, "  test('{description}', () {{");
        let _ = writeln!(
            out,
            "    markTestSkipped('Skipped: Dart http client cannot handle 101 Switching Protocols responses');"
        );
        let _ = writeln!(out, "  }});");
        let _ = writeln!(out);
        return;
    }

    let _ = writeln!(out, "  test('{description}', () async {{");
    let _ = writeln!(
        out,
        "    final baseUrl = Platform.environment['MOCK_SERVER_URL'] ?? 'http://localhost:8080';"
    );
    let _ = writeln!(
        out,
        "    final uri = Uri.parse('$baseUrl/fixtures/{fixture_id}');"
    );

    // Dart's http package does not allow setting these headers directly.
    const DART_RESTRICTED_HEADERS: &[&str] = &["content-length", "host", "transfer-encoding"];

    // Build headers map.
    let _ = writeln!(out, "    final headers = <String, String>{{");
    let content_type = request.content_type.as_deref().unwrap_or("application/json");
    if request.body.is_some() {
        let _ = writeln!(out, "      'content-type': '{content_type}',");
    }
    for (name, value) in &request.headers {
        if DART_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
            continue;
        }
        let escaped_name = escape_dart(name);
        let escaped_value = escape_dart(value);
        let _ = writeln!(out, "      '{escaped_name}': '{escaped_value}',");
    }
    // Add cookies as Cookie header.
    if !request.cookies.is_empty() {
        let cookie_str: Vec<String> = request
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let cookie_header = escape_dart(&cookie_str.join("; "));
        let _ = writeln!(out, "      'cookie': '{cookie_header}',");
    }
    let _ = writeln!(out, "    }};");

    // Build body if present.
    let body_expr = if let Some(body) = &request.body {
        let json_str = serde_json::to_string(body).unwrap_or_default();
        let escaped = escape_dart(&json_str);
        format!("'{escaped}'")
    } else {
        String::new()
    };

    // Send the request using the appropriate http method.
    match method.as_str() {
        "GET" => {
            let _ = writeln!(
                out,
                "    final response = await http.get(uri, headers: headers);"
            );
        }
        "POST" => {
            let body_arg = if body_expr.is_empty() {
                String::new()
            } else {
                format!(", body: {body_expr}")
            };
            let _ = writeln!(
                out,
                "    final response = await http.post(uri, headers: headers{body_arg});"
            );
        }
        "PUT" => {
            let body_arg = if body_expr.is_empty() {
                String::new()
            } else {
                format!(", body: {body_expr}")
            };
            let _ = writeln!(
                out,
                "    final response = await http.put(uri, headers: headers{body_arg});"
            );
        }
        "PATCH" => {
            let body_arg = if body_expr.is_empty() {
                String::new()
            } else {
                format!(", body: {body_expr}")
            };
            let _ = writeln!(
                out,
                "    final response = await http.patch(uri, headers: headers{body_arg});"
            );
        }
        "DELETE" => {
            let body_arg = if body_expr.is_empty() {
                String::new()
            } else {
                format!(", body: {body_expr}")
            };
            let _ = writeln!(
                out,
                "    final response = await http.delete(uri, headers: headers{body_arg});"
            );
        }
        "HEAD" => {
            // package:http doesn't have a head() helper — use the generic send.
            let _ = writeln!(out, "    final req = http.Request('HEAD', uri);");
            let _ = writeln!(out, "    req.headers.addAll(headers);");
            let _ = writeln!(
                out,
                "    final streamed = await http.Client().send(req);"
            );
            let _ = writeln!(
                out,
                "    final response = await http.Response.fromStream(streamed);"
            );
        }
        other => {
            // Generic fallback for uncommon methods (OPTIONS, CONNECT, TRACE, etc.).
            let escaped_method = escape_dart(other);
            let _ = writeln!(out, "    final req = http.Request('{escaped_method}', uri);");
            let _ = writeln!(out, "    req.headers.addAll(headers);");
            if !body_expr.is_empty() {
                let _ = writeln!(out, "    req.body = {body_expr};");
            }
            let _ = writeln!(
                out,
                "    final streamed = await http.Client().send(req);"
            );
            let _ = writeln!(
                out,
                "    final response = await http.Response.fromStream(streamed);"
            );
        }
    }

    // Assert status code.
    let _ = writeln!(
        out,
        "    expect(response.statusCode, equals({expected_status}), reason: 'status code mismatch');"
    );

    // Assert body if expected.
    if let Some(expected_body) = &expected.body {
        match expected_body {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected_body).unwrap_or_default();
                let escaped = escape_dart(&json_str);
                let _ = writeln!(out, "    final bodyJson = jsonDecode(response.body);");
                let _ = writeln!(out, "    final expectedJson = jsonDecode('{escaped}');");
                let _ = writeln!(
                    out,
                    "    expect(bodyJson, equals(expectedJson), reason: 'body mismatch');"
                );
            }
            serde_json::Value::String(s) => {
                let escaped = escape_dart(s);
                let _ = writeln!(
                    out,
                    "    expect(response.body.trim(), equals('{escaped}'), reason: 'body mismatch');"
                );
            }
            other => {
                let escaped = escape_dart(&other.to_string());
                let _ = writeln!(
                    out,
                    "    expect(response.body.trim(), equals('{escaped}'), reason: 'body mismatch');"
                );
            }
        }
    }

    // Assert response headers if specified.
    for (name, value) in &expected.headers {
        if value == "<<absent>>" || value == "<<present>>" || value == "<<uuid>>" {
            continue;
        }
        // content-encoding is set by the real server's compression middleware
        // but the mock server doesn't compress bodies, so skip this assertion.
        if name.to_lowercase() == "content-encoding" {
            continue;
        }
        let escaped_name = escape_dart(&name.to_lowercase());
        let escaped_value = escape_dart(value);
        let _ = writeln!(
            out,
            "    expect(response.headers['{escaped_name}'], contains('{escaped_value}'), reason: 'header {escaped_name} mismatch');"
        );
    }

    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
}

/// Emit a compilable skip stub for non-HTTP fixtures without a dart call override.
fn render_skip_stub(out: &mut String, fixture: &Fixture) {
    let description = escape_dart(&fixture.description);
    let fixture_id = &fixture.id;
    let _ = writeln!(out, "  test('{description}', () {{");
    let _ = writeln!(
        out,
        "    markTestSkipped('TODO: implement Dart e2e test for fixture \\'{fixture_id}\\'');"
    );
    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
}

/// Escape a string for embedding in a Dart single-quoted string literal.
fn escape_dart(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('$', "\\$")
}
