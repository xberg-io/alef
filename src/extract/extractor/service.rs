//! Service extraction pass — builds [`ServiceDef`] and [`HandlerContractDef`] entries
//! from the already-extracted [`ApiSurface`] using the per-crate [`ResolvedCrateConfig`].
//!
//! This pass runs **after** all source files have been processed and all
//! post-processing steps (trait-source resolution, newtype resolution,
//! disambiguation) are complete.  It reads [`ServiceConfig`] entries from the
//! resolved config and partitions owner-type methods into constructor /
//! configurators / registrations / entrypoints, then builds the IR structs.
//!
//! The pass also marks the owner type and every referenced contract trait as
//! `binding_excluded` (with a reason) so the generic struct/trait codegen does
//! not emit a second, conflicting binding for them.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::service::{HandlerContractConfig, RegistrationVariantSpec, ServiceConfig};
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef,
    RegistrationVariant, RegistrationVariantOverride, RegistrationVariantStyle, ServiceDef, TypeRef,
    WrapperConstructorArg, WrapperConstructorCall,
};

/// Run the service extraction pass in-place on `surface`.
///
/// For each `[[crates.services]]` entry, locate the owner `TypeDef`, partition
/// its methods, and push a `ServiceDef` onto `surface.services`.
///
/// For each `[[crates.handler_contracts]]` entry, locate the trait `TypeDef` and
/// build a `HandlerContractDef`, pushing it onto `surface.handler_contracts`.
///
/// Both owner types and contract traits are marked `binding_excluded` so they
/// are not also emitted as plain structs/traits.
///
/// Returns a list of service extraction errors (e.g. referenced method not found).
/// Callers that perform generation must treat these as fatal.
pub(crate) fn extract_services(surface: &mut ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    // Extract the new IR sections that don't require surface-level lookups first.
    extract_lifecycle_hooks(surface, config);
    extract_websocket_routes(surface, config);
    extract_sse_routes(surface, config);
    extract_error_types(surface, config);

    if config.services.is_empty() && config.handler_contracts.is_empty() {
        return vec![];
    }

    let mut warnings = Vec::new();

    // Recover any service method the main extraction skipped — registration
    // methods generic over the callback bound (e.g. `fn route<H: IntoHandler>`)
    // and a `new`-returning-`Self` constructor (treated as a field-constructed
    // default). Those generic-extraction skips are intentional for FFI safety,
    // but methods named explicitly in `[[crates.services]]` are bridged via the
    // service codegen and must be recovered. They are re-parsed from the
    // configured sources and injected into the owner type's method list; the
    // owner is later marked `binding_excluded`, so they never reach the generic
    // struct/trait codegen.
    warnings.extend(recover_service_methods(surface, config));

    // Build handler contracts first so we can reference them from service defs.
    for hc_cfg in &config.handler_contracts {
        match build_handler_contract(surface, hc_cfg) {
            Ok(hc_def) => {
                surface.handler_contracts.push(hc_def);
                // Mark the trait as binding-excluded so generic trait codegen skips it.
                mark_type_binding_excluded(
                    surface,
                    &hc_cfg.trait_name,
                    "managed by handler_contracts service extraction",
                );
            }
            Err(msg) => warnings.push(msg),
        }
    }

    for svc_cfg in &config.services {
        match build_service_def(surface, svc_cfg) {
            Ok(svc_def) => {
                surface.services.push(svc_def);
                // Mark the owner type as binding-excluded.
                mark_type_binding_excluded(surface, &svc_cfg.owner_type, "managed by services service extraction");
            }
            Err(msg) => warnings.push(msg),
        }
    }

    mark_variant_wrapper_types(surface);

    warnings
}

