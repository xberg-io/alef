//! Verifies that the Dart e2e codegen correctly:
//! 1. emits _setEnv helper when e2e.env is non-empty
//! 2. injects env vars in setUpAll before RustLib.init()
//! 3. sorts env keys alphabetically
//! 4. escapes special characters in values
//! 5. omits helper and imports when e2e.env is empty
//! 6. adds dart:ffi and package:ffi imports when needed
//! 7. includes ffi dependency in pubspec when needed

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

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
        mock_response: Some(alef::e2e::fixture::MockResponse {
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

fn render_dart_smoke(toml: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group("test_fixture")];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.dart"))
        .expect("smoke_test.dart is emitted")
        .content
        .clone()
}

fn render_pubspec(toml: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group("test_fixture")];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("pubspec.yaml"))
        .expect("pubspec.yaml is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "demo_client"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
result_var = "result"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

#[test]
fn non_empty_env_emits_setenv_helper() {
    let toml = format!(
        r#"{}

[crates.e2e.env]
ALLOW_PRIVATE_NETWORK = "true"
"#,
        BASE_TOML
    );

    let generated = render_dart_smoke(&toml);

    assert!(
        generated.contains("void _setEnv(String key, String value)"),
        "Helper function missing"
    );
    assert!(
        generated.contains("final libc = DynamicLibrary.process();"),
        "DynamicLibrary.process() missing"
    );
    assert!(generated.contains("calloc.free(keyPtr)"), "calloc.free() missing");

    assert!(
        generated.contains("_setEnv('ALLOW_PRIVATE_NETWORK', 'true');"),
        "setUpAll env injection missing"
    );

    assert!(generated.contains("import 'dart:ffi';"), "dart:ffi import missing");
    assert!(
        generated.contains("import 'package:ffi/ffi.dart';"),
        "package:ffi import missing"
    );
}

#[test]
fn env_keys_sorted_alphabetically() {
    let toml = format!(
        r#"{}

[crates.e2e.env]
ZEBRA = "z"
APPLE = "a"
MIDDLE = "m"
"#,
        BASE_TOML
    );

    let generated = render_dart_smoke(&toml);

    let setup_section = generated
        .split("setUpAll(() async {")
        .nth(1)
        .expect("setUpAll block found")
        .split("await RustLib.init()")
        .next()
        .expect("RustLib.init found");

    let apple_pos = setup_section.find("APPLE").expect("APPLE env var found");
    let middle_pos = setup_section.find("MIDDLE").expect("MIDDLE env var found");
    let zebra_pos = setup_section.find("ZEBRA").expect("ZEBRA env var found");

    assert!(apple_pos < middle_pos, "APPLE should come before MIDDLE");
    assert!(middle_pos < zebra_pos, "MIDDLE should come before ZEBRA");
}

#[test]
fn empty_env_omits_setenv_helper_and_imports() {
    let generated = render_dart_smoke(BASE_TOML);

    assert!(
        !generated.contains("void _setEnv(String key, String value)"),
        "Helper function should be omitted for empty env"
    );

    let lines: Vec<&str> = generated.lines().collect();
    let ffi_import_count = lines
        .iter()
        .take_while(|l| !l.contains("void main()"))
        .filter(|l| l.contains("import 'dart:ffi'") || l.contains("import 'package:ffi/ffi.dart'"))
        .count();
    assert_eq!(ffi_import_count, 0, "FFI imports should not be present for empty env");
}

#[test]
fn env_values_escape_quotes_and_backslashes() {
    let toml = format!(
        r#"{}

[crates.e2e.env]
PATH_WITH_BACKSLASH = "C:\\Users\\test"
STRING_WITH_QUOTE = "value with \"quotes\""
"#,
        BASE_TOML
    );

    let generated = render_dart_smoke(&toml);

    assert!(
        generated.contains("_setEnv('PATH_WITH_BACKSLASH', 'C:\\\\Users\\\\test');"),
        "Backslashes not escaped in setUpAll"
    );
    assert!(
        generated.contains("_setEnv('STRING_WITH_QUOTE', 'value with \\\"quotes\\\"');"),
        "Quotes not escaped in setUpAll"
    );
}

#[test]
fn pubspec_includes_ffi_dependency_when_env_present() {
    let toml = format!(
        r#"{}

[crates.e2e.env]
SOME_VAR = "value"
"#,
        BASE_TOML
    );

    let pubspec = render_pubspec(&toml);

    assert!(pubspec.contains("ffi:"), "ffi dependency missing in pubspec");
    assert!(pubspec.contains("^2.2.0"), "ffi version constraint missing");
}

#[test]
fn pubspec_includes_ffi_even_without_env_vars() {
    let pubspec = render_pubspec(BASE_TOML);

    assert!(pubspec.contains("ffi:"), "ffi dependency should be included");
}

#[test]
fn env_injection_before_rustlib_init() {
    let toml = format!(
        r#"{}

[crates.e2e.env]
ALLOW_PRIVATE_NETWORK = "true"
"#,
        BASE_TOML
    );

    let generated = render_dart_smoke(&toml);

    let setup_start = generated.find("setUpAll(() async {").expect("setUpAll found");
    let setup_section = &generated[setup_start..];

    let env_pos = setup_section.find("_setEnv(").expect("_setEnv call found");
    let init_pos = setup_section.find("await RustLib.init()").expect("RustLib.init found");

    assert!(env_pos < init_pos, "_setEnv should be called before RustLib.init()");
}
