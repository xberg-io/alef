//! Regression test: a `file_path` argument is wrapped in `java.nio.file.Path.of(...)` for the
//! Kotlin/JVM binding (which re-exports the Java facade, and that facade takes a `Path`) but must
//! stay a plain `String` for kotlin_android, whose binding signature takes a `String`.
//!
//! Wrapping it for Android is a Kotlin type error in the *generated* test source, which nothing on
//! the Rust side sees; it only surfaces when the emitted Gradle project is compiled. Asserting both
//! branches here is what makes the divergence a Rust-side failure.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin::KotlinE2eCodegen;
use alef::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const PATH_VALUE: &str = "pdf/fake_memo.pdf";

fn toml_for(language: &str) -> String {
    // `namespace` is a kotlin_android-only key; the Kotlin/JVM language table rejects it. ~keep
    let namespace = if language == "kotlin_android" {
        "namespace = \"dev.sample_crate\""
    } else {
        ""
    };
    format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.{language}]
package = "dev.sample_crate"
{namespace}

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_file"
result_var = "result"
async = true

[crates.e2e.calls.extract_file]
function = "extract_file"
result_var = "result"
async = true

[[crates.e2e.calls.extract_file.args]]
name = "path"
field = "input.path"
type = "file_path"

[crates.e2e.packages.{language}]
name = "sample_crate"
"#
    )
}

fn extract_file_fixture() -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "async_extract_file".to_string(),
        category: Some("async".to_string()),
        description: "extract_file test".to_string(),
        tags: vec!["async".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file".to_string()),
        input: serde_json::json!({ "path": PATH_VALUE }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "async.json".to_string(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

fn render(language: &str) -> String {
    let toml_src = toml_for(language);
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "async".to_string(),
        fixtures: vec![extract_file_fixture()],
    }];
    let files = match language {
        "kotlin_android" => KotlinAndroidE2eCodegen
            .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
            .expect("kotlin_android codegen succeeds"),
        "kotlin" => KotlinE2eCodegen
            .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
            .expect("kotlin codegen succeeds"),
        other => panic!("unsupported language for this test: {other}"),
    };
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("AsyncTest.kt"))
        .expect("an AsyncTest.kt file is emitted")
        .content
        .clone()
}

/// The call line for the fixture's single `file_path` argument.
fn call_line(content: &str) -> String {
    content
        .lines()
        .find(|line| line.contains(PATH_VALUE))
        .unwrap_or_else(|| panic!("no emitted line references the fixture path; got:\n{content}"))
        .trim()
        .to_string()
}

#[test]
fn kotlin_android_file_paths_are_not_wrapped() {
    let content = render("kotlin_android");
    let line = call_line(&content);

    assert!(
        line.contains(&format!("\"{PATH_VALUE}\"")),
        "kotlin_android must pass the path as a plain String literal; got line:\n{line}"
    );
    assert!(
        !line.contains("Path.of("),
        "kotlin_android's binding signature takes a String, so the path must not be wrapped in \
         java.nio.file.Path.of(...); got line:\n{line}"
    );
}

#[test]
fn kotlin_jvm_file_paths_are_wrapped_in_path_of() {
    let content = render("kotlin");
    let line = call_line(&content);

    assert!(
        line.contains(&format!("java.nio.file.Path.of(\"{PATH_VALUE}\")")),
        "Kotlin/JVM re-exports the Java facade, which takes a java.nio.file.Path, so the path must \
         be wrapped; got line:\n{line}"
    );
}