/// After every service is built, walk each registration variant's
/// [`WrapperConstructorCall`] and flip
/// [`TypeDef::is_variant_wrapper`](crate::core::ir::TypeDef::is_variant_wrapper)
/// on every type that appears as a wrapper. Backends consult this flag to opt
/// the type's static constructor into host-language constructor emission so
/// variant call sites like `RouteBuilder(method, path)` resolve to a real
/// instance instead of a "cannot create instances" runtime error.
fn mark_variant_wrapper_types(surface: &mut ApiSurface) {
    let mut wrapper_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for svc in &surface.services {
        for reg in &svc.registrations {
            for variant in &reg.variants {
                if let Some(call) = &variant.wrapper_call {
                    wrapper_names.insert(call.wrapper_type_name.clone());
                }
            }
        }
    }
    if wrapper_names.is_empty() {
        return;
    }
    for t in &mut surface.types {
        if wrapper_names.contains(&t.name) {
            t.is_variant_wrapper = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Re-parse the configured Rust sources to recover any service method that the
/// main extraction pass dropped. Two generic-extraction heuristics commonly drop
/// methods a service relies on: registration methods are skipped because they are
/// generic over the callback bound (e.g. `fn route<H: IntoHandler>`), and a
/// constructor named `new` returning `Self` is skipped (treated as a
/// field-constructed default). Every method named by the service config —
/// constructor, configurators, registrations, and entrypoints — is recovered when
/// absent from the owner type's already-extracted methods. Recovered methods are
/// injected into the owner `TypeDef`.
fn recover_service_methods(surface: &mut ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let mut errors = Vec::new();
    // (owner_type, method_name) pairs configured but missing from the surface.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for svc in &config.services {
        let owner_methods: Option<Vec<String>> = surface
            .types
            .iter()
            .find(|t| t.name == svc.owner_type && !t.is_trait)
            .map(|t| t.methods.iter().map(|m| m.name.clone()).collect());

        // Every method the service config references on the owner.
        let mut names: Vec<String> = vec![svc.constructor.clone().unwrap_or_else(|| "new".to_owned())];
        names.extend(svc.configurators.iter().cloned());
        names.extend(svc.registrations.iter().map(|r| r.method.clone()));
        names.extend(svc.entrypoints.iter().map(|e| e.method.clone()));

        for name in names {
            let present = owner_methods.as_ref().is_some_and(|ms| ms.contains(&name));
            if !present {
                wanted.push((svc.owner_type.clone(), name));
            }
        }
    }
    if wanted.is_empty() {
        return errors;
    }

    // Candidate source paths (mirror the pipeline's source grouping).
    let mut sources: Vec<&std::path::Path> = Vec::new();
    if config.source_crates.is_empty() {
        sources.extend(config.sources.iter().map(std::path::PathBuf::as_path));
    } else {
        for sc in &config.source_crates {
            sources.extend(sc.sources.iter().map(std::path::PathBuf::as_path));
        }
    }

    let aliases = ahash::AHashSet::new();
    for src in sources {
        let content = match std::fs::read_to_string(src) {
            Ok(content) => content,
            Err(err) => {
                errors.push(format!(
                    "service recovery: failed to read configured source `{}`: {err}",
                    src.display()
                ));
                continue;
            }
        };
        let file = match syn::parse_file(&content) {
            Ok(file) => file,
            Err(err) => {
                errors.push(format!(
                    "service recovery: failed to parse configured source `{}`: {err}",
                    src.display()
                ));
                continue;
            }
        };
        recover_from_items(&file.items, &config.name, &aliases, &wanted, surface);
    }

    for (owner, method) in wanted {
        let recovered = surface
            .types
            .iter()
            .find(|typ| typ.name == owner && !typ.is_trait)
            .is_some_and(|typ| typ.methods.iter().any(|candidate| candidate.name == method));
        if !recovered {
            errors.push(format!(
                "service `{owner}`: configured method `{method}` could not be recovered from configured sources"
            ));
        }
    }

    errors
}

/// Walk parsed items (recursing into inline modules) for inherent impl blocks on a
/// wanted owner type, extracting any wanted registration method via the shared
/// [`super::functions::extract_method`] and injecting it into the owner `TypeDef`.
fn recover_from_items(
    items: &[syn::Item],
    crate_name: &str,
    aliases: &ahash::AHashSet<String>,
    wanted: &[(String, String)],
    surface: &mut ApiSurface,
) {
    for item in items {
        match item {
            syn::Item::Impl(item_impl) if item_impl.trait_.is_none() => {
                let Some(owner) = (match &*item_impl.self_ty {
                    syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                }) else {
                    continue;
                };
                for impl_item in &item_impl.items {
                    let syn::ImplItem::Fn(method) = impl_item else {
                        continue;
                    };
                    if !super::helpers::is_pub(&method.vis) {
                        continue;
                    }
                    let method_name = method.sig.ident.to_string();
                    if !wanted.iter().any(|(o, m)| *o == owner && *m == method_name) {
                        continue;
                    }
                    let Some(owner_def) = surface.types.iter_mut().find(|t| t.name == owner && !t.is_trait) else {
                        continue;
                    };
                    if owner_def.methods.iter().any(|m| m.name == method_name) {
                        continue;
                    }
                    let recovered = super::functions::extract_method(method, crate_name, &owner, None, aliases);
                    owner_def.methods.push(recovered);
                }
            }
            syn::Item::Mod(item_mod) => {
                if let Some((_, sub_items)) = &item_mod.content {
                    recover_from_items(sub_items, crate_name, aliases, wanted, surface);
                }
            }
            _ => {}
        }
    }
}

fn mark_type_binding_excluded(surface: &mut ApiSurface, type_name: &str, reason: &str) {
    for t in &mut surface.types {
        if t.name == type_name {
            t.binding_excluded = true;
            t.binding_exclusion_reason = Some(reason.to_string());
            break;
        }
    }
}

fn find_method<'a>(methods: &'a [MethodDef], name: &str) -> Option<&'a MethodDef> {
    methods.iter().find(|m| m.name == name)
}

