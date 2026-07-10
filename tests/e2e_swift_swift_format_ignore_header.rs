//! Verifies the Swift e2e codegen prepends `// swift-format-ignore-file` to
//! every emitted `.swift` source. Apple's `swift-format` (typically wired into
//! the consumer repo's pre-commit set) does not honour the 4-space indent,
//! XCTest-first import order, or unwrapped long lines that the e2e generator
//! produces — without the marker, every `alef e2e generate` run produces files
//! that the next `swift-format` hook rewrites, defeating
//! `alef verify --exit-code` and breaking CI.
//!
//! Mirrors the same pattern in `alef-backend-swift`, where
//! `DemoMarkup.swift` already carries the directive.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "ignore-directive coverage fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
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

const BASE_TOML: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample-language-pack"
sources = ["src/lib.rs"]

[crates.e2e]
[crates.e2e.call]
function = "extract"
result_var = "result"
args = []
"#;

fn render_files() -> Vec<alef::core::backend::GeneratedFile> {
    let cfg: NewAlefConfig = toml::from_str(BASE_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group("smoke_case")];
    SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

fn swift_sources(files: &[alef::core::backend::GeneratedFile]) -> Vec<&alef::core::backend::GeneratedFile> {
    files
        .iter()
        .filter(|f| {
            let path = f.path.to_string_lossy();
            path.ends_with(".swift") && !path.ends_with("Package.swift")
        })
        .collect()
}

#[test]
fn test_helpers_swift_carries_ignore_directive() {
    let files = render_files();
    let helpers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("TestHelpers.swift"))
        .expect("TestHelpers.swift is emitted");
    assert!(
        helpers.content.contains("// swift-format-ignore-file\n"),
        "TestHelpers.swift must carry the swift-format-ignore-file directive so the \
         alef-emitted 4-space-indent body is not rewritten by Apple's swift-format on \
         every pre-commit run. Content:\n{}",
        helpers.content,
    );
}

#[test]
fn test_category_class_swift_carries_ignore_directive() {
    let files = render_files();
    let category_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted");
    assert!(
        category_file.content.contains("// swift-format-ignore-file\n"),
        "Per-category XCTest swift file must carry the swift-format-ignore-file \
         directive so generated test bodies (XCTest-first imports, 4-space indent, \
         long lines) survive `swift-format` unmodified. Content:\n{}",
        category_file.content,
    );
}

#[test]
fn ignore_directive_appears_immediately_after_alef_header() {
    let files = render_files();
    for file in swift_sources(&files) {
        let mut lines = file.content.lines();
        let mut header_lines = 0;
        for line in lines.by_ref() {
            if line.starts_with("//") {
                header_lines += 1;
                if line.trim() == "// swift-format-ignore-file" {
                    break;
                }
            } else {
                panic!(
                    "swift-format-ignore-file directive missing from header of {}",
                    file.path.display()
                );
            }
        }
        assert!(
            header_lines > 0,
            "{} must have a header containing the ignore directive",
            file.path.display(),
        );
    }
}

#[test]
fn ignore_directive_appears_exactly_once_per_swift_file() {
    let files = render_files();
    for file in swift_sources(&files) {
        let count = file.content.matches("// swift-format-ignore-file").count();
        assert_eq!(
            count,
            1,
            "{} must carry the swift-format-ignore-file directive exactly once, found {}",
            file.path.display(),
            count,
        );
    }
}
