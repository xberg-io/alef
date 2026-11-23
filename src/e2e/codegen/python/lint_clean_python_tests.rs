//! Verifies alef's generated Python e2e output survives a realistic consumer `ruff check`
//! untouched -- the class of defect fixed by task #528. Two symptoms were measured against a
//! real consumer regen: `from typing import Generator` is UP035-deprecated (autofixed to
//! `from collections.abc import Generator` on every `lint --fix`), and a pile of `# noqa: `
//! directives (`PLC0415`, `S603`, `S310`, `ARG001`, and — by far the largest volume — `S101`
//! on nearly every emitted `assert`) go RUF100-dirty under a consumer's own rule selection.
//! Either symptom makes a consumer's own `lint --fix` rewrite alef's stamped output, drifting
//! it off its `alef:hash` in between and eventually out of alef's ownership entirely.
//!
//! Unlike an isolated `ruff --isolated` probe (which uses ruff's *default* rules and proves
//! nothing about a real consumer config), these tests run `ruff check` against a config that
//! actually enables `UP` + `RUF` plus every opt-in family alef's generated code deliberately
//! suppresses per-construct (`S`, `PLC0415`, `ARG`, `ANN`, `A`, `T20`, `N`, `SIM`, `B`) and the
//! import sorter (`I`), with `S101` ignored for `test_*.py`/`conftest.py` — the standard
//! pytest+bandit convention. A `# noqa` is only safe when the rule it names is both enabled
//! *and* would otherwise fire on that exact line; this harness catches both directions: an
//! unselected/redundant noqa (RUF100) and a real, unsuppressed violation.
//!
//! Skips (rather than failing) when `ruff` is not on `PATH`, mirroring
//! `e2e::codegen::go::tests::main_test_go_gofmt_tests`'s formatter self-skip.

use std::process::{Command, Stdio};

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::fixture::{Fixture, FixtureGroup};

use super::super::config::render_conftest;
use super::render_test_file;

/// See the module doc for why each family is here. `S101` is ignored the way virtually every
/// pytest project that also enables bandit rules configures it — and task #528's own probe
/// showed that even *that* standard exclusion makes a *present* `# noqa: S101` RUF100-dirty,
/// not just an absent one, since the per-file-ignore already suppresses the finding on its
/// own. ~keep
const RUFF_CONFIG: &str = r#"
[tool.ruff.lint]
select = ["S", "PLC0415", "ARG", "UP", "RUF", "I", "F", "N", "SIM", "B", "ANN", "A", "T20", "Q"]

[tool.ruff.lint.per-file-ignores]
"test_*.py" = ["S101"]
"conftest.py" = ["S101"]
"#;

fn ruff_available() -> bool {
    which::which("ruff").is_ok()
}

/// Runs `ruff check` over `files` (filename -> content) under [`RUFF_CONFIG`]. Returns the
/// combined stdout+stderr and whether the check passed.
fn run_ruff_check(files: &[(&str, &str)]) -> (bool, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("pyproject.toml"), RUFF_CONFIG).expect("write pyproject.toml");
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).expect("write python file");
    }
    let output = Command::new("ruff")
        .args(["check", "--config", "pyproject.toml", "--no-cache", "."])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("run ruff check");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Builds a mixed fixture set exercising every branch this fix touched: an HTTP fixture with
