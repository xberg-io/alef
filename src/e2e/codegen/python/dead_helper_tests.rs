//! Dead-helper-emission regression tests split out of `test_file.rs` (matching the
//! `import_lines.rs`/`lint_clean_python_tests.rs`/`test_file_misc_tests.rs` split) to keep
//! `test_file.rs` under its baselined file-size ceiling. These tests cover the
//! `_alef_e2e_item_texts`/`_alef_e2e_text` dead-helper defect end to end: the fixture builder
//! they share, the gating predicate, and the per-file consistency invariant -- none of it
//! depends on anything else in `mod tests`, so it moves cleanly. ~keep

use super::*;

fn minimal_fixture(id: &str, assertions: Vec<crate::e2e::fixture::Assertion>) -> crate::e2e::fixture::Fixture {
    crate::e2e::fixture::Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        description: "Smoke test".to_string(),
        input: serde_json::Value::Null,
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions,
        call: None,
        skip: None,
        env: None,
        setup: Vec::new(),
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        mock_response: None,
        source: String::new(),
        category: None,
        tags: Vec::new(),
    }
}

/// Regression test for the dead-helper defect: `_alef_e2e_item_texts`/
/// `_alef_e2e_text` were emitted into every generated file unconditionally,
/// even when no assertion in the file ever called them (defined but
/// referenced zero times, in all 44 python e2e files at time of writing).
/// A fixture whose only assertion is `not_error` never calls the helper.
#[test]
fn render_test_file_without_array_assertions_omits_the_dead_item_text_helper() {
    let fixture = minimal_fixture(
        "widget_smoke",
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
    );
    let fixtures: Vec<&crate::e2e::fixture::Fixture> = vec![&fixture];
    let e2e_config = crate::e2e::config::E2eConfig::default();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let out = render_test_file(
        "smoke",
        &fixtures,
        &e2e_config,
        &config,
        &type_defs,
        &enums,
        &[],
        &[],
        false,
    );

    assert!(
        !out.contains("_alef_e2e_item_texts"),
        "a file with no array-contains assertion must not define the unused helper, got: {out}"
    );
    assert!(!out.contains("_alef_e2e_text"), "got: {out}");
}

/// The gating predicate for the dead-helper fix: `references_identifier` must
/// find the helper call as it is actually emitted (a real call site inside a
/// generator expression), and must not false-negative on it.
#[test]
fn references_identifier_finds_the_item_texts_helper_call_site() {
    let emitted_assertion = "        assert any(\"Function\" in text for item in result.structure for text in _alef_e2e_item_texts(item))\n";
    assert!(references_identifier(emitted_assertion, "_alef_e2e_item_texts"));

    let emitted_without_helper = "        assert result.content == \"hello\"\n";
    assert!(!references_identifier(emitted_without_helper, "_alef_e2e_item_texts"));
}

/// Collect the `_alef_e2e_*` helpers a generated file defines.
fn helpers_defined_in(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("def "))
        .filter(|rest| rest.starts_with("_alef_e2e_"))
        .filter_map(|rest| rest.split('(').next())
        .map(str::to_string)
        .collect()
}

/// Collect the `_alef_e2e_*` helpers a generated file calls, including calls made from
/// inside another helper's body — those are undefined names too.
fn helpers_called_in(source: &str) -> BTreeSet<String> {
    const PREFIX: &str = "_alef_e2e_";
    let mut called = BTreeSet::new();
    for line in source.lines().filter(|line| !line.starts_with("def ")) {
        let mut rest = line;
        while let Some(at) = rest.find(PREFIX) {
            let tail = &rest[at..];
            let name: String = tail
                .chars()
                .take_while(|character| character.is_alphanumeric() || *character == '_')
                .collect();
            if tail[name.len()..].starts_with('(') {
                called.insert(name);
            }
            rest = &tail[PREFIX.len()..];
        }
    }
    called
}

/// Regression test for the split-helper defect: `_alef_e2e_text` has two independent
/// callers — the enum `equals` assertion and `_alef_e2e_item_texts`' own body — but its
/// definition was gated on `_alef_e2e_item_texts` alone. Its definition therefore landed
/// only in the file that happened to carry an array assertion, and the enum-assertion
/// files called it undefined (22 x F821, which fails `alef all` at the ruff stage).
///
/// The invariant is per-file, so it is only violated across a *set* of categories: each
/// file in isolation is either self-consistent or the one that carries the definition.
#[test]
fn every_generated_python_file_defines_the_helpers_it_calls() {
    let enum_equals = minimal_fixture(
        "enum_equals",
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some("structure[0].kind".to_string()),
            value: Some(serde_json::json!("Function")),
            ..Default::default()
        }],
    );
    let array_contains = minimal_fixture(
        "array_contains",
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "contains".to_string(),
            field: Some("structure".to_string()),
            value: Some(serde_json::json!("Function")),
            ..Default::default()
        }],
    );
    let no_helpers = minimal_fixture(
        "no_helpers",
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
    );

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.fields_array.insert("structure".to_string());
    // The fixture's enum type must be explicit when this test supplies no IR.
    e2e_config.fields_enum.insert("structure[0].kind".to_string());
    let config = crate::core::config::ResolvedCrateConfig::default();

    let suite: Vec<(&str, String)> = [
        ("enum_equals", &enum_equals),
        ("array_contains", &array_contains),
        ("no_helpers", &no_helpers),
    ]
    .into_iter()
    .map(|(category, fixture)| {
        let fixtures: Vec<&crate::e2e::fixture::Fixture> = vec![fixture];
        let out = render_test_file(category, &fixtures, &e2e_config, &config, &[], &[], &[], &[], false);
        (category, out)
    })
    .collect();

    // Without both helpers actually reaching the emitted suite the invariant below is
    // vacuous: it is satisfied by a suite that calls nothing at all. ~keep
    let enum_file = &suite[0].1;
    let array_file = &suite[1].1;
    assert!(
        helpers_called_in(enum_file).contains("_alef_e2e_text"),
        "the enum `equals` assertion must call `_alef_e2e_text`, got:\n{enum_file}"
    );
    assert!(
        helpers_called_in(array_file).contains("_alef_e2e_item_texts"),
        "the array `contains` assertion must call `_alef_e2e_item_texts`, got:\n{array_file}"
    );

    for (category, out) in &suite {
        let defined = helpers_defined_in(out);
        let called = helpers_called_in(out);
        let undefined: Vec<&String> = called.difference(&defined).collect();
        assert!(
            undefined.is_empty(),
            "test_{category}.py calls undefined helpers {undefined:?}, got:\n{out}"
        );
        let unused: Vec<&String> = defined.difference(&called).collect();
        assert!(
            unused.is_empty(),
            "test_{category}.py defines unused helpers {unused:?}, got:\n{out}"
        );
    }

    assert_eq!(
        helpers_defined_in(&suite[2].1),
        BTreeSet::new(),
        "a file with no helper call must define no helpers, got:\n{}",
        suite[2].1
    );
}
