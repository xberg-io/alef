use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
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
/// The `exclude_languages` spellings that name this target: the language (`"r"`) and the backend
/// (`"extendr"`). ~keep
const TARGET_SPELLINGS: [&str; 2] = ["r", "extendr"];

/// Whether `bridge` is generated for the R/extendr target.
pub(super) fn bridge_targets_extendr(bridge: &TraitBridgeConfig) -> bool {
    crate::codegen::generators::trait_bridge::bridge_targets_language(bridge, &TARGET_SPELLINGS)
}

/// The configured bridges extendr emits `#[extendr]` items for.
///
/// Every extendr site that decides whether a bridge exists — the emitted `#[extendr]` items, the
/// `extendr_module!` entries, the R wrappers and `NAMESPACE`, and
/// `ExtendrBackend::trait_bridge_registration_surface` — enumerates this. ~keep
fn active_bridges(config: &ResolvedCrateConfig) -> impl Iterator<Item = &TraitBridgeConfig> {
    config.trait_bridges_for(Language::R)
}

/// The R-visible register / unregister / clear functions the extendr trait-bridge generator
/// emits for each configured bridge.
///
/// Registration additionally requires `registry_getter` — see
/// [`crate::codegen::generators::trait_bridge::bridge_register_symbol`], which
/// `collect_trait_bridge_functions` asks too so the reported surface and the `extendr_module!`
/// entries name the same set. ~keep
pub(super) fn extendr_registration_surface(config: &ResolvedCrateConfig) -> Vec<TraitBridgeRegistrationSurface> {
    active_bridges(config)
        .filter(|bridge| bridge.register_fn.is_some() || bridge.unregister_fn.is_some() || bridge.clear_fn.is_some())
        .map(|bridge| TraitBridgeRegistrationSurface {
            trait_name: bridge.trait_name.clone(),
            register_symbol: crate::codegen::generators::trait_bridge::bridge_register_symbol(bridge)
                .map(str::to_owned),
            unregister_symbol: bridge.unregister_fn.clone(),
            clear_symbol: bridge.clear_fn.clone(),
        })
        .collect()
}

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
/// definition). Enumerates `active_bridges` and asks `bridge_register_symbol`, so a bridge that
/// emits no `#[extendr] pub fn` — one excluded for this target, or a `register_fn` without a
/// `registry_getter` — does not shadow a real free function of the same name.
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
    for bridge_cfg in active_bridges(config) {
        if let Some(name) = crate::codegen::generators::trait_bridge::bridge_register_symbol(bridge_cfg) {
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
/// extendr backend will emit for this crate.
///
/// The register entry asks `bridge_register_symbol`, the same question
/// `ExtendrBridgeGenerator::gen_registration_fn` asks before writing the `#[extendr] pub fn`, so
/// the `extendr_module!` entries line up with the items in `lib.rs`. Naming a function here that
/// no `#[extendr]` item defines is a Rust compile error, not a missing binding. ~keep
pub(super) fn collect_trait_bridge_functions(config: &ResolvedCrateConfig) -> Vec<TraitBridgeFn> {
    let mut out = Vec::new();
    for bridge_cfg in active_bridges(config) {
        if let Some(name) = crate::codegen::generators::trait_bridge::bridge_register_symbol(bridge_cfg) {
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
