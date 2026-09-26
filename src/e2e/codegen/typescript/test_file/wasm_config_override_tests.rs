//! Neutral-name coverage for issue #444: `[crates.e2e] wasm_config_overrides` replaced a
//! hard-coded match on a real consumer type name (`SsrfPolicy`/`deny_private`) in
//! `build_args_and_setup`'s `handle` branch with a generic, project-configured field-path
//! override -- see `apply_wasm_config_overrides` in `handle_values.rs`. `EngineConfig`/
//! `RetryPolicy` stand in for any consumer's own config/nested-struct pair here; neither that
//! name nor the field name `enabled` is special-cased anywhere in the generator.
//!
//! Split out as its own file (mirroring `wasm_handle_config_transitive_import_tests.rs`, itself
//! split out of `tests.rs` for the same reason) rather than added to `args.rs`, a remediation
//! target under the `file-modularization` rule that must not grow. ~keep

use super::*;

fn engine_fixture() -> Fixture {
    Fixture {
        id: "engine".to_string(),
        description: "Engine".to_string(),
        ..Fixture::default()
    }
}

fn engine_config_type_defs() -> [TypeDef; 2] {
    [
        TypeDef {
            name: "EngineConfig".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "retry_policy".into(),
                ty: TypeRef::Named("RetryPolicy".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "RetryPolicy".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "enabled".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn engine_handle_arg() -> ArgMapping {
    ArgMapping {
        name: "engine".into(),
        field: "engine".into(),
        arg_type: "handle".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// The mechanism fires for a neutrally-named config type once a project opts in by naming the
/// field path in `[crates.e2e] wasm_config_overrides`. This is the direct #444 regression: the
/// old code only fired for a type literally named (or suffixed) `SsrfPolicy` with a
/// `deny_private` field -- a hard-coded consumer type name the generator must not carry.
#[test]
fn wasm_config_override_forces_configured_field_when_opted_in() {
    let type_defs = engine_config_type_defs();
    let fixture = engine_fixture();
    let input = serde_json::json!({ "engine": {} });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig {
        e2e: Some(E2eConfig {
            wasm_config_overrides: [("retry_policy.enabled".to_string(), toml::Value::Boolean(false))]
                .into_iter()
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        Some("WasmEngineConfig"),
        &type_defs,
        &[],
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );
    let setup = setup_lines.join("\n");

    assert!(
        setup.contains(
            "engineConfig.retryPolicy = (() => { const _u0 = WasmRetryPolicy.default(); _u0.enabled = false; \
             return _u0; })();"
        ),
        "a configured override path must construct the nested class and force the named field through its \
         setter: {setup}"
    );
}

/// Negative control: the identical fixture and IR, but with no `wasm_config_overrides` entry
/// configured. Nothing about `EngineConfig`/`RetryPolicy`'s names or shape should make the
/// generator invent an override on its own -- an empty engine config must render as a plain
/// `createEngine(null)` call, exactly as it would for any handle with no configured override.
#[test]
fn wasm_config_override_does_not_fire_without_opt_in() {
    let type_defs = engine_config_type_defs();
    let fixture = engine_fixture();
    let input = serde_json::json!({ "engine": {} });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        Some("WasmEngineConfig"),
        &type_defs,
        &[],
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        setup_lines,
        vec!["const engine = createEngine(null);".to_string()],
        "with no configured override, an empty handle config must take the plain null-config path, not \
         synthesize a class instance: {setup_lines:?}"
    );
}

/// The override must fold onto whatever value the fixture already set at that path (single
/// setter write), not append a second, later assignment -- the exact #431 double-emission shape,
/// reproduced here with neutral names and an explicit opt-in instead of the name-sniffed SSRF
/// case that fix originally shipped for.
#[test]
fn wasm_config_override_merges_onto_the_fixtures_own_value_for_a_single_emission() {
    let type_defs = engine_config_type_defs();
    let fixture = engine_fixture();
    let input = serde_json::json!({ "engine": { "retry_policy": { "enabled": true } } });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig {
        e2e: Some(E2eConfig {
            wasm_config_overrides: [("retry_policy.enabled".to_string(), toml::Value::Boolean(false))]
                .into_iter()
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        Some("WasmEngineConfig"),
        &type_defs,
        &[],
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );
    let setup = setup_lines.join("\n");

    assert!(
        setup.contains(
            "engineConfig.retryPolicy = (() => { const _u0 = WasmRetryPolicy.default(); _u0.enabled = false; \
             return _u0; })();"
        ),
        "the override must force the configured field even though the fixture asked for the opposite value: \
         {setup}"
    );
    assert_eq!(
        setup.matches("engineConfig.retryPolicy =").count(),
        1,
        "exactly one assignment must survive -- a second, later assignment on top of the fixture's own \
         setter is the #431 defect this mechanism must not reintroduce: {setup}"
    );
}
