//! Tests: when a fixture has a host-root route (robots/sitemap), each language's
//! generated test code references the per-fixture env var
//! (MOCK_SERVER_<FIXTURE_ID_UPPER>) rather than MOCK_SERVER_URL/fixtures/<id>.
//! Non-host-root fixtures must continue to use the namespaced URL pattern.
//!
//! Also verifies that each language's conftest/setup emits the MOCK_SERVERS= parsing logic.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::csharp::CSharpCodegen;
use alef_e2e::codegen::elixir::ElixirCodegen;
use alef_e2e::codegen::go::GoCodegen;
use alef_e2e::codegen::java::JavaCodegen;
use alef_e2e::codegen::php::PhpCodegen;
use alef_e2e::codegen::python::PythonE2eCodegen;
use alef_e2e::codegen::ruby::RubyCodegen;
use alef_e2e::codegen::typescript::TypeScriptCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

// ── fixture/config helpers ────────────────────────────────────────────────────

fn make_host_root_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("{id} fixture with host-root route"),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({
            "url": "http://example.com/",
            "mock_responses": [
                {"path": "/robots.txt", "status_code": 200, "body_inline": "User-agent: *\nDisallow: /"},
                {"path": "/", "status_code": 200, "body_inline": "<html/>"}
            ]
        }),
        mock_response: None,
        visitor: None,
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn make_plain_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("{id} plain fixture"),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({"url": "http://example.com/page"}),
        mock_response: None,
        visitor: None,
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn build_config(language: &str) -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "kreuzcrawl"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
java_group_id = "dev.kreuzberg"

[crates.e2e.call]
function = "scrape"
module = "Kreuzcrawl"
result_var = "result"
async = true
returns_result = true
args = [
  {{ name = "url", field = "url", type = "mock_url" }},
]

[crates.e2e.call.overrides.ruby]
module = "Kreuzcrawl"

[crates.e2e.call.overrides.php]
module = "Kreuzcrawl"

[crates.e2e.call.overrides.csharp]
class = "Kreuzcrawl"

[crates.e2e.call.overrides.elixir]
module = "Kreuzcrawl"
returns_result = true

[crates.e2e.call.overrides.go]
import_alias = "kreuzcrawl"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn groups_with(fixtures: Vec<Fixture>) -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures,
    }]
}

fn generate_all(
    codegen: &dyn E2eCodegen,
    language: &str,
    fixtures: Vec<Fixture>,
) -> Vec<alef_core::backend::GeneratedFile> {
    let (e2e, resolved) = build_config(language);
    let groups = groups_with(fixtures);
    codegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds")
}

// ── Python ────────────────────────────────────────────────────────────────────

#[test]
fn python_host_root_fixture_url_uses_mock_server_env_key() {
    let files = generate_all(
        &PythonE2eCodegen,
        "python",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let test_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap_or("").ends_with(".py") && f.path.to_str().unwrap_or("").contains("test_"))
        .expect("test file not found");
    assert!(
        test_file.content.contains("MOCK_SERVER_ROBOTS_DISALLOW_PATH"),
        "expected MOCK_SERVER_ROBOTS_DISALLOW_PATH in:\n{}",
        test_file.content
    );
}

#[test]
fn python_plain_fixture_url_uses_namespaced_pattern() {
    let files = generate_all(&PythonE2eCodegen, "python", vec![make_plain_fixture("basic_crawl")]);
    let test_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap_or("").ends_with(".py") && f.path.to_str().unwrap_or("").contains("test_"))
        .expect("test file not found");
    assert!(
        test_file.content.contains("/fixtures/basic_crawl"),
        "expected /fixtures/basic_crawl in:\n{}",
        test_file.content
    );
}

