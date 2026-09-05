//! Generation-order regression coverage across several e2e codegen backends.
//!
//! Every existing determinism test in this repo is pinned to the one site that previously
//! broke: `tests/backends_swift_deterministic_generation_test.rs` covers only Swift bindings,
//! and `tests/e2e_php_accessor_extraction_ordering.rs` only PHP's e2e generator. Nothing
//! walked the Go, Rust, Python, or TypeScript e2e emitters, which is exactly why a real bug
//! survived there: `e2e::codegen::go`'s env-var-forwarding loop iterated a `HashMap` straight
//! into `main_test.go`, so two back-to-back `alef e2e generate` runs over the *same* config
//! could emit the forwarding blocks in a different order -- a spurious diff with no source
//! change behind it.
//!
//! This test renders one fixture set through four language backends -- go, rust, python, and
//! node (TypeScript) -- many times in the same process and asserts every emitted file is
//! byte-identical across all renders. The config deliberately populates `[crates.e2e.env]`
//! with several entries and `[crates.e2e.calls]` with more than one named call, since those
//! were exactly the `HashMap`/`HashSet` fields whose straight-to-output iteration this
//! determinism-hardening pass fixed (they are now `BTreeMap`s and iterate in key order by
//! construction, but this test exists so a *future* map-typed field added to `E2eConfig` or a
//! per-backend renderer gets caught the same way, not just the ones fixed today).
use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::codegen::typescript::TypeScriptCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use std::path::PathBuf;

/// `HashMap`/`HashSet` iteration order can coincide across two renders by chance (small maps
/// especially), so a single re-render is not enough to make an accidental pass unlikely.
/// Follows the same precedent and count as `backends_swift_deterministic_generation_test.rs`.
const REPEATED_GENERATION_COUNT: usize = 32;

fn build_config(language: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "demo_widgets"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.env]
ZEBRA_ALLOW_PRIVATE_NETWORK = "z"
ALPHA_ALLOW_PRIVATE_NETWORK = "a"
MIKE_ALLOW_PRIVATE_NETWORK = "m"
BRAVO_ALLOW_PRIVATE_NETWORK = "b"

[crates.e2e.call]
function = "make_widget"
module = "DemoWidgets"
result_var = "result"
async = true
returns_result = true

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"

[crates.e2e.calls.secondary]
function = "make_widget_variant"
module = "DemoWidgets"
result_var = "result"
async = true
returns_result = true

[[crates.e2e.calls.secondary.args]]
name = "name"
field = "input.name"
type = "string"
"#,
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn build_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![
            Fixture {
                id: "smoke_default_call".to_string(),
                category: Some("smoke".to_string()),
                description: "default call".to_string(),
                input: serde_json::json!({ "name": "first" }),
                assertions: vec![Assertion {
                    assertion_type: "not_error".to_string(),
                    ..Assertion::default()
                }],
                source: "smoke.json".to_string(),
                ..Fixture::default()
            },
            Fixture {
                id: "smoke_secondary_call".to_string(),
                category: Some("smoke".to_string()),
                description: "named secondary call".to_string(),
                call: Some("secondary".to_string()),
                input: serde_json::json!({ "name": "second" }),
                assertions: vec![Assertion {
                    assertion_type: "not_error".to_string(),
                    ..Assertion::default()
                }],
                source: "smoke.json".to_string(),
                ..Fixture::default()
            },
        ],
    }
}

/// Render once and return every emitted file as `(path, content)`, sorted by path so this
/// helper's own comparison is not itself sensitive to the order `generate` returns files in --
/// only to whether each file's *content* is stable.
fn render_once(codegen: &dyn E2eCodegen, language: &str) -> Vec<(PathBuf, String)> {
    let (e2e, resolved) = build_config(language);
    let groups = vec![build_fixture_group()];
    let mut files: Vec<(PathBuf, String)> = codegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .unwrap_or_else(|err| panic!("{language} e2e generation must succeed: {err:?}"))
        .into_iter()
        .map(|f| (f.path, f.content))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn assert_stable_across_repeated_generation(codegen: &dyn E2eCodegen, language: &str) {
    let expected = render_once(codegen, language);
    assert!(
        !expected.is_empty(),
        "{language} e2e codegen must emit at least one file"
    );

    for attempt in 1..REPEATED_GENERATION_COUNT {
        let actual = render_once(codegen, language);
        assert_eq!(
            actual, expected,
            "{language} e2e codegen must be byte-identical across repeated generation of the \
             same config (mismatch on attempt {attempt} of {REPEATED_GENERATION_COUNT}); a \
             `HashMap`/`HashSet` iterating straight into emitted output is randomly seeded per \
             process and would fail this assertion intermittently rather than deterministically."
        );
    }
}

#[test]
fn go_e2e_codegen_is_deterministic_across_repeated_generation() {
    assert_stable_across_repeated_generation(&GoCodegen, "go");
}

#[test]
fn rust_e2e_codegen_is_deterministic_across_repeated_generation() {
    assert_stable_across_repeated_generation(&RustE2eCodegen, "rust");
}

#[test]
fn python_e2e_codegen_is_deterministic_across_repeated_generation() {
    assert_stable_across_repeated_generation(&PythonE2eCodegen, "python");
}

#[test]
fn typescript_e2e_codegen_is_deterministic_across_repeated_generation() {
    assert_stable_across_repeated_generation(&TypeScriptCodegen, "node");
}
