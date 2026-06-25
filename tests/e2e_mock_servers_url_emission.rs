//! Tests: when a fixture has a host-root route (robots/sitemap), each language's
//! generated test code references the per-fixture env var
//! (MOCK_SERVER_<FIXTURE_ID_UPPER>) rather than MOCK_SERVER_URL/fixtures/<id>.
//! Non-host-root fixtures must continue to use the namespaced URL pattern.
//!
//! Also verifies that each language's conftest/setup emits the MOCK_SERVERS= parsing logic.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::codegen::typescript::TypeScriptCodegen;
use alef::e2e::codegen::wasm::WasmCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

// ── fixture/config helpers ────────────────────────────────────────────────────

fn make_host_root_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("{id} fixture with host-root route"),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
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
        args: Vec::new(),
        assertion_recipes: Vec::new(),
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
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({"url": "http://example.com/page"}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
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

fn build_config(language: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "demo_crawler"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
java_group_id = "dev.sample_crate"

[crates.e2e.call]
function = "scrape"
module = "DemoCrawler"
result_var = "result"
async = true
returns_result = true
args = [
  {{ name = "url", field = "url", type = "mock_url" }},
]

[crates.e2e.call.overrides.ruby]
module = "DemoCrawler"

[crates.e2e.call.overrides.php]
module = "DemoCrawler"

[crates.e2e.call.overrides.csharp]
class = "DemoCrawler"

[crates.e2e.call.overrides.elixir]
module = "DemoCrawler"
returns_result = true

[crates.e2e.call.overrides.go]
import_alias = "demo_crawler"
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
) -> Vec<alef::core::backend::GeneratedFile> {
    let (e2e, resolved) = build_config(language);
    let groups = groups_with(fixtures);
    codegen
        .generate(&groups, &e2e, &resolved, &[], &[])
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

// NOTE: `go_main_test_emits_mock_servers_parsing` was removed in the
// harness-pattern refactor (commit b6112c283 "myriad e2e test fixes"). Go no
// longer parses `MOCK_SERVERS=` inside `main_test.go`; per-fixture URLs are
// resolved at test time via the `MOCK_SERVER_<FIXTURE_ID>` env vars set by
// whatever process spawns the mock-server (parent test runner / harness
// binary), with a `MOCK_SERVER_URL/fixtures/<id>` fallback. See
// `src/e2e/codegen/go.rs` (`fixture.has_host_root_route()` branch).

// ── Java ──────────────────────────────────────────────────────────────────────

// NOTE: `java_mock_server_listener_emits_mock_servers_parsing` was removed in
// the same b6112c283 refactor. `render_mock_server_listener` was kept as
// `#[allow(dead_code)]` Rust scaffolding for a future re-wiring but is not
// currently emitted: crawler-style fixtures rely on
// `MOCK_SERVER_<FIXTURE_ID>` env vars (set by parent harness) with a
// `MOCK_SERVER_URL/fixtures/<id>` fallback; server-pattern HTTP fixtures use
// `HarnessMain.java` instead. See `src/e2e/codegen/java.rs` lines 1714-1729.

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
fn ruby_spec_helper_skips_spawn_when_mock_server_url_preset() {
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
        spec_helper.content.contains("existing_url = ENV['MOCK_SERVER_URL']")
            && spec_helper.content.contains("if existing_url && !existing_url.empty?"),
        "spec_helper.rb must honor a pre-set MOCK_SERVER_URL and skip self-spawn:\n{}",
        spec_helper.content
    );
    // Guard must appear before the popen3 spawn call.
    let guard = spec_helper
        .content
        .find("if existing_url && !existing_url.empty?")
        .expect("guard present");
    let spawn = spec_helper.content.find("popen3").expect("popen3 present");
    assert!(
        guard < spawn,
        "the pre-set MOCK_SERVER_URL guard must precede popen3:\n{}",
        spec_helper.content
    );
}

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
fn elixir_test_helper_skips_spawn_when_mock_server_url_preset() {
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
        test_helper
            .content
            .contains("unless System.get_env(\"MOCK_SERVER_URL\")"),
        "test_helper.exs must honor a pre-set MOCK_SERVER_URL and skip self-spawn:\n{}",
        test_helper.content
    );
    // Guard must appear before the Port.open spawn call.
    let guard = test_helper
        .content
        .find("unless System.get_env(\"MOCK_SERVER_URL\")")
        .expect("guard present");
    let spawn = test_helper.content.find("Port.open").expect("Port.open present");
    assert!(
        guard < spawn,
        "the pre-set MOCK_SERVER_URL guard must precede Port.open:\n{}",
        test_helper.content
    );
}

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

