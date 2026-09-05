//! Regression coverage for the None-fallback bare-constructor defect: `default_types` (the map
//! `emit_function_wrappers` consults to decide which params get a facade `= None` convenience
//! default) is wider than "has a usable no-argument constructor" -- it is unioned with
//! `options_dataclass_types`, so a type reachable only as a *required* nested field (no core
//! `Default` impl of its own, e.g. `ChunkPlan { seed: RunConfig }` below) is still a member. The
//! pre-fix emitter granted every `default_types` param a facade `= None` and synthesized the
//! fallback as a bare `_rust.{Type}()` call, which raises `TypeError: missing N required
//! positional arguments` for a type whose fields are all required -- exactly the shape reported
//! against the real `ChunkClassificationConfig`. Every assertion here fails against the pre-fix
//! code.

use std::collections::HashMap;

use super::gen_api_py;
use crate::core::config::DtoConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

const MODULE_NAME: &str = "_internal_bindings";
const PACKAGE_NAME: &str = "sample_pkg";

/// `RunConfig`: has a core `Default` impl -- the one case a bare `_rust.RunConfig()` fallback is
/// actually safe to synthesize.
const RUN_CONFIG: &str = "RunConfig";
/// `ChunkPlan`: no core `Default` impl of its own (its `seed` field is required, mirroring
/// `ChunkClassificationConfig`'s required `definitions`/`llm`/`batch_size`/`max_concurrency`),
/// but joins `default_types` anyway through the `options_dataclass_types` closure because `seed`
/// points at the already-`has_default` `RunConfig`.
const CHUNK_PLAN: &str = "ChunkPlan";

fn api_surface() -> ApiSurface {
    let run_config = TypeDef {
        name: RUN_CONFIG.to_owned(),
        rust_path: format!("sample_core::{RUN_CONFIG}"),
        has_default: true,
        fields: vec![FieldDef {
            name: "label".to_owned(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let chunk_plan = TypeDef {
        name: CHUNK_PLAN.to_owned(),
        rust_path: format!("sample_core::{CHUNK_PLAN}"),
        has_default: false,
        fields: vec![FieldDef {
            name: "seed".to_owned(),
            ty: TypeRef::Named(RUN_CONFIG.to_owned()),
            optional: false,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };

    // Two single-param functions, each the ONLY param of its own function, so Python's
    // no-default-after-default rule (`python_signature_params`' suffix-defaulting) cannot make
    // one function's outcome depend on the other's -- `configure` proves the `RunConfig` (real
    // `has_default`) case is untouched, `plan_chunks` proves the `ChunkPlan` (closure-only) case
    // is fixed, independently of each other.
    let configure = FunctionDef {
        name: "configure".to_owned(),
        rust_path: "sample_core::configure".to_owned(),
        params: vec![ParamDef {
            name: "config".to_owned(),
            ty: TypeRef::Named(RUN_CONFIG.to_owned()),
            optional: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    };
    let plan_chunks = FunctionDef {
        name: "plan_chunks".to_owned(),
        rust_path: "sample_core::plan_chunks".to_owned(),
        params: vec![ParamDef {
            name: "plan".to_owned(),
            ty: TypeRef::Named(CHUNK_PLAN.to_owned()),
            optional: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    };

    ApiSurface {
        types: vec![run_config, chunk_plan],
        functions: vec![configure, plan_chunks],
        ..ApiSurface::default()
    }
}

fn render(api: &ApiSurface) -> String {
    gen_api_py(
        api,
        MODULE_NAME,
        PACKAGE_NAME,
        &[],
        &DtoConfig::default(),
        &HashMap::new(),
        &std::collections::BTreeMap::new(),
        &[],
        &[],
        &ahash::AHashSet::new(),
        &crate::core::config::ResolvedCrateConfig::default(),
    )
}

/// A required field with no `Default` of its own (`ChunkPlan`) must NOT be granted a facade
/// `= None` default: the only fallback the facade could synthesize, `_rust.ChunkPlan()`, would
/// raise `TypeError` the instant a caller actually relies on the default.
#[test]
fn closure_only_param_is_not_granted_a_facade_default() {
    let api_py = render(&api_surface());
    assert!(
        api_py.contains("def plan_chunks(plan: ChunkPlan)"),
        "a `ChunkPlan` param must render as required (no `| None = None`), matching the native \
         constructor's real required fields:\n{api_py}"
    );
}

/// No bare `_rust.ChunkPlan()` fallback -- ternary or `if x is None:` guard -- may appear
/// anywhere in the generated facade: every variant fabricates an instance the native
/// constructor's required `seed` field would reject.
#[test]
fn closure_only_param_never_calls_the_bare_native_constructor() {
    let api_py = render(&api_surface());
    assert!(
        !api_py.contains("_rust.ChunkPlan()"),
        "no code path may call the bare, argument-less native constructor for a type with \
         required fields:\n{api_py}"
    );
}

/// The conversion still runs unconditionally -- `plan` is required, so there is nothing to
/// guard against `None` for.
#[test]
fn closure_only_param_converts_unconditionally() {
    let api_py = render(&api_surface());
    assert!(
        api_py.contains("_rust_plan = _to_rust_chunk_plan(plan)\n"),
        "the required param must convert straight through with no ternary/None-guard:\n{api_py}"
    );
}

/// CONTROL: a type that genuinely has an all-defaults (`has_default`) constructor must keep the
/// exact pre-fix behaviour -- a facade `= None` default backed by a real bare-constructor
/// fallback. This is the case the emitter's fallback mechanism exists for.
#[test]
fn has_default_param_keeps_its_facade_default_and_bare_fallback() {
    let api_py = render(&api_surface());
    assert!(
        api_py.contains("def configure(config: RunConfig | None = None)"),
        "a `has_default` param must keep its facade `= None` convenience default:\n{api_py}"
    );
    assert!(
        api_py.contains("_rust_config = _to_rust_run_config(config) if config is not None else _rust.RunConfig()"),
        "a `has_default` param must keep the bare-constructor fallback -- it is safe here \
         because `RunConfig` has no required fields:\n{api_py}"
    );
}