fn build_handler_contract(surface: &ApiSurface, cfg: &HandlerContractConfig) -> Result<HandlerContractDef, String> {
    // Locate the trait TypeDef in the surface.
    let trait_def = surface
        .types
        .iter()
        .find(|t| t.name == cfg.trait_name && t.is_trait)
        .ok_or_else(|| {
            format!(
                "handler_contract `{}`: trait not found in extracted surface; \
                 check that the trait is pub and the source file is listed",
                cfg.trait_name
            )
        })?;

    let dispatch = find_method(&trait_def.methods, &cfg.dispatch_method)
        .ok_or_else(|| {
            format!(
                "handler_contract `{}`: dispatch method `{}` not found on trait",
                cfg.trait_name, cfg.dispatch_method
            )
        })?
        .clone();

    let optional_methods: Vec<MethodDef> = cfg
        .optional_overrides
        .iter()
        .filter_map(|name| find_method(&trait_def.methods, name).cloned())
        .collect();

    Ok(HandlerContractDef {
        trait_name: cfg.trait_name.clone(),
        rust_path: trait_def.rust_path.clone(),
        dispatch,
        optional_methods,
        wire_request_type: cfg.wire_request_type.clone(),
        wire_response_type: cfg.wire_response_type.clone(),
        dispatch_extra_params: cfg.dispatch_extra_params.clone(),
        wire_param_name: cfg.wire_param_name.clone(),
        dispatch_return_type: cfg.dispatch_return_type.clone(),
        response_adapter: cfg.response_adapter.clone(),
        doc: trait_def.doc.clone(),
    })
}

