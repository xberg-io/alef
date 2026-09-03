//! Python test-file import-line computation, split out of `test_file.rs` (over the
//! 1,000-line file-size cap) to keep the touched concern's growth out of that file.
//!
//! Declared as a submodule of `test_file` (its only caller), not a sibling under `python`,
//! so `super` below reaches `test_file` first and `python::helpers` needs one more `super`.

use std::collections::BTreeSet;

use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;

use super::super::helpers::is_skipped;
use super::super::test_function::{KwargRenderContext, UsedTypeNames, render_kwarg_field_value};
use super::references_identifier;

/// Collect nested config/struct AND enum type names referenced by a `json_object` arg's value --
/// both the single-object shape (a field whose own type is a generated pyclass, e.g. `nested:
/// NestedConfig` inside `ExtractionConfig`, or an enum-typed field at any nesting depth, e.g.
/// `preset: PreprocessingPreset` inside `PreprocessingOptions` inside `ConversionConfig`) and the
/// "batch" array-of-typed-items shape (`element_type`, e.g. `BatchFileItem`).
///
/// Runs the identical traversal `render_kwarg_field_value` (typed_values.rs) uses to emit the
/// actual constructor calls, discarding the rendered text and keeping only the type names it
/// references -- so what gets constructed and what gets imported cannot silently disagree.
///
/// Struct and enum names land in the caller's two separate sets (`used_config_types` /
/// `used_enum_types`), mirroring the split `render_kwarg_field_value` already returns via
/// `UsedTypeNames`. Merging both into one set here would fold every enum-typed field --
/// including the top-level ones `collect_json_object_enum_types` already finds -- into the
/// config-class import group, alphabetically interleaving two groups the import line otherwise
/// keeps apart and reordering every generated file with more than one enum, not just the ones
/// missing an import. ~keep
pub(super) fn collect_nested_config_types(
    arg: &ArgMapping,
    value: &serde_json::Value,
    constructor_type: Option<&str>,
    context: KwargRenderContext<'_>,
    used_config_types: &mut BTreeSet<String>,
    used_enum_types: &mut BTreeSet<String>,
) {
    if let Some(obj) = value.as_object() {
        for (key, field_value) in obj.iter() {
            let mut nested = UsedTypeNames::default();
            let _ = render_kwarg_field_value(
                key,
                field_value,
                constructor_type,
                &format!("/{key}"),
                context,
                &mut nested,
            );
            used_config_types.extend(nested.structs);
            used_enum_types.extend(nested.enums);
        }
    }

    if let Some(elem_type) = &arg.element_type
        && let Some(arr) = value.as_array()
    {
        for item in arr.iter().filter_map(|v| v.as_object()) {
            for (key, field_value) in item.iter() {
                let mut nested = UsedTypeNames::default();
                let _ = render_kwarg_field_value(
                    key,
                    field_value,
                    Some(elem_type.as_str()),
                    &format!("/{key}"),
                    context,
                    &mut nested,
                );
                used_config_types.extend(nested.structs);
                used_enum_types.extend(nested.enums);
            }
        }
    }
}

