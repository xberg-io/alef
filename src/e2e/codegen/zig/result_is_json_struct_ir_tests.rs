//! Regression coverage for the zig e2e generator's `result_is_json_struct` auto-detection
//! (`result_shape::ir_says_json_struct`).
//!
//! `render_test_fn` used to decide `result_is_json_struct` purely from an explicit
//! `[overrides.zig] result_is_json_struct = true` or a configured `client_factory`. The zig
//! backend (`zig_return_type` in `src/backends/zig/gen_bindings/functions.rs`) maps EVERY
//! `Named` struct return whose IR type has `has_serde` to `[]u8` (JSON) unconditionally,
//! regardless of that config. A plain top-level function returning such a struct, with no
//! override and no `client_factory`, took the typed-struct assertion path and emitted
//! `result.<field>` against a byte slice — a compile error for every field on every such call.
//!
//! These tests drive the real entry point, `render_test_file`, with no
//! `result_is_json_struct`/`client_factory` config at all — the JSON-struct routing must come
//! from the IR alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::CallIr;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_file::render_test_file;

fn response_type() -> TypeDef {
    TypeDef {
        name: "Response".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "summary".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }
}

fn opaque_tree_type() -> TypeDef {
    TypeDef {
        name: "Tree".to_string(),
        is_opaque: true,
        fields: vec![FieldDef {
            name: "summary".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }
}

fn summary_fixture() -> Fixture {
    Fixture {
        id: "summary_smoke".to_string(),
        description: "Summary smoke".to_string(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("summary".to_string()),
            value: Some(serde_json::Value::String("hi".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

/// The defect itself: a plain function returning a serde-JSON DTO, with no
/// `result_is_json_struct` override and no `client_factory`, must still route through the
/// JSON-parsing path. Before the fix, this rendered `result.summary` against the `[]u8` the
/// actual Zig backend emits — a path this test proves is no longer taken by asserting the
/// JSON-parsing shape is present and the broken direct-field shape is absent.
#[test]
fn json_dto_return_with_no_config_routes_through_json_path_via_ir() {
    let type_defs = vec![response_type()];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("Response".to_string()),
        ..FunctionDef::default()
    }];
    let ir = CallIr {
        functions: &functions,
        type_defs: &type_defs,
    };
    let fixture = summary_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".to_string();

    let rendered = render_test_file(
        "smoke",
        &[&fixture],
        &e2e,
        "process",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        ir,
        &[],
    );

    assert!(
        rendered.contains("std.json.parseFromSlice"),
        "a serde-JSON DTO return must route through the JSON path, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.summary"),
        "must not emit direct field access against the []u8 the backend actually returns, got:\n{rendered}"
    );
}

/// Misclassification guard / negative control: a function returning a genuine opaque handle
/// (no `has_serde`) must NOT be routed through the JSON path — it keeps the typed-struct
/// accessor shape, proving the IR fallback is additive and type-driven, not "always JSON now".
#[test]
fn opaque_handle_return_with_no_config_stays_on_the_typed_struct_path() {
    let type_defs = vec![opaque_tree_type()];
    let functions = vec![FunctionDef {
        name: "parse".to_string(),
        return_type: TypeRef::Named("Tree".to_string()),
        ..FunctionDef::default()
    }];
    let ir = CallIr {
        functions: &functions,
        type_defs: &type_defs,
    };
    let fixture = summary_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".to_string();

    let rendered = render_test_file(
        "smoke",
        &[&fixture],
        &e2e,
        "parse",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        ir,
        &[],
    );

    assert!(
        !rendered.contains("std.json.parseFromSlice"),
        "an opaque-handle return must not be routed through the JSON path, got:\n{rendered}"
    );
    assert!(
        rendered.contains("result.summary"),
        "an opaque-handle return keeps direct typed-struct field access, got:\n{rendered}"
    );
}

/// An explicit `result_is_json_struct` override keeps working unchanged even when the IR would
/// not have inferred it (e.g. a call the IR does not resolve) — config still wins, the IR only
/// adds.
#[test]
fn an_explicit_override_still_forces_json_struct_without_ir_support() {
    let fixture = summary_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "mystery".to_string();
    e2e.call.overrides.insert(
        "zig".to_string(),
        crate::e2e::config::CallOverride {
            result_is_json_struct: true,
            ..Default::default()
        },
    );

    let rendered = render_test_file(
        "smoke",
        &[&fixture],
        &e2e,
        "mystery",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        CallIr::default(),
        &[],
    );

    assert!(
        rendered.contains("std.json.parseFromSlice"),
        "an explicit override must still force the JSON path, got:\n{rendered}"
    );
}

#[test]
fn json_result_is_not_wrapped_based_on_function_name() {
    for name in ["process", "interact"] {
        let type_defs = vec![response_type()];
        let functions = vec![FunctionDef {
            name: name.to_string(),
            return_type: TypeRef::Named("Response".to_string()),
            ..FunctionDef::default()
        }];
        let fixture = summary_fixture();
        let mut e2e = E2eConfig::default();
        e2e.call.function = name.to_string();
        let rendered = render_test_file(
            "smoke",
            &[&fixture],
            &e2e,
            name,
            "result",
            &[],
            "sample",
            "sample",
            &ResolvedCrateConfig::default(),
            &type_defs,
            &[],
            CallIr {
                functions: &functions,
                type_defs: &type_defs,
            },
            &[],
        );
        assert!(
            rendered.contains("parseFromSlice(std.json.Value, allocator, _result_json"),
            "{name}: {rendered}"
        );
        assert!(!rendered.contains("_wrapped_json"), "{name}: {rendered}");
        assert!(
            rendered.contains("result.object.get(\"summary\")"),
            "{name}: {rendered}"
        );
    }
}