fn build_service_def(surface: &ApiSurface, cfg: &ServiceConfig) -> Result<ServiceDef, String> {
    // Locate the owner TypeDef.
    let owner_def = surface
        .types
        .iter()
        .find(|t| t.name == cfg.owner_type && !t.is_trait)
        .ok_or_else(|| {
            format!(
                "service `{}`: owner type not found in extracted surface; \
                 check that the struct is pub and the source file is listed",
                cfg.owner_type
            )
        })?;

    let methods = &owner_def.methods;
    let rust_path = owner_def.rust_path.clone();
    let doc = owner_def.doc.clone();
    let cfg_attr = owner_def.cfg.clone();

    // Constructor
    let constructor_name = cfg.constructor.as_deref().unwrap_or("new");
    let constructor = find_method(methods, constructor_name)
        .ok_or_else(|| {
            format!(
                "service `{}`: constructor method `{}` not found",
                cfg.owner_type, constructor_name
            )
        })?
        .clone();

    // Configurators
    let mut configurators = Vec::with_capacity(cfg.configurators.len());
    for configurator_name in &cfg.configurators {
        let configurator = find_method(methods, configurator_name).ok_or_else(|| {
            format!(
                "service `{}`: configurator method `{}` not found",
                cfg.owner_type, configurator_name
            )
        })?;
        configurators.push(configurator.clone());
    }

    // Registrations — built from RegistrationSpec, sourcing the method from
    // the owner's methods. Note: these methods were extracted with the
    // generic-callback-param skip bypassed (see mod.rs extraction logic).
    let mut registrations = Vec::new();
    for reg_spec in &cfg.registrations {
        let method = find_method(methods, &reg_spec.method).ok_or_else(|| {
            format!(
                "service `{}`: registration method `{}` not found; \
                     ensure callback_bound matches the generic parameter name \
                     so the method was extracted",
                cfg.owner_type, reg_spec.method
            )
        })?;

        // Split parameters: callback param vs metadata params.
        let metadata_params: Vec<_> = method
            .params
            .iter()
            .filter(|p| p.name != reg_spec.callback_param)
            .cloned()
            .collect();

        let variants = resolve_variants(surface, cfg, reg_spec, &metadata_params)?;

        registrations.push(RegistrationDef {
            method: reg_spec.method.clone(),
            callback_param: reg_spec.callback_param.clone(),
            callback_contract: reg_spec.callback_contract.clone(),
            metadata_params,
            receiver: method.receiver.clone(),
            return_type: method.return_type.clone(),
            error_type: method.error_type.clone(),
            doc: method.doc.clone(),
            variants,
            // path_param_constraints and handler_shape default to empty/BareCallable;
            // consumers populate these via alef.toml language overrides in Phase C.
            path_param_constraints: Vec::new(),
            handler_shape: Default::default(),
        });
    }

    // Entrypoints
    let mut entrypoints = Vec::new();
    for ep_spec in &cfg.entrypoints {
        let method = find_method(methods, &ep_spec.method).ok_or_else(|| {
            format!(
                "service `{}`: entrypoint method `{}` not found",
                cfg.owner_type, ep_spec.method
            )
        })?;

        let kind = parse_entrypoint_kind(&ep_spec.kind).ok_or_else(|| {
            format!(
                "service `{}`: entrypoint `{}` has unknown kind `{}`",
                cfg.owner_type, ep_spec.method, ep_spec.kind
            )
        })?;

        entrypoints.push(EntrypointDef {
            method: ep_spec.method.clone(),
            kind,
            is_async: method.is_async,
            params: method.params.clone(),
            return_type: method.return_type.clone(),
            error_type: method.error_type.clone(),
            doc: method.doc.clone(),
        });
    }

    Ok(ServiceDef {
        name: cfg.owner_type.clone(),
        rust_path,
        constructor,
        configurators,
        registrations,
        entrypoints,
        doc,
        cfg: cfg_attr,
    })
}

fn parse_entrypoint_kind(s: &str) -> Option<EntrypointKind> {
    match s {
        "run" => Some(EntrypointKind::Run),
        "finalize" => Some(EntrypointKind::Finalize),
        _ => None,
    }
}

/// Parse a `style` string from `alef.toml` into a [`RegistrationVariantStyle`].
///
fn parse_variant_style(s: Option<&str>) -> Result<RegistrationVariantStyle, String> {
    match s {
        Some("builder") => Ok(RegistrationVariantStyle::Builder),
        Some("verb_decorator") => Ok(RegistrationVariantStyle::VerbDecorator),
        Some("hybrid") | None => Ok(RegistrationVariantStyle::Hybrid),
        Some(style) => Err(format!("unknown registration variant style `{style}`")),
    }
}