/// `import pytest` and the `sys.stdout.write` diagnostic branch are only genuinely
/// referenced in the emitted body under specific conditions. Mirroring those exactly
/// (rather than a coarser "any fixture has an env api key" check) means each import is
/// only emitted when it will actually be used — a fixture with both a mock response AND
/// an env api key never reaches the `pytest.skip(...)` branch in `test_function.rs` (it
/// takes the mock/real-API `sys.stdout.write` branch instead), so blanket-including
/// `pytest` for it produced a real unused import; blanket-including `sys` for every env
/// api key fixture (mock or not) did the same for the print/`T201` branch. ~keep
pub(super) fn compute_pytest_and_sys_import_needs(
    fixtures: &[&Fixture],
    client_factory: Option<&str>,
    has_error_test: bool,
    is_async: bool,
) -> (bool, bool) {
    let has_skipped_fixture = fixtures
        .iter()
        .filter(|f| !f.is_http_test())
        .any(|f| is_skipped(f, "python"));
    let has_pytest_skip_call = client_factory.is_some()
        && fixtures.iter().filter(|f| !f.is_http_test()).any(|f| {
            let has_mock = f.mock_response.is_some() || f.http.is_some();
            !has_mock && f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some()
        });
    let needs_pytest = has_error_test || is_async || has_skipped_fixture || has_pytest_skip_call;

    let needs_sys_import = client_factory.is_some()
        && fixtures.iter().filter(|f| !f.is_http_test()).any(|f| {
            let has_mock = f.mock_response.is_some() || f.http.is_some();
            has_mock && f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some()
        });

    (needs_pytest, needs_sys_import)
}

/// Which stdlib/third-party bare imports a rendered test file needs, decided by the caller from
/// fixture/config inspection before the body is rendered. Bundled because every flag here is
/// read-only, invariant input to [`finalize_stdlib_and_bare_imports`] -- unlike
/// `stdlib_imports`/`thirdparty_bare`, which that function mutates, so those stay their own
/// `&mut Vec<String>` parameters rather than folding into this `Copy` struct (the same split
/// `KwargRenderContext`/`ArgSink` draw in `typed_values.rs`).
#[derive(Clone, Copy)]
pub(super) struct ImportNeeds {
    pub has_http_tests: bool,
    pub needs_base64_import: bool,
    pub needs_json_import: bool,
    pub needs_os_import: bool,
    pub needs_path_import: bool,
    pub needs_sys_import: bool,
    pub needs_pytest: bool,
}

/// Finalizes `stdlib_imports`/`thirdparty_bare`: adds `json`/`re` when the already-rendered
/// `fixtures_body` actually references them (`http_test.jinja` only needs those modules for
/// fixtures whose request/response shape reaches the branch that uses them — reading the
/// answer off the rendered body keeps this the one source of truth instead of a second copy
/// of `http.rs`'s branch conditions that could silently drift from it), adds the
/// unconditional/precomputed entries, and sorts each list isort-canonically. ~keep
pub(super) fn finalize_stdlib_and_bare_imports(
    fixtures_body: &str,
    needs: ImportNeeds,
    stdlib_imports: &mut Vec<String>,
    thirdparty_bare: &mut Vec<String>,
) {
    let needs_json_import = needs.needs_json_import
        || references_identifier(fixtures_body, "json.dumps")
        || references_identifier(fixtures_body, "json.loads");
    let needs_re_import =
        references_identifier(fixtures_body, "re.match") || references_identifier(fixtures_body, "re.search");

    if needs.needs_base64_import {
        stdlib_imports.push("import base64".to_string());
    }
    if needs_json_import {
        stdlib_imports.push("import json".to_string());
    }
    if needs.needs_os_import {
        stdlib_imports.push("import os".to_string());
    }
    if needs.needs_path_import {
        stdlib_imports.push("from pathlib import Path".to_string());
    }
    if needs_re_import {
        stdlib_imports.push("import re".to_string());
    }
    if needs.has_http_tests {
        stdlib_imports.push("import urllib.request".to_string());
    }
    if needs.needs_sys_import {
        stdlib_imports.push("import sys".to_string());
    }
    if needs.needs_pytest {
        thirdparty_bare.push("import pytest".to_string());
    }
    // A plain lexicographic sort interleaves `from X import Y` before `import Z` whenever X
    // sorts earlier than Z (e.g. "from pathlib import Path" before "import os"), which isort
    // (ruff's I001) rejects — it wants every `import X` line before every `from X import Y`
    // line within a section. Sorting on `(is_from, line)` gets both groups right and each one
    // alphabetized without maintaining two separate Vecs end-to-end. ~keep
    stdlib_imports.sort_by(|a, b| (a.starts_with("from "), a).cmp(&(b.starts_with("from "), b)));
    thirdparty_bare.sort();
}

