use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, TypeDef, TypeRef};

pub fn bridge_handle_path(api: &ApiSurface, bridge: &TraitBridgeConfig, core_import: &str) -> String {
    let alias = bridge.type_alias.as_deref().unwrap_or(&bridge.trait_name);
    api.types
        .iter()
        .find(|typ| typ.name == alias && !typ.rust_path.is_empty())
        .map(|typ| typ.rust_path.replace('-', "_"))
        .or_else(|| api.excluded_type_paths.get(alias).map(|path| path.replace('-', "_")))
        .unwrap_or_else(|| format!("{core_import}::{alias}"))
}

/// Generate a backend visitor bridge wrapper name from configuration.
pub fn bridge_wrapper_name(prefix: &str, bridge: &TraitBridgeConfig) -> String {
    format!("{}{}Bridge", prefix, bridge.trait_name)
}

/// Return true when a type reference points at any configured bridge handle alias.
pub fn is_bridge_handle_type_ref(ty: &TypeRef, bridges: &[TraitBridgeConfig]) -> bool {
    bridges
        .iter()
        .filter_map(|bridge| bridge.type_alias.as_deref())
        .any(|alias| field_type_matches_alias(ty, alias))
}

/// Return true when a function name is emitted by trait-bridge codegen.
pub fn is_trait_bridge_managed_fn(func_name: &str, bridges: &[TraitBridgeConfig]) -> bool {
    bridges.iter().any(|bridge| {
        bridge.register_fn.as_deref() == Some(func_name)
            || bridge.unregister_fn.as_deref() == Some(func_name)
            || bridge.clear_fn.as_deref() == Some(func_name)
    })
}

/// Find the first function parameter that matches a trait bridge configuration
/// (by type alias before falling back to parameter name).
///
/// Bridges configured with `bind_via = "options_field"` are skipped — they live on a
/// struct field rather than directly as a parameter, and are returned by
/// [`find_bridge_field`] instead.
pub fn find_bridge_param<'a>(
    func: &FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for (idx, param) in func.params.iter().enumerate() {
        let named = match &param.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n.as_str())
                } else {
                    None
                }
            }
            _ => None,
        };
        for bridge in bridges {
            if bridge.bind_via != BridgeBinding::FunctionParam {
                continue;
            }
            if let Some(type_name) = named
                && bridge.type_alias.as_deref() == Some(type_name)
            {
                return Some((idx, bridge));
            }
        }
        // A generic name such as `host` may be shared by unrelated traits. Search all type
        // aliases first so configuration order cannot override an explicit type match. ~keep
        for bridge in bridges {
            if bridge.bind_via == BridgeBinding::FunctionParam
                && bridge.param_name.as_deref() == Some(param.name.as_str())
            {
                return Some((idx, bridge));
            }
        }
    }
    None
}

/// Match info for a trait bridge whose handle lives as a struct field
/// (`bind_via = "options_field"`).
#[derive(Debug, Clone)]
pub struct BridgeFieldMatch<'a> {
    /// Index of the function parameter that carries the owning struct.
    pub param_index: usize,
    /// Name of the parameter (e.g., `"options"`).
    pub param_name: String,
    /// IR type name of the parameter, with any `Option<>` wrapper unwrapped.
    pub options_type: String,
    /// True if the param is `Option<TypeName>` rather than `TypeName`.
    pub param_is_optional: bool,
    /// Name of the field on `options_type` that holds the bridge handle.
    pub field_name: String,
    /// The matching field definition (carries the field's `TypeRef`).
    pub field: &'a FieldDef,
    /// The bridge configuration that produced the match.
    pub bridge: &'a TraitBridgeConfig,
}