/// Resolve the [`RegistrationVariantSpec`] entries declared in `alef.toml` into
/// [`RegistrationVariant`]s with pre-built call recipes.
///
/// Two resolution modes:
///
/// 1. **Wrapper mode (preferred when applicable):** if exactly one metadata
///    param's type names a [`TypeDef`](crate::core::ir::TypeDef) with a static
///    `new` constructor, the variant's `fixed` keys are matched against that
///    constructor's params; the extractor builds a
///    [`WrapperConstructorCall`] recipe that backends render at the call site,
///    and the variant's `signature_params` is the constructor's *non-fixed*
///    params plus any *other* base metadata params.
///
/// 2. **Direct mode (fallback):** `fixed` keys are matched against the base
///    [`RegistrationDef::metadata_params`] directly; `wrapper_call` is `None`
///    and `signature_params` is the non-overridden subset of base metadata.
///
/// In both modes, enum-typed pins are validated against the param type's
/// [`EnumDef`] variants and resolved to fully-qualified Rust paths
/// (`<rust_path>::<Variant>`); non-enum pins pass through verbatim.
fn resolve_variants(
    surface: &ApiSurface,
    svc_cfg: &ServiceConfig,
    reg_spec: &crate::core::config::service::RegistrationSpec,
    metadata_params: &[ParamDef],
) -> Result<Vec<RegistrationVariant>, String> {
    let wrapper = find_wrapper_constructor(surface, metadata_params);
    let mut out = Vec::with_capacity(reg_spec.variants.len());
    for v_spec in &reg_spec.variants {
        let resolved = if let Some(w) = &wrapper {
            resolve_via_wrapper(surface, svc_cfg, reg_spec, v_spec, metadata_params, w)?
        } else {
            resolve_via_direct(surface, svc_cfg, reg_spec, v_spec, metadata_params)?
        };
        out.push(resolved);
    }
    Ok(out)
}

/// Identifies the single metadata param whose type is a [`TypeDef`] carrying a
/// static `new` constructor (returns `Self`/the wrapper type), and returns the
/// pair so [`resolve_via_wrapper`] can use it.
fn find_wrapper_constructor<'a>(
    surface: &'a ApiSurface,
    metadata_params: &'a [ParamDef],
) -> Option<(&'a ParamDef, &'a crate::core::ir::TypeDef, &'a MethodDef)> {
    let mut found: Option<(&ParamDef, &crate::core::ir::TypeDef, &MethodDef)> = None;
    for param in metadata_params {
        let TypeRef::Named(type_name) = &param.ty else { continue };
        let Some(type_def) = surface.types.iter().find(|t| &t.name == type_name && !t.is_trait) else {
            continue;
        };
        let Some(ctor) = type_def
            .methods
            .iter()
            .find(|m| m.name == "new" && m.receiver.is_none() && !m.params.is_empty())
        else {
            continue;
        };
        if found.is_some() {
            // Multiple wrapper-typed metadata params with a static `new` — too ambiguous
            // to pick automatically. Fall back to direct mode (callers will get a
            // direct-mode validation error if `fixed` keys don't match base metadata).
            return None;
        }
        found = Some((param, type_def, ctor));
    }
    found
}

fn resolve_via_wrapper(
    surface: &ApiSurface,
    svc_cfg: &ServiceConfig,
    reg_spec: &crate::core::config::service::RegistrationSpec,
    v_spec: &RegistrationVariantSpec,
    metadata_params: &[ParamDef],
    wrapper: &(&ParamDef, &crate::core::ir::TypeDef, &MethodDef),
) -> Result<RegistrationVariant, String> {
    let (wrapper_param, wrapper_type, ctor) = *wrapper;
    let mut overrides = Vec::with_capacity(v_spec.fixed.len());
    let mut args = Vec::with_capacity(ctor.params.len());
    let mut free_params = Vec::new();

    for ctor_param in &ctor.params {
        if let Some(raw_value) = v_spec.fixed.get(&ctor_param.name) {
            let value_expr = match resolve_enum_override(surface, &ctor_param.ty, raw_value) {
                EnumResolution::Resolved(path) => path,
                EnumResolution::NotAnEnum => raw_value.clone(),
                EnumResolution::UnknownVariant(enum_name) => {
                    return Err(format!(
                        "service `{}` registration `{}` variant `{}`: wrapper-constructor param `{}` of enum `{}` has no variant `{}`",
                        svc_cfg.owner_type, reg_spec.method, v_spec.name, ctor_param.name, enum_name, raw_value
                    ));
                }
            };
            overrides.push(RegistrationVariantOverride {
                param_name: ctor_param.name.clone(),
                value_expr: value_expr.clone(),
            });
            args.push(WrapperConstructorArg::Fixed {
                param_name: ctor_param.name.clone(),
                value_expr,
            });
        } else {
            args.push(WrapperConstructorArg::Free {
                param: ctor_param.clone(),
            });
            free_params.push(ctor_param.clone());
        }
    }

    // Any `fixed` key that doesn't name a constructor param is an error.
    for fixed_name in v_spec.fixed.keys() {
        if !ctor.params.iter().any(|p| &p.name == fixed_name) {
            return Err(format!(
                "service `{}` registration `{}` variant `{}`: fixed param `{}` not found in wrapper `{}::{}` constructor params",
                svc_cfg.owner_type, reg_spec.method, v_spec.name, fixed_name, wrapper_type.name, ctor.name
            ));
        }
    }

    // signature_params = free constructor params + any non-wrapper base metadata params,
    // preserving declared order.
    let mut signature_params = free_params;
    for mp in metadata_params {
        if mp.name != wrapper_param.name {
            signature_params.push(mp.clone());
        }
    }

    let wrapper_type_path = if wrapper_type.rust_path.is_empty() {
        wrapper_type.name.clone()
    } else {
        wrapper_type.rust_path.clone()
    };

    Ok(RegistrationVariant {
        name: v_spec.name.clone(),
        overrides,
        wrapper_call: Some(WrapperConstructorCall {
            metadata_param: wrapper_param.name.clone(),
            wrapper_type_path,
            wrapper_type_name: wrapper_type.name.clone(),
            constructor_method: ctor.name.clone(),
            args,
        }),
        signature_params,
        doc: v_spec.doc.clone(),
        style: parse_variant_style(v_spec.style.as_deref()).map_err(|message| {
            format!(
                "service `{}` registration `{}` variant `{}`: {message}",
                svc_cfg.owner_type, reg_spec.method, v_spec.name
            )
        })?,
        language_overrides: Default::default(),
    })
}