// ── Install isolation + pre-set MOCK_SERVER_URL (node / wasm) ──────────────────

#[test]
fn typescript_emits_isolated_pnpm_workspace_in_registry_mode() {
    let (mut e2e, resolved) = build_config("node");
    e2e.dep_mode = alef::e2e::config::DependencyMode::Registry;
    let groups = groups_with(vec![make_plain_fixture("basic_crawl")]);
    let files = TypeScriptCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    let workspace = files
        .iter()
        .find(|f| f.path.ends_with("pnpm-workspace.yaml"))
        .expect("node codegen in Registry mode must emit pnpm-workspace.yaml so pnpm install does not sweep the test app into an outer workspace and skip its devDependencies");
    assert!(
        workspace.content.contains("packages:"),
        "pnpm-workspace.yaml must declare an isolated workspace root:\n{}",
        workspace.content
    );
}

#[test]
fn typescript_omits_pnpm_workspace_in_local_mode() {
    // In Local (workspace:*) mode the test app depends on the binding via
    // workspace protocol, which can only resolve through the consumer's root
    // pnpm-workspace.yaml. Emitting `packages: []` would shadow the consumer's
    // workspace and break `pnpm install` with no matching version.
    let (e2e, resolved) = build_config("node");
    assert_eq!(e2e.dep_mode, alef::e2e::config::DependencyMode::Local);
    let groups = groups_with(vec![make_plain_fixture("basic_crawl")]);
    let files = TypeScriptCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    assert!(
        files.iter().all(|f| !f.path.ends_with("pnpm-workspace.yaml")),
        "node codegen in Local mode must not emit pnpm-workspace.yaml — it would shadow the consumer's workspace and break workspace:* resolution"
    );
}

#[test]
fn typescript_global_setup_skips_spawn_when_mock_server_url_preset() {
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
        global_setup
            .content
            .contains("const presetUrl = process.env.MOCK_SERVER_URL ?? process.env.SUT_URL;")
            && global_setup.content.contains("if (presetUrl)"),
        "globalSetup.ts must honor a pre-set MOCK_SERVER_URL and skip self-spawn:\n{}",
        global_setup.content
    );
    let guard = global_setup.content.find("if (presetUrl)").expect("guard present");
    let spawn = global_setup.content.find("spawn(").expect("spawn present");
    assert!(
        guard < spawn,
        "the pre-set MOCK_SERVER_URL guard must precede the spawn() call:\n{}",
        global_setup.content
    );
}

#[test]
fn wasm_setup_ts_initializes_wasm_per_worker() {
    // The wasm init MUST appear in setup.ts (vitest setupFiles, per-worker)
    // because globalSetup runs only in the main process; worker processes spawn
    // their own module graph and would hit __wbindgen_add_to_stack_pointer crashes
    // without a per-worker init call. Uses initSync + readFileSync to bypass
    // Node.js fetch() not supporting file:// URLs.
    let files = generate_all(
        &WasmCodegen,
        "wasm",
        vec![make_host_root_fixture("robots_disallow_path")],
    );
    let setup_ts = files
        .iter()
        .find(|f| f.path.ends_with("setup.ts"))
        .expect("wasm setup.ts not found — it must be emitted for HTTP fixtures");
    assert!(
        setup_ts.content.contains("initSync"),
        "wasm setup.ts must call initSync to initialize the wasm module per worker:\n{}",
        setup_ts.content
    );
    assert!(
        setup_ts.content.contains("readFileSync"),
        "wasm setup.ts must use readFileSync to load the wasm binary (fetch() doesn't support file://):\n{}",
        setup_ts.content
    );
}
