//! Verifies the Dart e2e codegen emits non-empty tearDownAll block.
//! Regression test for a bug where fixtures with no HTTP and no SUT spawn
//! would emit empty tearDownAll(() async {}), causing dart-format to reflow
//! and create infinite formatting loops.
//!
//! Fix: always emit RustLib.dispose() call to ensure non-empty body.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

fn make_fixture(id: &str, description: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: description.to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![],
        source: "smoke.json".to_string(),
        http: None, // Explicitly no HTTP fixtures
    }
}

fn make_group(fixtures: Vec<Fixture>) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures,
    }
}

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-app"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_app"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "smoke_test"
result_var = "result"
result_is_simple = true

[[crates.e2e.call.args]]
name = "input"
field = "input.data"
type = "string"
"#;

fn render(fixtures: Vec<Fixture>) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixtures)];
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

/// When a fixture has no HTTP fixtures and no SUT spawn, tearDownAll must still
/// have a non-empty body. The fix is to always call RustLib.dispose() to ensure
/// dart-format doesn't reflow the empty closure and cause formatting loops.
#[test]
fn teardown_all_always_calls_rust_lib_dispose() {
    let fixtures = vec![make_fixture(
        "minimal_smoke_test",
        "minimal smoke test with no HTTP and no SUT",
    )];

    let rendered = render(fixtures);

    // Must emit tearDownAll block.
    assert!(
        rendered.contains("tearDownAll(() async {"),
        "must emit tearDownAll block. Rendered:\n{rendered}"
    );

    // Must call RustLib.dispose() inside tearDownAll.
    assert!(
        rendered.contains("RustLib.dispose();"),
        "must call RustLib.dispose() in tearDownAll to ensure non-empty body. Rendered:\n{rendered}"
    );

    // Verify the pattern: tearDownAll body should contain dispose before closing.
    let teardown_start = rendered
        .find("tearDownAll(() async {")
        .expect("tearDownAll block found");
    let teardown_end = rendered[teardown_start..]
        .find("});")
        .expect("tearDownAll closing found");
    let teardown_body = &rendered[teardown_start..teardown_start + teardown_end];

    assert!(
        teardown_body.contains("RustLib.dispose();"),
        "RustLib.dispose() must be in tearDownAll body. Body:\n{teardown_body}"
    );
}

/// Verify the exact emit format to prevent dart-format reflow.
#[test]
fn teardown_all_has_correct_format() {
    let fixtures = vec![make_fixture("format_check_smoke", "check dart-format compatibility")];

    let rendered = render(fixtures);

    // The expected pattern (single-line, with body content).
    // We need to find the tearDownAll and verify it's formatted correctly.
    let lines: Vec<&str> = rendered.lines().collect();
    let teardown_idx = lines
        .iter()
        .position(|line| line.contains("tearDownAll(() async {"))
        .expect("tearDownAll found");

    // The body should contain the guarded RustLib.dispose() call.
    assert!(
        teardown_idx + 2 < lines.len(),
        "tearDownAll must have a body (at least RustLib.dispose)"
    );
    assert!(
        lines[teardown_idx + 1].contains("if (_rustLibInitialized) {"),
        "first line after tearDownAll opening must guard RustLib.dispose(). Found: {}",
        lines[teardown_idx + 1]
    );
    assert!(
        lines[teardown_idx + 2].contains("RustLib.dispose();"),
        "guarded tearDownAll body must contain RustLib.dispose(). Found: {}",
        lines[teardown_idx + 2]
    );
}