fn resolve_via_direct(
    surface: &ApiSurface,
    svc_cfg: &ServiceConfig,
    reg_spec: &crate::core::config::service::RegistrationSpec,
    v_spec: &RegistrationVariantSpec,
    metadata_params: &[ParamDef],
) -> Result<RegistrationVariant, String> {
    let mut overrides = Vec::with_capacity(v_spec.fixed.len());
    for (param_name, raw_value) in &v_spec.fixed {
        let param = metadata_params.iter().find(|p| &p.name == param_name).ok_or_else(|| {
            format!(
                "service `{}` registration `{}` variant `{}`: fixed param `{}` not found in base metadata params",
                svc_cfg.owner_type, reg_spec.method, v_spec.name, param_name
            )
        })?;

        let value_expr = match resolve_enum_override(surface, &param.ty, raw_value) {
            EnumResolution::Resolved(path) => path,
            EnumResolution::NotAnEnum => raw_value.clone(),
            EnumResolution::UnknownVariant(enum_name) => {
                return Err(format!(
                    "service `{}` registration `{}` variant `{}`: param `{}` of enum `{}` has no variant `{}`",
                    svc_cfg.owner_type, reg_spec.method, v_spec.name, param_name, enum_name, raw_value
                ));
            }
        };

        overrides.push(RegistrationVariantOverride {
            param_name: param_name.clone(),
            value_expr,
        });
    }

    let signature_params: Vec<ParamDef> = metadata_params
        .iter()
        .filter(|p| !v_spec.fixed.contains_key(&p.name))
        .cloned()
        .collect();

    Ok(RegistrationVariant {
        name: v_spec.name.clone(),
        overrides,
        wrapper_call: None,
        signature_params,
        doc: v_spec.doc.clone(),
        style: parse_variant_style(v_spec.style.as_deref()).map_err(|message| {
            format!(
                "service `{}` registration `{}` variant `{}`: {message}",
                svc_cfg.owner_type, reg_spec.method, v_spec.name
            )
        })?,
        language_overrides: Default::default(),
    })
}

enum EnumResolution {
    /// The param resolved to an enum and the supplied value matched a variant;
    /// the resolved Rust path is the inner string.
    Resolved(String),
    /// The param's type does not name an [`EnumDef`] — pass the raw value through.
    NotAnEnum,
    /// The param resolved to an enum but the supplied value is not a known variant.
    UnknownVariant(String),
}