#[test]
fn python_conftest_emits_mock_servers_parsing() {
    let files = generate_all(
        &PythonE2eCodegen,
        "python",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let conftest = files
        .iter()
        .find(|f| f.path.ends_with("conftest.py"))
        .expect("conftest.py not found");
    assert!(
        conftest.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in conftest.py:\n{}",
        conftest.content
    );
}

// ── TypeScript (uses "node" language name) ────────────────────────────────────

#[test]
fn typescript_host_root_fixture_url_uses_mock_server_env_key() {
    let files = generate_all(
        &TypeScriptCodegen,
        "node",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let test_file = files
        .iter()
        .find(|f| {
            let p = f.path.to_str().unwrap_or("");
            (p.ends_with(".ts") && p.contains("test_")) || p.ends_with(".test.ts")
        })
        .expect("test file not found");
    assert!(
        test_file.content.contains("MOCK_SERVER_ROBOTS_DISALLOW_PATH"),
        "expected MOCK_SERVER_ROBOTS_DISALLOW_PATH in:\n{}",
        test_file.content
    );
}

#[test]
fn typescript_global_setup_emits_mock_servers_parsing() {
    let files = generate_all(
        &TypeScriptCodegen,
        "node",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let global_setup = files
        .iter()
        .find(|f| f.path.ends_with("globalSetup.ts"))
        .expect("globalSetup.ts not found");
    assert!(
        global_setup.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in globalSetup.ts:\n{}",
        global_setup.content
    );
}

// ── Go ────────────────────────────────────────────────────────────────────────

#[test]
fn go_host_root_fixture_url_uses_mock_server_env_key() {
    let files = generate_all(&GoCodegen, "go", vec![make_host_root_fixture("robots_disallow_path")]);
    let test_file = files
        .iter()
        .find(|f| {
            let p = f.path.to_str().unwrap_or("");
            p.ends_with("_test.go") && !p.ends_with("main_test.go") && !p.ends_with("helpers_test.go")
        })
        .expect("test file not found");
    assert!(
        test_file.content.contains("MOCK_SERVER_ROBOTS_DISALLOW_PATH"),
        "expected MOCK_SERVER_ROBOTS_DISALLOW_PATH in:\n{}",
        test_file.content
    );
}

#[test]
fn go_main_test_emits_mock_servers_parsing() {
    let files = generate_all(&GoCodegen, "go", vec![make_host_root_fixture("robots_disallow_path")]);
    let main_test = files
        .iter()
        .find(|f| f.path.ends_with("main_test.go"))
        .expect("main_test.go not found");
    assert!(
        main_test.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in main_test.go:\n{}",
        main_test.content
    );
}

// ── Java ──────────────────────────────────────────────────────────────────────

#[test]
fn java_mock_server_listener_emits_mock_servers_parsing() {
    let files = generate_all(
        &JavaCodegen,
        "java",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let listener = files
        .iter()
        .find(|f| f.path.to_str().unwrap_or("").ends_with("MockServerListener.java"))
        .expect("MockServerListener.java not found");
    assert!(
        listener.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in MockServerListener.java:\n{}",
        listener.content
    );
}

// ── C# ───────────────────────────────────────────────────────────────────────

#[test]
fn csharp_host_root_fixture_url_uses_mock_server_env_key() {
    let files = generate_all(
        &CSharpCodegen,
        "csharp",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    // Test class file is e.g. SmokeTests.cs — not TestSetup.cs
    let test_file = files
        .iter()
        .find(|f| {
            let p = f.path.to_str().unwrap_or("");
            p.ends_with("Tests.cs") && !p.ends_with("TestSetup.cs")
        })
        .expect("test class file not found");
    assert!(
        test_file.content.contains("MOCK_SERVER_ROBOTS_DISALLOW_PATH"),
        "expected MOCK_SERVER_ROBOTS_DISALLOW_PATH in:\n{}",
        test_file.content
    );
}

#[test]
fn csharp_test_setup_emits_mock_servers_parsing() {
    let files = generate_all(
        &CSharpCodegen,
        "csharp",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let setup = files
        .iter()
        .find(|f| f.path.ends_with("TestSetup.cs"))
        .expect("TestSetup.cs not found");
    assert!(
        setup.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in TestSetup.cs:\n{}",
        setup.content
    );
}

// ── PHP ───────────────────────────────────────────────────────────────────────

#[test]
fn php_bootstrap_emits_mock_servers_parsing() {
    let files = generate_all(&PhpCodegen, "php", vec![make_host_root_fixture("robots_disallow_path")]);
    let bootstrap = files
        .iter()
        .find(|f| f.path.ends_with("bootstrap.php"))
        .expect("bootstrap.php not found");
    assert!(
        bootstrap.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in bootstrap.php:\n{}",
        bootstrap.content
    );
}

// ── Ruby ──────────────────────────────────────────────────────────────────────

#[test]
fn ruby_spec_helper_emits_mock_servers_parsing() {
    let files = generate_all(
        &RubyCodegen,
        "ruby",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let spec_helper = files
        .iter()
        .find(|f| f.path.ends_with("spec_helper.rb"))
        .expect("spec_helper.rb not found");
    assert!(
        spec_helper.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in spec_helper.rb:\n{}",
        spec_helper.content
    );
}

// ── Elixir ────────────────────────────────────────────────────────────────────

#[test]
fn elixir_test_helper_emits_mock_servers_parsing() {
    let files = generate_all(
        &ElixirCodegen,
        "elixir",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let test_helper = files
        .iter()
        .find(|f| f.path.ends_with("test_helper.exs"))
        .expect("test_helper.exs not found");
    assert!(
        test_helper.content.contains("MOCK_SERVERS="),
        "expected MOCK_SERVERS= parsing in test_helper.exs:\n{}",
        test_helper.content
    );
}