/// Narrow each `from <module> import <names>` line to the names the emitted unit actually
/// references, dropping any line left with no names.
///
/// The import candidates are over-approximated from config (call args, option types, enum and
/// nested types, trait-bridge teardown functions), so the emitted unit is the authority on which
/// of them are real references. Pruning against it keeps the two directions of the invariant in
/// one place: nothing referenced goes unimported, and nothing imported goes unreferenced. ~keep
pub(super) fn prune_unreferenced_from_imports(imports: &mut Vec<String>, emitted: &[&str]) {
    let pruned: Vec<String> = imports
        .iter()
        .filter_map(|line| {
            let Some((prefix, names)) = line.split_once(" import ") else {
                return Some(line.clone());
            };
            let kept: Vec<&str> = names
                .split(", ")
                .map(str::trim)
                .filter(|name| emitted.iter().any(|source| references_identifier(source, name)))
                .collect();
            if kept.is_empty() {
                return None;
            }
            Some(format!("{prefix} import {}", kept.join(", ")))
        })
        .collect();
    *imports = pruned;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    use crate::e2e::codegen::python::test_function::LeafSource;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashMap;

    fn config_arg() -> ArgMapping {
        ArgMapping {
            name: "config".to_string(),
            field: "input.config".to_string(),
            arg_type: "json_object".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// Direct coverage of `collect_nested_config_types` itself: before this test, only its
    /// definition and its one call site (`test_file.rs`) existed -- nothing exercised the
    /// function in isolation, only its rendered *constructor* text via `render_kwarg_field_value`.
    #[test]
    fn collect_nested_config_types_collects_single_object_nested_type() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("NestedConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let arg = config_arg();
        let value = serde_json::json!({"nested": {"value": "x"}});
        let mut used_config_types: BTreeSet<String> = BTreeSet::new();
        let mut used_enum_types: BTreeSet<String> = BTreeSet::new();
        let context = KwargRenderContext {
            type_defs: &type_defs,
            enums: &[],
            enum_fields: &HashMap::new(),
            docs_files: &[],
            leaf_source: LeafSource::Literal,
        };
        collect_nested_config_types(
            &arg,
            &value,
            Some("ExtractionConfig"),
            context,
            &mut used_config_types,
            &mut used_enum_types,
        );

        assert_eq!(
            used_config_types,
            ["NestedConfig".to_string()].into_iter().collect(),
            "the nested struct type must be collected for import, got: {used_config_types:?}"
        );
    }

    /// Map counterpart: a field typed `Map<String, NestedConfig>` must also surface its value
    /// type through `collect_nested_config_types`, mirroring the same-shaped construction fix in
    /// `typed_values.rs`.
    #[test]
    fn collect_nested_config_types_collects_map_value_nested_type() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "profiles".to_string(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("NestedConfig".to_string())),
                ),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let arg = config_arg();
        let value = serde_json::json!({"profiles": {"first": {"value": "x"}}});
        let mut used_config_types: BTreeSet<String> = BTreeSet::new();
        let mut used_enum_types: BTreeSet<String> = BTreeSet::new();
        let context = KwargRenderContext {
            type_defs: &type_defs,
            enums: &[],
            enum_fields: &HashMap::new(),
            docs_files: &[],
            leaf_source: LeafSource::Literal,
        };
        collect_nested_config_types(
            &arg,
            &value,
            Some("ExtractionConfig"),
            context,
            &mut used_config_types,
            &mut used_enum_types,
        );

        assert_eq!(
            used_config_types,
            ["NestedConfig".to_string()].into_iter().collect(),
            "the map value's struct type must be collected for import, got: {used_config_types:?}"
        );
    }

    /// The "batch" (`element_type`) shape counterpart: a nested struct field inside each array
    /// element must also be collected.
    #[test]
    fn collect_nested_config_types_collects_batch_element_nested_type() {
        let item_type = TypeDef {
            name: "BatchFileItem".to_string(),
            rust_path: "demo::BatchFileItem".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("NestedConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![item_type, inner];

        let mut arg = config_arg();
        arg.element_type = Some("BatchFileItem".to_string());
        let value = serde_json::json!([{"nested": {"value": "x"}}]);
        let mut used_config_types: BTreeSet<String> = BTreeSet::new();
        let mut used_enum_types: BTreeSet<String> = BTreeSet::new();
        let context = KwargRenderContext {
            type_defs: &type_defs,
            enums: &[],
            enum_fields: &HashMap::new(),
            docs_files: &[],
            leaf_source: LeafSource::Literal,
        };
        collect_nested_config_types(
            &arg,
            &value,
            None,
            context,
            &mut used_config_types,
            &mut used_enum_types,
        );

        assert_eq!(
            used_config_types,
            ["NestedConfig".to_string()].into_iter().collect(),
            "the batch element's nested struct type must be collected for import, got: {used_config_types:?}"
        );
    }

    /// The other half of the invariant `collect_nested_config_types` exists to protect: a type
    /// collected as a *candidate* import that the emitted body never actually references (e.g.
    /// because a docs-file redirect took the field down a different rendering path than the
    /// import scan assumed) must not survive into the final import line. `prune_unreferenced_from_imports`
    /// is what enforces that against the real rendered output.
    #[test]
    fn prune_unreferenced_from_imports_drops_a_collected_but_unreferenced_type() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("GhostConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "GhostConfig".to_string(),
            rust_path: "demo::GhostConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let arg = config_arg();
        let value = serde_json::json!({"nested": {"value": "x"}});
        let mut used_config_types: BTreeSet<String> = BTreeSet::new();
        let mut used_enum_types: BTreeSet<String> = BTreeSet::new();
        let context = KwargRenderContext {
            type_defs: &type_defs,
            enums: &[],
            enum_fields: &HashMap::new(),
            docs_files: &[],
            leaf_source: LeafSource::Literal,
        };
        collect_nested_config_types(
            &arg,
            &value,
            Some("ExtractionConfig"),
            context,
            &mut used_config_types,
            &mut used_enum_types,
        );
        assert!(
            used_config_types.contains("GhostConfig"),
            "test setup: GhostConfig must be collected as a candidate for this to be a real test of pruning"
        );

        let mut imports = vec!["from sample_pkg import process, ExtractionConfig, GhostConfig".to_string()];
        // Simulates a rendered body that only ever constructed `ExtractionConfig(...)` --
        // `GhostConfig` never actually appears as a referenced identifier.
        let emitted_body = "    result = process(opts=ExtractionConfig(nested={\"value\": \"x\"}))\n";
        prune_unreferenced_from_imports(&mut imports, &[emitted_body]);

        assert_eq!(
            imports,
            vec!["from sample_pkg import process, ExtractionConfig".to_string()],
            "a collected-but-unreferenced type must not survive into the import line, got: {imports:?}"
        );
    }

    fn fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            description: "Smoke test".to_string(),
            input,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            }],
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

    fn e2e_config_with_options_type(options_type: &str) -> crate::e2e::config::E2eConfig {
        let mut e2e_config = crate::e2e::config::E2eConfig::default();
        e2e_config.call.module = "sample_pkg".to_string();
        e2e_config.call.function = "process".to_string();
        e2e_config.call.args = vec![config_arg()];
        e2e_config.call.overrides.insert(
            "python".to_string(),
            crate::e2e::config::CallOverride {
                options_type: Some(options_type.to_string()),
                ..Default::default()
            },
        );
        e2e_config
    }

    /// End-to-end coverage of the import half of the nested-config fix: `render_test_file`
    /// (not a hand-populated `used_config_types`, which every prior test used) must run the real
    /// scan -- through `collect_nested_config_types` -- and land a `from <module> import
    /// <Class>` line naming the nested class, alongside the constructor call that references it.
    #[test]
    fn render_test_file_imports_nested_config_class_for_object_field() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("NestedConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let e2e_config = e2e_config_with_options_type("ExtractionConfig");
        let config = crate::core::config::ResolvedCrateConfig::default();
        let fixture = fixture_with_input(
            "nested_object",
            serde_json::json!({"config": {"nested": {"value": "x"}}}),
        );
        let fixtures: Vec<&Fixture> = vec![&fixture];

        let out = super::super::render_test_file(
            "smoke",
            &fixtures,
            &e2e_config,
            &config,
            &type_defs,
            &[],
            &[],
            &[],
            false,
        );

        let import_line = out
            .lines()
            .find(|line| line.starts_with("from sample_pkg import"))
            .unwrap_or_else(|| panic!("no `from sample_pkg import ...` line in output, got:\n{out}"));
        assert!(
            import_line.contains("NestedConfig"),
            "the nested config class must be imported, got: {import_line:?}"
        );
        assert!(
            out.contains("NestedConfig(value="),
            "the nested config class must be constructed, got:\n{out}"
        );
    }

    /// Depth counterpart: a field nested two levels deep (`ExtractionConfig` -> `NestedConfig` ->
    /// `DeeperConfig`) must import every level, not just the immediate child.
    #[test]
    fn render_test_file_imports_nested_config_class_at_depth() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("NestedConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let middle = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "inner".to_string(),
                ty: TypeRef::Named("DeeperConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let deepest = TypeDef {
            name: "DeeperConfig".to_string(),
            rust_path: "demo::DeeperConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, middle, deepest];

        let e2e_config = e2e_config_with_options_type("ExtractionConfig");
        let config = crate::core::config::ResolvedCrateConfig::default();
        let fixture = fixture_with_input(
            "nested_depth",
            serde_json::json!({"config": {"nested": {"inner": {"value": "x"}}}}),
        );
        let fixtures: Vec<&Fixture> = vec![&fixture];

        let out = super::super::render_test_file(
            "smoke",
            &fixtures,
            &e2e_config,
            &config,
            &type_defs,
            &[],
            &[],
            &[],
            false,
        );

        let import_line = out
            .lines()
            .find(|line| line.starts_with("from sample_pkg import"))
            .unwrap_or_else(|| panic!("no `from sample_pkg import ...` line in output, got:\n{out}"));
        assert!(
            import_line.contains("NestedConfig") && import_line.contains("DeeperConfig"),
            "both nested levels must be imported, got: {import_line:?}"
        );
        assert!(
            out.contains("NestedConfig(inner=DeeperConfig(value="),
            "both levels must be constructed, got:\n{out}"
        );
    }

    /// Regression test for the shipped defect: an enum-typed field nested inside a nested config
    /// object (e.g. `PreprocessingOptions.preset: PreprocessingPreset` inside `ConversionOptions`,
    /// html-to-markdown's `test_options.py`) is rendered correctly by `render_kwarg_field_value`
    /// (which calls `PreprocessingPreset("Aggressive")`) but, before this fix, was never recorded
    /// into `used_config_types` -- only the nested *struct* class (`PreprocessingOptions`) was.
    /// The import line must carry both names the body actually references.
    #[test]
    fn render_test_file_imports_enum_type_nested_inside_a_nested_config_object() {
        let outer = TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "demo::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "preprocessing".to_string(),
                ty: TypeRef::Named("PreprocessingOptions".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "PreprocessingOptions".to_string(),
            rust_path: "demo::PreprocessingOptions".to_string(),
            fields: vec![FieldDef {
                name: "preset".to_string(),
                ty: TypeRef::Named("PreprocessingPreset".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];
        let enums = vec![crate::core::ir::EnumDef {
            name: "PreprocessingPreset".to_string(),
            rust_path: "demo::PreprocessingPreset".to_string(),
            ..Default::default()
        }];

        let e2e_config = e2e_config_with_options_type("ConversionOptions");
        let config = crate::core::config::ResolvedCrateConfig::default();
        let fixture = fixture_with_input(
            "preprocessing_aggressive",
            serde_json::json!({"config": {"preprocessing": {"preset": "Aggressive"}}}),
        );
        let fixtures: Vec<&Fixture> = vec![&fixture];

        let out = super::super::render_test_file(
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

        let import_line = out
            .lines()
            .find(|line| line.starts_with("from sample_pkg import"))
            .unwrap_or_else(|| panic!("no `from sample_pkg import ...` line in output, got:\n{out}"));
        assert!(
            import_line.contains("PreprocessingPreset"),
            "the nested enum type must be imported alongside its containing class, got: {import_line:?}"
        );
        assert!(
            out.contains("PreprocessingPreset(\"Aggressive\")"),
            "the nested enum field must be constructed, got:\n{out}"
        );
    }

    /// Map-field counterpart, proving the new `TypeRef::Map` coverage (`typed_values.rs`) is
    /// also visible through the import half, not just the constructor half.
    #[test]
    fn render_test_file_imports_nested_config_class_for_map_field() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "profiles".to_string(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("NestedConfig".to_string())),
                ),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let e2e_config = e2e_config_with_options_type("ExtractionConfig");
        let config = crate::core::config::ResolvedCrateConfig::default();
        let fixture = fixture_with_input(
            "nested_map",
            serde_json::json!({"config": {"profiles": {"first": {"value": "x"}}}}),
        );
        let fixtures: Vec<&Fixture> = vec![&fixture];

        let out = super::super::render_test_file(
            "smoke",
            &fixtures,
            &e2e_config,
            &config,
            &type_defs,
            &[],
            &[],
            &[],
            false,
        );

        let import_line = out
            .lines()
            .find(|line| line.starts_with("from sample_pkg import"))
            .unwrap_or_else(|| panic!("no `from sample_pkg import ...` line in output, got:\n{out}"));
        assert!(
            import_line.contains("NestedConfig"),
            "the map value's class must be imported, got: {import_line:?}"
        );
        assert!(
            out.contains(r#"{"first": NestedConfig(value="#),
            "the map value must be constructed, got:\n{out}"
        );
    }

    /// End-to-end control for the `$mock_url` typed-lowering fix (`typed_values.rs`): a nested
    /// config field carrying a `$mock_url` placeholder must still construct through its own
    /// class in the actually-rendered test file, not fall back to a raw dict just because the
    /// value needs runtime placeholder substitution.
    #[test]
    fn render_test_file_constructs_nested_config_class_for_mock_url_fixture() {
        let outer = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("NestedConfig".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inner = TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "url".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let type_defs = vec![outer, inner];

        let e2e_config = e2e_config_with_options_type("ExtractionConfig");
        let config = crate::core::config::ResolvedCrateConfig::default();
        let fixture = fixture_with_input(
            "nested_mock_url",
            serde_json::json!({"config": {"nested": {"url": "$mock_url/path"}}}),
        );
        let fixtures: Vec<&Fixture> = vec![&fixture];

        let out = super::super::render_test_file(
            "smoke",
            &fixtures,
            &e2e_config,
            &config,
            &type_defs,
            &[],
            &[],
            &[],
            false,
        );

        assert!(
            out.contains("NestedConfig(url="),
            "the nested struct field must still be constructed with its own class for a \
             $mock_url fixture, got:\n{out}"
        );
        assert!(
            !out.contains("**json.loads"),
            "the nested constructor must not fall back to unpacking a raw dict, got:\n{out}"
        );
    }
}