/// Best-effort resolution: if `ty` is a `TypeRef::Named` whose name matches an
/// `EnumDef` in `surface.enums`, attempt to match `raw_value` against the enum's
/// variant names and return the fully-qualified Rust path
/// (`<EnumDef::rust_path>::<Variant>`). Returns `NotAnEnum` for non-enum params.
fn resolve_enum_override(surface: &ApiSurface, ty: &TypeRef, raw_value: &str) -> EnumResolution {
    let name = match ty {
        TypeRef::Named(n) => n,
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => n,
            _ => return EnumResolution::NotAnEnum,
        },
        _ => return EnumResolution::NotAnEnum,
    };
    let Some(enum_def) = surface.enums.iter().find(|e| &e.name == name) else {
        return EnumResolution::NotAnEnum;
    };
    if enum_def.variants.iter().any(|v| v.name == raw_value) {
        let base = if enum_def.rust_path.is_empty() {
            enum_def.name.clone()
        } else {
            enum_def.rust_path.clone()
        };
        EnumResolution::Resolved(format!("{base}::{raw_value}"))
    } else {
        EnumResolution::UnknownVariant(enum_def.name.clone())
    }
}

// ---------------------------------------------------------------------------
// New IR section extractors — config → IR (no surface lookup required)
// ---------------------------------------------------------------------------

/// Populate [`ApiSurface::lifecycle_hooks`] from `[[crates.lifecycle_hooks]]` config.
///
/// Each config entry maps 1:1 to a [`crate::core::ir::LifecycleHookDef`].  No validation against
/// the extracted surface is performed here; backends that need to cross-check
/// against handler contracts should do so at generation time.
fn extract_lifecycle_hooks(surface: &mut ApiSurface, config: &ResolvedCrateConfig) {
    for hook_cfg in &config.lifecycle_hooks {
        surface.lifecycle_hooks.push(crate::core::ir::LifecycleHookDef {
            name: hook_cfg.name.clone(),
            callback_contract: hook_cfg.callback_contract.clone(),
            doc: hook_cfg.doc.clone().unwrap_or_default(),
            is_async: hook_cfg.is_async,
        });
    }
}

/// Populate [`ApiSurface::websocket_routes`] from `[[crates.websocket_routes]]` config.
fn extract_websocket_routes(surface: &mut ApiSurface, config: &ResolvedCrateConfig) {
    for ws_cfg in &config.websocket_routes {
        surface.websocket_routes.push(crate::core::ir::WebSocketRouteDef {
            handler_wrapper_type: ws_cfg.handler_wrapper_type.clone(),
            socket_type: ws_cfg.socket_type.clone(),
            doc: ws_cfg.doc.clone().unwrap_or_default(),
        });
    }
}

/// Populate [`ApiSurface::sse_routes`] from `[[crates.sse_routes]]` config.
fn extract_sse_routes(surface: &mut ApiSurface, config: &ResolvedCrateConfig) {
    for sse_cfg in &config.sse_routes {
        surface.sse_routes.push(crate::core::ir::SseRouteDef {
            producer_wrapper_type: sse_cfg.producer_wrapper_type.clone(),
            event_type: sse_cfg.event_type.clone(),
            doc: sse_cfg.doc.clone().unwrap_or_default(),
        });
    }
}

/// Populate [`ApiSurface::error_types`] from `[[crates.error_types]]` config.
///
/// The numeric `http_status` field in the config is converted to a typed
/// [`crate::core::ir::HttpStatus`] variant.  Numeric codes that match a named variant are
/// promoted; all others fall through to `HttpStatus::Custom(code)`.
fn extract_error_types(surface: &mut ApiSurface, config: &ResolvedCrateConfig) {
    for err_cfg in &config.error_types {
        let http_status = match err_cfg.http_status {
            400 => crate::core::ir::HttpStatus::BadRequest,
            401 => crate::core::ir::HttpStatus::Unauthorized,
            403 => crate::core::ir::HttpStatus::Forbidden,
            404 => crate::core::ir::HttpStatus::NotFound,
            409 => crate::core::ir::HttpStatus::Conflict,
            422 => crate::core::ir::HttpStatus::UnprocessableEntity,
            429 => crate::core::ir::HttpStatus::TooManyRequests,
            500 => crate::core::ir::HttpStatus::InternalServerError,
            other => crate::core::ir::HttpStatus::Custom(other),
        };
        surface.error_types.push(crate::core::ir::ErrorTypeDef {
            name: err_cfg.name.clone(),
            http_status,
            problem_details_type: err_cfg.problem_details_type.clone(),
            doc: err_cfg.doc.clone().unwrap_or_default(),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise extraction against in-memory Rust source strings
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
