//! Regression coverage for a one-sided-trim bug in `equals` assertion codegen.
//!
//! The bug: several languages wrapped the *actual* (result) side of an `equals`
//! assertion in a trim/strip call (`.strip()`, `.trim()`, `trimws(...)`, etc.) while
//! emitting the fixture's `expected` literal verbatim. Fixture expectations may
//! legitimately end in `\n` (e.g. table/blockquote conversion output), so trimming only
//! one side made those assertions permanently unsatisfiable. Trimming *both* sides would
//! "fix" that but silently mask real trailing-whitespace regressions — the contract here
//! is exact equality: neither side is normalized.
//!
//! This is a single data-driven table test, parameterized over all 15 language codegens
//! that H2M (html-to-markdown) consumes, so a future per-language regression in any one
//! of them is caught by this one test rather than requiring 15 separate discoveries.
//! (`wasm` is intentionally excluded: its codegen delegates directly to the `typescript`
//! backend and shares its `assertions.rs`, so it has no independent code path to regress.)
//!
//! `rust` was missing from this table even though it has its own independent
//! `render_equals_assertion` path (`src/e2e/codegen/rust/assertion_helpers.rs`) with its
//! own no-trim contract and unit tests — those per-backend unit tests don't provide the
//! cross-language guard this table does, so the gap left Rust's exact-equality contract
//! unverified at this level (alef task #162).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::c::CCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::r::RCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::codegen::typescript::TypeScriptCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// Substrings that indicate a normalizing call was applied to one (or both) sides of an
/// `equals` comparison. Any of these appearing in a generated equals-assertion test file
/// means the exact-equality contract was violated.
const TRIM_MARKERS: &[&str] = &[
    ".strip()",
    ".trim()",
    "trimws(",
    "String.trim(",
    "trimmingCharacters",
    "str_trim_eq",
    "std.mem.trim",
    ".Trim(",
    "strings.TrimSpace",
];

fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "demo-markup-rs"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "dm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "demo_markup"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]

[crates.e2e.call.overrides.python]
module = "demo_markup"
function = "convert"

[crates.e2e.call.overrides.node]
module = "@demo/markup"
function = "convert"

[crates.e2e.call.overrides.go]
module = "github.com/demo/markup"
function = "Convert"
alias = "dm"
returns_result = true

[crates.e2e.call.overrides.java]
class = "io.demo.markup.DemoMarkup"
function = "convert"

[crates.e2e.call.overrides.csharp]
class = "DemoMarkup"
function = "Convert"

[crates.e2e.call.overrides.php]
module = "DemoMarkup"
class = "DemoMarkup"
function = "convert"

[crates.e2e.call.overrides.ruby]
module = "demo_markup"

[crates.e2e.call.overrides.elixir]
module = "DemoMarkup"
function = "convert"

[crates.e2e.call.overrides.r]
module = "demomarkup"
function = "convert"

[crates.e2e.call.overrides.c]
header = "demo_markup.h"
function = "dm_convert"
prefix = "dm"

[crates.e2e.call.overrides.kotlin_android]
class = "io.demo.android.DemoMarkup"
function = "convert"

[crates.e2e.call.overrides.swift]
module = "DemoMarkup"
function = "convert"

[crates.e2e.call.overrides.dart]
module = "dm"
function = "convert"

[crates.e2e.call.overrides.zig]
module = "demo_markup"
function = "convert"
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn resolve(
    cfg: &NewAlefConfig,
) -> (
    alef::core::config::ResolvedCrateConfig,
    alef::core::config::e2e::E2eConfig,
) {
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

/// A bare-result `equals` assertion whose expected value legitimately ends in a trailing
/// newline, mirroring real table/blockquote fixture output.
fn build_fixture_with_trailing_newline_equals() -> FixtureGroup {
    FixtureGroup {
        category: "conversion".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "equals_trailing_newline".to_string(),
            category: Some("conversion".to_string()),
            description: "equals assertion against a value with a trailing newline".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<p>hi</p>" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "equals".to_string(),
                field: None,
                value: Some(serde_json::Value::String("line one\nline two\n".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

/// Runs one language's codegen over the shared fixture and returns the concatenated
/// content of every generated file (the assertion may land in a test file whose name
/// varies per backend, so we don't try to guess a single filename).
fn generate_all_content(codegen: &dyn E2eCodegen) -> String {
    let cfg = build_config();
    let (resolved, e2e) = resolve(&cfg);
    let groups = vec![build_fixture_with_trailing_newline_equals()];
    let files = codegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .unwrap_or_else(|error| panic!("{} generation failed: {error:#}", codegen.language_name()));
    files
        .iter()
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n----\n")
}

/// One named case per language. `cargo test equals_assertion_is_exact_for_` runs the
/// whole table; a failure names the exact language that regressed.
macro_rules! equals_is_exact_case {
    ($name:ident, $codegen:expr) => {
        #[test]
        fn $name() {
            let content = generate_all_content(&$codegen);
            for marker in TRIM_MARKERS {
                assert!(
                    !content.contains(marker),
                    "{}: equals assertion must not trim/normalize either side (found {marker:?}). Content:\n{content}",
                    $codegen.language_name(),
                );
            }
            assert!(
                content.contains("line one\\nline two\\n") || content.contains("line one\nline two\n"),
                "{}: expected literal with trailing newline must be emitted verbatim. Content:\n{content}",
                $codegen.language_name(),
            );
        }
    };
}

equals_is_exact_case!(equals_assertion_is_exact_for_rust, RustE2eCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_python, PythonE2eCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_node, TypeScriptCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_go, GoCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_java, JavaCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_csharp, CSharpCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_php, PhpCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_ruby, RubyCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_elixir, ElixirCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_r, RCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_c, CCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_kotlin_android, KotlinAndroidE2eCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_swift, SwiftE2eCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_dart, DartE2eCodegen);
equals_is_exact_case!(equals_assertion_is_exact_for_zig, ZigE2eCodegen);
