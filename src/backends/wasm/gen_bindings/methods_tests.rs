use super::*;
use crate::core::ir::{ParamDef, ReceiverKind};
use std::collections::HashMap;

fn mapper() -> WasmMapper {
    WasmMapper::new(HashMap::new(), "Wasm".to_string())
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

/// A static `default()` method, as synthesized from a type's own custom `impl Default` (not
/// `#[derive(Default)]`) — the shape that put a bare-name path into `core_call` in the first
/// place: `gen_struct_methods` only falls back to the synthetic `<Wasm{T} as Default>::default()`
/// wrapper when `typ.methods` has no `default` entry, so a real custom `impl Default` routes
/// through this generic static-method path instead.
fn default_method(return_type_name: &str) -> MethodDef {
    MethodDef {
        name: "default".to_string(),
        is_static: true,
        return_type: TypeRef::Named(return_type_name.to_string()),
        receiver: None,
        ..Default::default()
    }
}

/// Regression coverage: a type whose `rust_path` nests it under a private module (mirroring
/// a real consumer shape where the type is never re-exported at the crate root)
/// must have its static core call built from that full module path, not from
/// `{core_import}::{typ.name}`. Before the fix, `gen_method`'s static branch built the call as
/// `format!("{core_import}::{type_name}::{method}(...)")`, ignoring `typ.rust_path` entirely —
/// for `RenderOptions` nested under `core::config::render`, that produced
/// `sample_core::RenderOptions::default()`, which rustc rejects with
/// "cannot find `RenderOptions` in `sample_core`" even though the feature gating the type is on.
#[test]
fn gen_method_static_default_uses_full_rust_path_for_nested_type() {
    let typ = TypeDef {
        name: "RenderOptions".to_string(),
        rust_path: "sample_core::config::render::RenderOptions".to_string(),
        ..Default::default()
    };
    let method = default_method("RenderOptions");

    let out = gen_method(
        &method,
        &mapper(),
        "RenderOptions",
        "sample_core",
        &AHashSet::default(),
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[],
    );

    assert!(
        out.contains("sample_core::config::render::RenderOptions::default()"),
        "static default() must call the type's real module path: {out}"
    );
    assert!(
        !out.contains("sample_core::RenderOptions::default()"),
        "static default() must not assume the type is re-exported at the crate root: {out}"
    );
}

/// Negative control: when the type genuinely does live at the crate root — `rust_path` has no
/// `::` beyond what `core_type_path` treats as bare — the call still resolves to
/// `{core_import}::{name}`. This proves the fix is a real path lookup, not a blanket rewrite
/// that always nests the call under some fixed module.
#[test]
fn gen_method_static_default_uses_bare_path_for_root_type() {
    let typ = TypeDef {
        name: "PlainOptions".to_string(),
        rust_path: "PlainOptions".to_string(),
        ..Default::default()
    };
    let method = default_method("PlainOptions");

    let out = gen_method(
        &method,
        &mapper(),
        "PlainOptions",
        "sample_core",
        &AHashSet::default(),
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[],
    );

    assert!(
        out.contains("sample_core::PlainOptions::default()"),
        "a crate-root type must still resolve to `{{core_import}}::{{name}}`: {out}"
    );
}

/// Same defect, different call shape: a non-static instance method on a nested type used
/// `format!("{core_import}::{type_name}::from(self.clone())...")`. Cover it too, since it shares
/// the same bare-name assumption and the same `qualified_type_path` fix.
#[test]
fn gen_method_instance_delegate_uses_full_rust_path_for_nested_type() {
    let typ = TypeDef {
        name: "RenderOptions".to_string(),
        rust_path: "sample_core::config::render::RenderOptions".to_string(),
        ..Default::default()
    };
    let method = MethodDef {
        name: "is_sane".to_string(),
        is_static: false,
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };

    let out = gen_method(
        &method,
        &mapper(),
        "RenderOptions",
        "sample_core",
        &AHashSet::default(),
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[],
    );

    assert!(
        out.contains("sample_core::config::render::RenderOptions::from(self.clone())"),
        "instance delegation must call the type's real module path: {out}"
    );
    assert!(
        !out.contains("sample_core::RenderOptions::from(self.clone())"),
        "instance delegation must not assume the type is re-exported at the crate root: {out}"
    );
}

/// `core_crate_override` makes the binding crate depend on the override crate *only*, while
/// the IR keeps every type's original source-crate prefix. `source_crate_remaps` is what
/// bridges the two, and this emitter was the one place that reached for the unremapped path:
/// every delegating method and every `Default` body named a crate the generated manifest does
/// not list, so the crate would not compile at all (E0433, once per site). ~keep
#[test]
fn gen_method_rewrites_the_source_crate_prefix_when_a_remap_is_configured() {
    let typ = TypeDef {
        name: "CorsConfig".to_string(),
        rust_path: "sample_core::CorsConfig".to_string(),
        ..Default::default()
    };
    let method = MethodDef {
        name: "is_sane".to_string(),
        is_static: false,
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };

    let out = gen_method(
        &method,
        &mapper(),
        "CorsConfig",
        "sample_http",
        &AHashSet::default(),
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[("sample_core", "sample_http")],
    );

    assert!(
        out.contains("sample_http::CorsConfig::from(self.clone())"),
        "the delegation must name the override crate the binding actually depends on: {out}"
    );
    assert!(
        !out.contains("sample_core::"),
        "no reference to the un-depended-on source crate may survive: {out}"
    );
}

/// Same defect as `functions::tests::sync_opaque_handle_param_is_taken_by_reference`, but for the
/// method-parameter path: wasm-bindgen's glue for a by-value exported struct calls
/// `arg.__destroy_into_raw()`, nulling the JS object's `__wbg_ptr`, so a handle passed by value is
/// dead after one call. Method params were built without consulting `opaque_types` at all, so
/// every opaque-handle method param leaked into the signature by value. ~keep
#[test]
fn sync_method_opaque_handle_param_is_taken_by_reference() {
    let typ = TypeDef {
        name: "Session".to_string(),
        rust_path: "sample_fixture::Session".to_string(),
        ..Default::default()
    };
    let method = MethodDef {
        name: "attach".to_string(),
        is_static: true,
        params: vec![param("engine", TypeRef::Named("CrawlEngineHandle".to_string()))],
        return_type: TypeRef::Unit,
        ..Default::default()
    };
    let opaque: AHashSet<String> = ["CrawlEngineHandle".to_string()].into_iter().collect();

    let out = gen_method(
        &method,
        &mapper(),
        "Session",
        "sample_fixture",
        &opaque,
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[],
    );

    assert!(
        out.contains("engine: &WasmCrawlEngineHandle"),
        "opaque handle method param must be by reference so repeated calls do not hit a null pointer:\n{out}"
    );
    assert!(
        !out.contains("engine: WasmCrawlEngineHandle"),
        "by-value opaque handle method param leaks into the signature:\n{out}"
    );
}

/// Same contract for the async named-param reshaping path: the fall-through `_` arm handled every
/// param that was not a non-opaque `Named` type, including opaque handles, but built the type the
/// same unreferenced way as the base list. Requires a sibling non-opaque `Named` param to make
/// `has_named_params` true and route through the reshaping branch at all. ~keep
#[test]
fn async_method_opaque_handle_param_is_taken_by_reference() {
    let typ = TypeDef {
        name: "Session".to_string(),
        rust_path: "sample_fixture::Session".to_string(),
        ..Default::default()
    };
    let method = MethodDef {
        name: "run".to_string(),
        is_async: true,
        receiver: Some(ReceiverKind::Ref),
        params: vec![
            param("options", TypeRef::Named("RunOptions".to_string())),
            param("engine", TypeRef::Named("CrawlEngineHandle".to_string())),
        ],
        return_type: TypeRef::Unit,
        error_type: Some("CrawlError".to_string()),
        ..Default::default()
    };
    let opaque: AHashSet<String> = ["CrawlEngineHandle".to_string()].into_iter().collect();

    let out = gen_method(
        &method,
        &mapper(),
        "Session",
        "sample_fixture",
        &opaque,
        "Wasm",
        &typ,
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &[],
    );

    assert!(
        out.contains("engine: &WasmCrawlEngineHandle"),
        "opaque handle param must be by reference in the async named-reshape fall-through arm:\n{out}"
    );
    assert!(
        !out.contains("engine: WasmCrawlEngineHandle"),
        "by-value opaque handle param leaks into the async signature:\n{out}"
    );
}