/// Find the first function parameter whose IR type carries a bridge field
/// (`bind_via = "options_field"`).
///
/// For each function parameter whose IR type is `Named(N)` or `Optional<Named(N)>`,
/// look up `N` in `types`. If `N` matches any bridge's `options_type`, search its
/// fields for one whose name matches the bridge's resolved options field (or whose
/// type's `Named` alias matches the bridge's `type_alias`). Returns the first match.
///
/// Bridges configured with `bind_via = "function_param"` are skipped — those go
/// through [`find_bridge_param`] instead.
pub fn find_bridge_field<'a>(
    func: &FunctionDef,
    types: &'a [TypeDef],
    bridges: &'a [TraitBridgeConfig],
) -> Option<BridgeFieldMatch<'a>> {
    fn unwrap_named(ty: &TypeRef) -> Option<(&str, bool)> {
        match ty {
            TypeRef::Named(n) => Some((n.as_str(), false)),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some((n.as_str(), true))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    for (idx, param) in func.params.iter().enumerate() {
        let Some((type_name, is_optional)) = unwrap_named(&param.ty) else {
            continue;
        };
        let Some(type_def) = types.iter().find(|t| t.name == type_name) else {
            continue;
        };
        for bridge in bridges {
            if bridge.bind_via != BridgeBinding::OptionsField {
                continue;
            }
            if bridge.options_type.as_deref() != Some(type_name) {
                continue;
            }
            let field_name = bridge.resolved_options_field();
            for field in &type_def.fields {
                let matches_name = field_name.is_some_and(|n| field.name == n);
                let matches_alias = bridge
                    .type_alias
                    .as_deref()
                    .is_some_and(|alias| field_type_matches_alias(&field.ty, alias));
                if matches_name || matches_alias {
                    return Some(BridgeFieldMatch {
                        param_index: idx,
                        param_name: param.name.clone(),
                        options_type: type_name.to_string(),
                        param_is_optional: is_optional,
                        field_name: field.name.clone(),
                        field,
                        bridge,
                    });
                }
            }
        }
    }
    None
}

/// Names of Clone opaque types represented by read-only native Arc wrappers.
pub fn native_marshalled_opaque_params(api: &ApiSurface) -> std::collections::HashSet<String> {
    // Read-only Arc wrappers can carry Clone handles without a serde round trip.
    // Mutex-backed, excluded, lifetime-bearing and trait wrappers need different conversions.
    api.types
        .iter()
        .filter(|t| {
            t.is_opaque
                && t.is_clone
                && !t.is_trait
                && !t.binding_excluded
                && !t.has_lifetime_params
                && !crate::codegen::generators::type_needs_mutex(t)
        })
        .map(|t| t.name.clone())
        .collect()
}

/// Decide whether a trait-callback parameter type should be marshalled to the host as the
/// binding's NATIVE object (constructed through the binding's Rust→host conversion) rather
/// than as a serialized string.
///
/// This is the single, backend-agnostic classification rule for trait-bridge callback params.
/// Backends consult it (typically by seeding a per-generator struct-param allowlist with
/// [`native_marshalled_struct_params`]) to choose between handing the host a native object vs.
/// the prior string representation. Keeping the rule here — not in any one backend — lets every
/// backend share the same notion of "this param is a struct the host should receive as a native
/// value".
///
/// A type qualifies only when it is a **known serde struct**:
/// - present in `api.types` (so the binding actually emits a representation for it),
/// - a struct, i.e. NOT a trait (`!is_trait`) and NOT opaque/handle (`!is_opaque`),
/// - derives serde (`has_serde`), so the binding's value conversion is well-defined,
/// - not excluded from the binding surface (`!binding_excluded`).
///
/// Enums (which live in `api.enums`, never `api.types`), opaque/handle types, excluded types,
/// and unknown `Named` types all fail this test and are left to their prior representation.
pub fn is_native_marshalled_struct(type_name: &str, api: &ApiSurface) -> bool {
    api.types
        .iter()
        .any(|t| t.name == type_name && !t.is_trait && !t.is_opaque && t.has_serde && !t.binding_excluded)
}

/// Collect the names of all trait-callback parameter types across `trait_def`'s methods that
/// qualify for native-object marshalling per [`is_native_marshalled_struct`].
///
/// The result seeds each backend generator's struct-param allowlist; generators look a param's
/// `Named` type up in that set to decide its marshalling, so the positive allowlist is computed
/// once, in shared code, and reused identically by every backend.
pub fn native_marshalled_struct_params(trait_def: &TypeDef, api: &ApiSurface) -> std::collections::HashSet<String> {
    fn leaf_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => leaf_named(inner),
            _ => None,
        }
    }

    let mut out = std::collections::HashSet::new();
    for method in &trait_def.methods {
        for param in &method.params {
            if let Some(name) = leaf_named(&param.ty)
                && is_native_marshalled_struct(name, api)
            {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Collect the names of native-marshalled structs that appear as a callback's RETURN type.
///
/// Restricted to a bare `Named` return: the native fast-path extracts the binding's native object
/// and converts it via `From<core::T>`, which is well-defined only for a direct struct return.
/// `Optional`/`Vec`/other return shapes keep the existing mapping/JSON path. Return-side
/// counterpart to [`native_marshalled_struct_params`] — a host may return the binding's native
/// result object instead of an untyped mapping, and the bridge accepts it (falling back to the
/// mapping path otherwise).
pub fn native_marshalled_struct_returns(trait_def: &TypeDef, api: &ApiSurface) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for method in &trait_def.methods {
        if let TypeRef::Named(name) = &method.return_type
            && is_native_marshalled_struct(name, api)
        {
            out.insert(name.clone());
        }
    }
    out
}

/// Look up the trait `TypeDef` a bridge wraps, by the bridge's configured `trait_name`.
///
/// Trait definitions live in `api.types` with `is_trait == true`. Backends use this to recover the
/// trait's methods (params + return types) when emitting a typed, host-implementable surface for a
/// plugin bridge — e.g. an Elixir behaviour or a Python Protocol. Returns `None` when the trait is
/// not present in `api.types` (e.g. fully excluded from the binding surface).
pub fn find_trait_def<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    api.types
        .iter()
        .find(|typ| typ.is_trait && typ.name == bridge.trait_name)
}

/// Whether `bridge` is emitted for the target that answers to `target_spellings`.
///
/// `exclude_languages` may name a backend by its language (`"node"`, `"r"`, `"elixir"`) or by the
/// backend itself (`"napi"`, `"extendr"`, `"rustler"`), so a target that answers to more than one
/// spelling is excluded when *any* of its spellings is listed. ~keep
pub fn bridge_targets_language(bridge: &TraitBridgeConfig, target_spellings: &[&str]) -> bool {
    target_spellings.iter().all(|spelling| bridge.is_active_for(spelling))
}

/// The `register_fn` for which a registration function is actually generated.
///
/// `register_fn` alone is not enough: every backend's `gen_registration_fn` returns an empty
/// string without `registry_getter` (the FFI backend panics instead), because the generated body
/// has no registry to insert into. A site that wires the name into a module manifest —
/// `extendr_module!`, a NAMESPACE file, a reported registration surface — must ask this rather
/// than read `register_fn` directly, or it names a symbol no emitter wrote. ~keep
pub fn bridge_register_symbol(bridge: &TraitBridgeConfig) -> Option<&str> {
    bridge
        .register_fn
        .as_deref()
        .filter(|_| bridge.registry_getter.is_some())
}

/// The trait `TypeDef` a bridge wraps, when the target named by `target_spellings` emits that
/// bridge at all.
///
/// Combines the two gates a bridge emitter applies: the bridge must not be excluded for the target
/// (see [`bridge_targets_language`]) and its trait must resolve in the `ApiSurface`.
///
/// Backends that emit the Rust bridge and the host-language wrapper from separate passes ask
/// through this one lookup, so the passes cannot answer the question differently and leave a
/// wrapper calling a symbol no pass generated. `Backend::trait_bridge_registration_surface` asks
/// through it too, so the reference docs describe the API that was actually emitted. ~keep
pub fn active_bridge_trait_def<'a>(
    bridge: &TraitBridgeConfig,
    api: &'a ApiSurface,
    target_spellings: &[&str],
) -> Option<&'a TypeDef> {
    // `target` is reserved syntax in the `tracing` macros, so the field carries the backend's
    // spelling under a different key. ~keep
    let bridge_target = target_spellings.first().copied().unwrap_or("");
    if !bridge_targets_language(bridge, target_spellings) {
        tracing::debug!(
            trait_name = %bridge.trait_name,
            bridge_target,
            "trait bridge skipped: exclude_languages names this target"
        );
        return None;
    }
    let trait_def = find_trait_def(bridge, api);
    if trait_def.is_none() {
        tracing::warn!(
            trait_name = %bridge.trait_name,
            bridge_target,
            "trait bridge skipped: the configured trait is absent from the API surface"
        );
    }
    trait_def
}