/// a JSON request+response body (hoisted `json` import, `S310` on `Request(...)`), an HTTP
/// fixture with a `uuid` header assertion (hoisted `re` import), a plain assertion, a
/// `matches_regex` assertion (`re.search`), a `not_empty` assertion, an `error` assertion
/// (`pytest.raises`/`B017`), a fixture skipped for python (`@pytest.mark.skip`), a fixture
/// with both a mock response and an env api key (the `sys.stdout.write` real/mock branch),
/// a fixture with an env api key and no mock (the `pytest.skip(...)` branch), and a visitor
/// fixture whose `visit_heading` callback needs the conditional `A002` suppression.
fn mixed_fixtures() -> Vec<Fixture> {
    let specs = [
        serde_json::json!({
            "id": "http_json_roundtrip",
            "description": "Round-trips a JSON body",
            "http": {
                "handler": {"route": "/widgets", "method": "POST"},
                "request": {"method": "POST", "path": "/widgets", "body": {"name": "alef"}},
                "expected_response": {"status_code": 200, "body": {"ok": true}}
            }
        }),
        serde_json::json!({
            "id": "http_uuid_header",
            "description": "Checks a uuid response header",
            "http": {
                "handler": {"route": "/widgets", "method": "GET"},
                "request": {"method": "GET", "path": "/widgets"},
                "expected_response": {"status_code": 200, "headers": {"x-request-id": "<<uuid>>"}}
            }
        }),
        serde_json::json!({
            "id": "basic_call",
            "description": "Calls the function",
            "input": null,
            "assertions": [{"type": "not_empty", "field": "items"}]
        }),
        serde_json::json!({
            "id": "regex_match",
            "description": "Field matches a pattern",
            "assertions": [{"type": "matches_regex", "field": "id", "value": "^[a-z]+$"}]
        }),
        serde_json::json!({
            "id": "invalid_input_errors",
            "description": "Raises on invalid input",
            "assertions": [
                {"type": "error"},
                {"type": "equals", "field": "error.status_code", "value": 429}
            ]
        }),
        serde_json::json!({
            "id": "bare_error",
            "description": "Raises without naming a variant",
            "assertions": [{"type": "error"}]
        }),
        serde_json::json!({
            "id": "quoted_unicode_text",
            "description": "Compares text holding double quotes and ambiguous unicode",
            "assertions": [{"type": "equals", "field": "title", "value": "say \"hi\" \u{203a} \u{20ac}"}]
        }),
        serde_json::json!({
            "id": "skipped_case",
            "description": "Skipped for python",
            "skip": {"languages": ["python"], "reason": "not supported"},
            "assertions": [{"type": "not_empty", "field": "items"}]
        }),
        serde_json::json!({
            "id": "real_or_mock",
            "description": "Uses the real API when a key is configured",
            "env": {"api_key_var": "MY_API_KEY"},
            "mock_response": {"status": 200, "body": {}},
            "assertions": [{"type": "not_empty", "field": "items"}]
        }),
        serde_json::json!({
            "id": "requires_real_api",
            "description": "Requires a live API key",
            "env": {"api_key_var": "MY_API_KEY"},
            "assertions": [{"type": "not_empty", "field": "items"}]
        }),
        serde_json::json!({
            "id": "visitor_heading",
            "description": "Visits headings",
            "visitor": {"callbacks": {"visit_heading": {"action": "skip"}}},
            "assertions": [{"type": "not_empty", "field": "items"}]
        }),
    ];
    specs
        .into_iter()
        .map(|spec| serde_json::from_value(spec).expect("fixture must parse"))
        .collect()
}

