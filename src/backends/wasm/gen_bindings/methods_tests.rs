use super::*;
use crate::core::ir::ReceiverKind;
use std::collections::HashMap;

fn mapper() -> WasmMapper {
    WasmMapper::new(HashMap::new(), "Wasm".to_string())
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
