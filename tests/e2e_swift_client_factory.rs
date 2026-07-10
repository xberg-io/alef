//! Verifies the Swift e2e codegen emits client-object instantiation when
//! `CallOverride.client_factory` is set, and falls back to free-function calls
//! when it is absent.
//!
//! Also verifies that `render_package_swift` always emits `.iOS(...)` alongside
//! `.macOS(...)`, regardless of `client_factory` presence.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "request": { "model": "gpt-4o", "messages": [] } }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
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

fn make_group(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture(id)],
    }
}

fn render_swift(toml: &str, fixture_id: &str) -> Vec<alef::core::backend::GeneratedFile> {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

fn smoke_test_content(files: &[alef::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted")
        .content
        .clone()
}

fn package_swift_content(files: &[alef::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.ends_with("Package.swift"))
        .expect("Package.swift is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "demo_client"
result_var = "result"
async = true

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// When `client_factory` is set, the generated test must:
///   1. Instantiate `DefaultClient` with `apiKey:` + `baseUrl:` pointing at the mock
///   2. Call the method on `_client` instead of as a free function
///   3. NOT call the free function `chat(...)` directly
#[test]
fn with_client_factory_emits_client_instantiation() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
client_factory = "DefaultClient"
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_basic");
    let rendered = smoke_test_content(&files);

    assert!(
        rendered.contains("DefaultClient(apiKey:"),
        "must instantiate DefaultClient with apiKey. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("AlefE2EMockServer.baseURL"),
        "must reference AlefE2EMockServer.baseURL for the mock base url. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("_client.chat("),
        "must call chat on client instance. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("try await chat("),
        "must NOT call free-function chat when client_factory is set. Rendered:\n{rendered}"
    );
}

/// When `client_factory` is absent, the generator must emit a free-function call.
/// This ensures no regression for flat-function Swift bindings.
#[test]
fn without_client_factory_emits_free_function_call() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_basic");
    let rendered = smoke_test_content(&files);

    assert!(
        rendered.contains("try await DemoClient.chat("),
        "must call module-qualified free function chat directly. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("DefaultClient("),
        "must NOT instantiate DefaultClient when client_factory is absent. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("_client."),
        "must NOT reference client instance when client_factory is absent. Rendered:\n{rendered}"
    );
}

/// Package.swift must always include both `.macOS(...)` and `.iOS(...)` platforms,
/// regardless of whether `client_factory` is configured. The iOS minimum tracks
/// `toolchain::SWIFT_MIN_IOS` so the e2e consumer's deployment target is >= the
/// dep's deployment target (SwiftPM hides products otherwise).
#[test]
fn package_swift_always_includes_ios_platform() {
    let toml_no_cf = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
options_via = "from_json"
"#
    );
    let files_no_cf = render_swift(&toml_no_cf, "smoke_basic");
    let pkg_no_cf = package_swift_content(&files_no_cf);
    assert!(
        pkg_no_cf.contains(".macOS("),
        "Package.swift must include macOS platform. Content:\n{pkg_no_cf}"
    );
    assert!(
        pkg_no_cf.contains(".iOS(.v"),
        "Package.swift must include .iOS platform. Content:\n{pkg_no_cf}"
    );

    let toml_cf = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
client_factory = "DefaultClient"
options_via = "from_json"
"#
    );
    let files_cf = render_swift(&toml_cf, "smoke_basic");
    let pkg_cf = package_swift_content(&files_cf);
    assert!(
        pkg_cf.contains(".iOS(.v"),
        "Package.swift must include .iOS platform also when client_factory is set. Content:\n{pkg_cf}"
    );
}

/// SwiftPM 6.0 derives path-dep identity from the path basename, ignoring any
/// `name:` override. To avoid identity collision between the e2e consumer and
/// the dep (both at directories named `swift/`), the e2e package is emitted
/// under `swift_e2e/`, and the dep is referenced by `.package(path:)` (no
/// `name:`) with `.product(package: "<basename>")`. Regression test for the
/// fixture case where the e2e package path previously collided with the dep path.
#[test]
fn package_swift_uses_path_basename_for_product_package_ref() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_basic");
    let pkg_file = files
        .iter()
        .find(|f| f.path.ends_with("Package.swift"))
        .expect("Package.swift is emitted");
    assert!(
        pkg_file
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .contains("/swift_e2e/Package.swift"),
        "Package.swift must be emitted under swift_e2e/. Path: {:?}",
        pkg_file.path
    );
    let pkg = &pkg_file.content;
    assert!(
        !pkg.contains(".package(name:"),
        "Package.swift must not use deprecated .package(name:path:) form. Content:\n{pkg}"
    );
    assert!(
        pkg.contains(r#".product(name: "DemoClient", package: "swift")"#),
        "Package.swift must reference the dep by path basename `swift`. Content:\n{pkg}"
    );
    assert!(
        pkg.contains(r#".package(path: "../../packages/swift")"#),
        "Package.swift must declare the dep via .package(path:) without name:. Content:\n{pkg}"
    );
}
