use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeRef};

/// Return the set of type names that are excluded from extendr class registration.
///
/// Mirrors the filters applied in `generate_bindings`:
///   • Trait types — never registered (no concrete class).
///   • Arc-incompatible opaque types (Rc-based, cfg-feature-gated) — skipped.
///   • Extendr-incompatible types: structs whose fields contain `Vec<T>` where T is a
///     non-opaque, non-enum named type. Extendr cannot convert these from R lists.
///
/// The returned set is used by wrapper-file generation to skip class env emission for
/// types that are not present in `extendr_module!`.
/// A trait-bridge function (register / unregister / clear) that must be wired into
/// `extendr_module!`, `extendr-wrappers.R`, and `NAMESPACE` alongside ordinary
/// free functions emitted from `api.functions`.
///
/// The IR (`ApiSurface`) does not contain these symbols because they are synthesised
/// by `gen_trait_bridge` from `TraitBridgeConfig` rather than parsed from Rust source.
/// Each entry records the name and the R-visible parameters so the R-side wrappers
/// can call `.Call("wrap__<name>", <args>, PACKAGE = ...)` with a matching signature.
pub(super) struct TraitBridgeFn {
    pub(super) name: String,
    /// Parameter names in R-visible order. R is dynamically typed so the type is
    /// erased — `register_fn` takes an R object (named list of closures), `unregister_fn`
    /// takes a plugin name, `clear_fn` takes nothing.
    pub(super) params: Vec<String>,
}

/// Collect the set of free-function names that the trait-bridge generator will emit
/// (`register_<trait>` / `unregister_<trait>` / `clear_<trait>`). Used to filter
/// `api.functions` so a free function with the same name as a trait-bridge fn is
/// not emitted twice in `lib.rs` (which would be a Rust `E0428` duplicate
/// definition). Honours `exclude_languages` so excluded bridges don't shadow real
/// free functions.
///
/// Example: `clear_text_backends` is defined both as `pub fn` in
/// `crates/sample_core/src/plugins/ocr.rs` (so it appears in `api.functions`) AND
/// synthesised by the trait-bridge generator for the `TextBackend` trait. The
/// trait-bridge form is the canonical one — it resolves to the
/// `sample_core::plugins::text_backend::clear_text_backends` path module rather than
/// the top-level alias — so emit it from the bridge generator and skip the
/// duplicate from `api.functions`.
pub(super) fn collect_trait_bridge_fn_names(config: &ResolvedCrateConfig) -> ahash::AHashSet<String> {
    let mut names = ahash::AHashSet::new();
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
            continue;
        }
        if let Some(name) = bridge_cfg.register_fn.as_deref() {
            names.insert(name.to_string());
        }
        if let Some(name) = bridge_cfg.unregister_fn.as_deref() {
            names.insert(name.to_string());
        }
        if let Some(name) = bridge_cfg.clear_fn.as_deref() {
            names.insert(name.to_string());
        }
    }
    names
}

/// Collect every trait-bridge register / unregister / clear function that the
/// extendr backend will emit for this crate, honouring `exclude_languages`.
///
/// The order matches `gen_trait_bridge` so the resulting extendr_module! entries
/// line up with the `#[extendr]` items in `lib.rs`.
pub(super) fn collect_trait_bridge_functions(config: &ResolvedCrateConfig) -> Vec<TraitBridgeFn> {
    let mut out = Vec::new();
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
            continue;
        }
        if let Some(name) = bridge_cfg.register_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: vec!["r_backend".to_string()],
            });
        }
        if let Some(name) = bridge_cfg.unregister_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: vec!["name".to_string()],
            });
        }
        if let Some(name) = bridge_cfg.clear_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: Vec::new(),
            });
        }
    }
    out
}

fn collect_bridge_handle_aliases(bridges: &[TraitBridgeConfig]) -> ahash::AHashSet<String> {
    bridges.iter().filter_map(|bridge| bridge.type_alias.clone()).collect()
}

pub(super) fn collect_excluded_class_types(api: &ApiSurface, bridges: &[TraitBridgeConfig]) -> ahash::AHashSet<String> {
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let bridge_handle_aliases = collect_bridge_handle_aliases(bridges);
    let arc_incompatible: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && bridge_handle_aliases.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    let is_struct_like = |n: &str| -> bool { !opaque_types.contains(n) && !arc_incompatible.contains(n) };
    let is_native_incompatible = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if is_struct_like(n) => true,
                TypeRef::Vec(_) => true,
                _ => false,
            },
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Vec(inner2) => match inner2.as_ref() {
                    TypeRef::Named(n) if is_struct_like(n) => true,
                    TypeRef::Vec(_) => true,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        }
    };

    let mut excluded: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_trait)
        .map(|t| t.name.clone())
        .collect();
    for t in &arc_incompatible {
        excluded.insert(t.clone());
    }
    for t in &api.types {
        if t.is_opaque || t.is_trait {
            continue;
        }
        if t.fields.iter().any(|f| is_native_incompatible(&f.ty)) {
            excluded.insert(t.name.clone());
        }
    }
    excluded
}

/// Return true if the method should be filtered out of an emitted impl block.
///
/// Mirrors `method_references_arc_incompatible` and `method_references_enum` from
/// `generate_bindings`. Used by wrapper-file generation to skip wrapper entries for
/// methods that the Rust impl block will not contain.
pub(super) fn method_is_excluded_from_impl(
    method: &crate::core::ir::MethodDef,
    api: &ApiSurface,
    bridges: &[TraitBridgeConfig],
) -> bool {
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let enum_names: ahash::AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let bridge_handle_aliases = collect_bridge_handle_aliases(bridges);
    let arc_incompatible: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && bridge_handle_aliases.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    let references_arc_incompatible = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Named(n) => arc_incompatible.contains(n),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if arc_incompatible.contains(n)),
            _ => false,
        }
    };
    let references_enum = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Named(n) => enum_names.contains(n.as_str()),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())),
            _ => false,
        }
    };
    let param_is_owned_struct = |ty: &TypeRef| -> bool {
        let is_non_opaque_struct =
            |n: &str| !opaque_types.contains(n) && !enum_names.contains(n) && !arc_incompatible.contains(n);
        match ty {
            TypeRef::Named(n) => is_non_opaque_struct(n),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if is_non_opaque_struct(n)),
            _ => false,
        }
    };

    if references_arc_incompatible(&method.return_type)
        || method.params.iter().any(|p| references_arc_incompatible(&p.ty))
    {
        return true;
    }
    if references_enum(&method.return_type)
        || method
            .params
            .iter()
            .any(|p| references_enum(&p.ty) || param_is_owned_struct(&p.ty))
    {
        return true;
    }
    let references_map = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Map(_, _) => true,
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Map(_, _)),
            _ => false,
        }
    };
    if references_map(&method.return_type) || method.params.iter().any(|p| references_map(&p.ty)) {
        return true;
    }
    if method_return_unsupported(method) {
        return true;
    }
    if method.sanitized {
        return true;
    }
    false
}

/// Return true if a method's return type cannot be auto-converted into `Robj` by extendr.
///
/// Extendr provides no `Robj` conversion for `Option<Named>` (no `From<Option<ExternalPtr<T>>>`),
/// `Vec<Named>` (no `From<Vec<LocalStruct>>`), or `Option<Vec<_>>` (fails `ToVectorValue`). Mirror
/// of the closure of the same name in `generate_bindings`.
pub(super) fn method_return_unsupported(method: &crate::core::ir::MethodDef) -> bool {
    match &method.return_type {
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(_)),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Bytes)
        }
        _ => false,
    }
}
