//! Tests: every language whose generated e2e tests read the mock-server URL
//! from `MOCK_SERVER_URL` must also generate a harness that spawns the
//! mock-server binary and exports that URL. Without the harness the tests fall
//! back to a hardcoded `http://localhost:8080` and fail with connection
//! refused, because the mock-server binds an ephemeral `127.0.0.1` port.
//!
//! Languages with a self-contained spawn pattern that is exercised here:
//!   * Dart — `setUpAll` spawns the binary and exports `MOCK_SERVER_URL`.
//!   * Zig  — `build.zig` spawns the binary at configure time and sets the
//!     run-step environment.
//!
//! The remaining languages are covered by `e2e_mock_servers_url_emission.rs`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{
    Assertion, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest,
};

/// Build an HTTP server fixture (the shape that hits the mock server directly
/// via `MOCK_SERVER_URL/fixtures/<id>`).
fn make_http_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("{id} HTTP fixture"),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "status_code".to_string(),
            field: None,
            value: Some(serde_json::json!(200)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: Some(HttpFixture {
            handler: HttpHandler {
                route: "/ping".to_string(),
                method: "GET".to_string(),
                body_schema: None,
                parameters: Default::default(),
                middleware: None,
            },
            request: HttpRequest {
                method: "GET".to_string(),
                path: "/ping".to_string(),
                headers: Default::default(),
                query_params: Default::default(),
                cookies: Default::default(),
                body: None,
                form_data: None,
                content_type: None,
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: Some(serde_json::json!({"ok": true})),
                body_partial: None,
                headers: Default::default(),
                validation_errors: None,
            },
        }),
    }
}

fn groups_with(fixtures: Vec<Fixture>) -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures,
    }]
}

fn config_for(language: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "ping"
result_var = "result"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn generate(codegen: &dyn E2eCodegen, language: &str) -> Vec<alef::core::backend::GeneratedFile> {
    let (e2e, resolved) = config_for(language);
    let groups = groups_with(vec![make_http_fixture("ping_ok")]);
    codegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

#[test]
fn dart_http_fixture_test_file_spawns_mock_server() {
    let files = generate(&DartE2eCodegen, "dart");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_test.dart"))
        .expect("dart test file is emitted");
    let content = &test_file.content;
    assert!(
        content.contains("Process.start") || content.contains("Process.run"),
        "dart harness must spawn a server process. Rendered:\n{content}"
    );
    assert!(
        content.contains("app_harness.dart"),
        "dart harness must reference app_harness.dart. Rendered:\n{content}"
    );
    assert!(
        content.contains("SUT_URL="),
        "dart harness must parse the SUT_URL= startup line. Rendered:\n{content}"
    );
}

#[test]
fn zig_http_fixture_build_spawns_mock_server() {
    let files = generate(&ZigE2eCodegen, "zig");
    let build_zig = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.zig"))
        .expect("build.zig is emitted");
    let content = &build_zig.content;
    assert!(
        content.contains("mock-server"),
        "zig build must reference the mock-server binary. Rendered:\n{content}"
    );
    assert!(
        content.contains("MOCK_SERVER_URL"),
        "zig build must export MOCK_SERVER_URL to the test run steps. Rendered:\n{content}"
    );
    assert!(
        content.contains("std.process.spawn") || content.contains("std.process.Child"),
        "zig build must spawn a child process for the mock-server. Rendered:\n{content}"
    );
}

#[test]
fn elixir_http_fixture_forces_http1_on_req() {
    let files = generate(&ElixirCodegen, "elixir");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_test.exs"))
        .expect("elixir HTTP test file is emitted");
    let content = &test_file.content;
    assert!(
        content.contains("Req."),
        "elixir test file must emit a Req call. Rendered:\n{content}"
    );
    assert!(
        content.contains("finch: AlefE2EFinch"),
        "every Req call must use the HTTP/1 Finch pool. Rendered:\n{content}"
    );
    for line in content.lines().filter(|l| l.contains("Req.")) {
        assert!(
            line.contains("finch: AlefE2EFinch"),
            "Req call missing named Finch pool: {line}\nRendered:\n{content}"
        );
    }
}