/// True if `field_ty` references a `Named` type whose name equals `alias`,
/// allowing for `Option<>` and `Vec<>` wrappers.
fn field_type_matches_alias(field_ty: &TypeRef, alias: &str) -> bool {
    match field_ty {
        TypeRef::Named(n) => n == alias,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_type_matches_alias(inner, alias),
        _ => false,
    }
}

/// True when a `TypeRef` references a trait type (directly or through
/// `Option`/`Vec`/`Map` wrappers). Trait-typed values cannot cross a foreign
/// bridge — the host cannot construct or return a Rust trait object — so
/// methods whose signature references a trait are excluded from
/// defaulted-method forwarding and keep the Rust default unconditionally.
pub fn type_references_trait(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(name) => {
            api.types.iter().any(|t| t.is_trait && &t.name == name) || api.excluded_trait_names.contains(name)
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_references_trait(inner, api),
        TypeRef::Map(k, v) => type_references_trait(k, api) || type_references_trait(v, api),
        _ => false,
    }
}

/// True when any param or the return type of `method` references a trait type.
pub fn method_signature_references_trait(method: &crate::core::ir::MethodDef, api: &ApiSurface) -> bool {
    type_references_trait(&method.return_type, api) || method.params.iter().any(|p| type_references_trait(&p.ty, api))
}

/// The names of the Rust-defaulted own methods of `trait_type` whose signatures a
/// foreign bridge can marshal — the standard input to a backend's
/// defaulted-method forwarding opt-in (`gen_method_presence_check`).
pub fn forwardable_defaulted_method_names(trait_type: &TypeDef, api: &ApiSurface) -> std::collections::HashSet<String> {
    trait_type
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && m.has_default_impl && !method_signature_references_trait(m, api))
        .map(|m| m.name.clone())
        .collect()
}