fn e2e_config_with_client_factory() -> E2eConfig {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(
        "python".to_string(),
        CallOverride {
            client_factory: Some("create_client".to_string()),
            ..Default::default()
        },
    );
    E2eConfig {
        call: CallConfig {
            function: "do_thing".to_string(),
            module: "mypackage".to_string(),
            result_var: "result".to_string(),
            overrides,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The measured defect, reproduced end to end: `render_conftest` and `render_test_file`'s
/// real output — not a hand-simplified stand-in — must not change under a `ruff check` that
/// enables the rule families the generator suppresses per-construct.
#[test]
fn conftest_and_test_file_survive_a_realistic_ruff_lint_pass() {
    if !ruff_available() {
        return;
    }

    let fixtures = mixed_fixtures();
    let e2e_config = e2e_config_with_client_factory();
    let groups = vec![FixtureGroup {
        category: "basic".to_string(),
        fixtures: fixtures.clone(),
    }];

    let conftest_py = render_conftest(&e2e_config, &groups, &[], &[]);
    let fixture_refs: Vec<&Fixture> = fixtures.iter().collect();
    let config = ResolvedCrateConfig::default();
    let sample_error = crate::core::ir::ErrorDef {
        exception_base: None,
        name: "SampleError".to_string(),
        rust_path: "mypackage::SampleError".to_string(),
        original_rust_path: String::new(),
        variants: vec![crate::core::ir::ErrorVariant::default()],
        doc: String::new(),
        methods: vec![crate::core::ir::MethodDef {
            name: "status_code".to_string(),
            params: Vec::new(),
            return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let test_basic_py = render_test_file(
        "basic",
        &fixture_refs,
        &e2e_config,
        &config,
        &[],
        &[],
        &[],
        std::slice::from_ref(&sample_error),
        false,
    );

    let (success, output) = run_ruff_check(&[
        ("conftest.py", conftest_py.as_str()),
        ("test_basic.py", test_basic_py.as_str()),
    ]);
    assert!(
        success,
        "generated Python must survive a realistic ruff lint pass untouched; ruff reported:\n{output}\n\
         --- conftest.py ---\n{conftest_py}\n--- test_basic.py ---\n{test_basic_py}"
    );
}

/// The visitor context probe is new emitted code with its own `# noqa` surface, so it has to
/// clear the same realistic lint pass. It renders the SAME mixed fixture set as the test above --
/// filtering down to just the visitor fixture drops the two env-var fixtures that consume the
/// hoisted `import os`, and ruff then fails the file on an unused import that has nothing to do
/// with the probe. The `assert!`s before the ruff run are the point: without a bridge that
/// declares `visit_heading` no probe is emitted at all, and ruff would report a clean file that
/// never contained the code under test. ~keep
#[test]
fn visitor_context_probe_survives_a_realistic_ruff_lint_pass() {
    if !ruff_available() {
        return;
    }

    let fixtures = mixed_fixtures();
    let e2e_config = e2e_config_with_client_factory();
    let config = ResolvedCrateConfig {
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "DocumentWalker".to_string(),
            type_alias: Some("DocumentWalkerHandle".to_string()),
            context_type: Some("TraversalState".to_string()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = vec![
        crate::core::ir::TypeDef {
            name: "DocumentWalker".to_string(),
            rust_path: "mypackage::DocumentWalker".to_string(),
            is_trait: true,
            methods: vec![crate::core::ir::MethodDef {
                name: "visit_heading".to_string(),
                has_default_impl: true,
                ..Default::default()
            }],
            ..crate::core::ir::TypeDef::default()
        },
        crate::core::ir::TypeDef {
            name: "TraversalState".to_string(),
            rust_path: "mypackage::TraversalState".to_string(),
            // Required for the bridge (and this probe) to resolve the class path rather than the
            // dict path -- see `context_binding_class` / `eligible_context_def`. Without this the
            // generated visitor never dereferences `ctx` and every assertion below the render call
            // would pass over a probe that was never emitted. ~keep
            is_clone: true,
            fields: vec![crate::core::ir::FieldDef {
                name: "node_kind".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            methods: vec![crate::core::ir::MethodDef {
                name: "attributes".to_string(),
                return_type: crate::core::ir::TypeRef::Map(
                    Box::new(crate::core::ir::TypeRef::String),
                    Box::new(crate::core::ir::TypeRef::String),
                ),
                receiver: Some(crate::core::ir::ReceiverKind::Ref),
                ..Default::default()
            }],
            ..crate::core::ir::TypeDef::default()
        },
    ];

    let fixture_refs: Vec<&Fixture> = fixtures.iter().collect();
    let rendered = render_test_file(
        "visitor",
        &fixture_refs,
        &e2e_config,
        &config,
        &type_defs,
        &[],
        &[],
        &[],
        false,
    );

    assert!(
        rendered.contains("self._probe_traversal_state(ctx)"),
        "the generated visitor must dereference its context argument:\n{rendered}"
    );
    assert!(
        rendered.contains("\"node_kind\",") && rendered.contains("\"attributes\","),
        "the probe must read the context type's declared field and call its declared method:\n{rendered}"
    );
    assert!(
        rendered.contains("assert not _visitor.context_errors"),
        "the recorded probe failures must be asserted in the test body:\n{rendered}"
    );

    let (success, output) = run_ruff_check(&[("test_visitor.py", rendered.as_str())]);
    assert!(
        success,
        "the generated visitor context probe must survive a realistic ruff lint pass; ruff reported:\n{output}\n\
         --- test_visitor.py ---\n{rendered}"
    );
}

/// Negative control: a deliberately deprecated import run through the exact same harness must
/// still be flagged. Without this, a silently-misconfigured `run_ruff_check` (wrong config
/// path, `ruff` swallowing an error, an empty `select`) would render the positive test above
/// as a pass no matter what it was fed — proving nothing.
#[test]
fn the_harness_still_flags_a_deliberately_deprecated_import() {
    if !ruff_available() {
        return;
    }

    let bad = "def f() -> None:\n    from typing import Generator\n    x: Generator = iter([])\n    assert x\n";
    let (success, output) = run_ruff_check(&[("test_negative_control.py", bad)]);
    assert!(!success, "the harness must be able to fail; ruff reported:\n{output}");
    assert!(
        output.contains("UP035"),
        "expected a UP035 (deprecated `typing.Generator` import) finding, got:\n{output}"
    );
}
