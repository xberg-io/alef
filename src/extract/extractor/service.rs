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
use crate::core::config::service::{HandlerContractConfig, ServiceConfig};
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, RegistrationDef, ServiceDef,
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
/// Returns a list of non-fatal warning strings (e.g. referenced method not found).
/// Hard configuration errors were already caught at `resolve_one` time.
pub(crate) fn extract_services(surface: &mut ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
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
    recover_service_methods(surface, config);

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

    warnings
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
fn recover_service_methods(surface: &mut ApiSurface, config: &ResolvedCrateConfig) {
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
        return;
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
        let Ok(content) = std::fs::read_to_string(src) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&content) else {
            continue;
        };
        recover_from_items(&file.items, &config.name, &aliases, &wanted, surface);
    }
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
    let configurators: Vec<MethodDef> = cfg
        .configurators
        .iter()
        .filter_map(|name| find_method(methods, name).cloned())
        .collect();

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

        registrations.push(RegistrationDef {
            method: reg_spec.method.clone(),
            callback_param: reg_spec.callback_param.clone(),
            callback_contract: reg_spec.callback_contract.clone(),
            metadata_params,
            receiver: method.receiver.clone(),
            return_type: method.return_type.clone(),
            error_type: method.error_type.clone(),
            doc: method.doc.clone(),
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

// ---------------------------------------------------------------------------
// Tests — exercise extraction against in-memory Rust source strings
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::service::{EntrypointSpec, HandlerContractConfig, RegistrationSpec, ServiceConfig};
    use crate::extract::extractor;

    /// Write a temporary Rust source file and extract its surface.
    fn extract_source(src: &str) -> ApiSurface {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lib.rs");
        std::fs::write(&file_path, src).expect("write test source");
        extractor::extract(&[file_path.as_path()], "test_crate", "0.1.0", None).expect("extraction must succeed")
    }

    /// Like [`extract_source`] but keeps the temp dir alive and returns the source
    /// path, so the service pass can re-parse it to recover generic registration
    /// methods that the main extraction skipped.
    fn extract_source_persistent(src: &str) -> (tempfile::TempDir, std::path::PathBuf, ApiSurface) {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lib.rs");
        std::fs::write(&file_path, src).expect("write test source");
        let surface =
            extractor::extract(&[file_path.as_path()], "test_crate", "0.1.0", None).expect("extraction must succeed");
        (dir, file_path, surface)
    }

    /// Minimal Rust source with an owner type, a contract trait, and supporting
    /// methods that exercise every classification bucket.
    ///
    /// Note: the constructor is named `new` and returns `Self`, which the main
    /// extraction pass intentionally skips (treated as a field-constructed
    /// default).  The service pass must recover it from source so the
    /// `ServiceDef` constructor is populated.
    const SERVICE_SOURCE: &str = r#"
/// App documentation.
pub struct App {
    addr: String,
}

impl App {
    /// Create a new App.
    pub fn new() -> Self { todo!() }

    /// Set bind address (configurator).
    pub fn set_address(mut self, addr: String) -> Self { todo!() }

    /// Register a route (registration — generic param H: IntoHandler).
    pub fn add_route<H: IntoHandler>(mut self, path: String, handler: H) -> Self { todo!() }

    /// Run the service (async entrypoint).
    pub async fn run(self) -> Result<(), String> { todo!() }

    /// Consume into a router (finalize entrypoint).
    pub fn into_router(self) -> Router { todo!() }
}

/// Handler contract trait.
pub trait Handler {
    async fn call(&self, req: RequestData) -> ResponseData;
}

/// Wire request DTO.
pub struct RequestData {
    pub path: String,
}

/// Wire response DTO.
pub struct ResponseData {
    pub status: u32,
}

/// Router type (returned by finalize).
pub struct Router {}

// IntoHandler is a bound used in generic registration — not an exported binding type.
pub trait IntoHandler {}
"#;

    fn make_resolved_config_with_service() -> crate::core::config::ResolvedCrateConfig {
        crate::core::config::ResolvedCrateConfig {
            name: "test_crate".to_string(),
            services: vec![ServiceConfig {
                owner_type: "App".to_string(),
                constructor: Some("new".to_string()),
                configurators: vec!["set_address".to_string()],
                registrations: vec![RegistrationSpec {
                    method: "add_route".to_string(),
                    callback_param: "handler".to_string(),
                    callback_bound: "IntoHandler".to_string(),
                    callback_contract: "Handler".to_string(),
                }],
                entrypoints: vec![
                    EntrypointSpec {
                        method: "run".to_string(),
                        kind: "run".to_string(),
                    },
                    EntrypointSpec {
                        method: "into_router".to_string(),
                        kind: "finalize".to_string(),
                    },
                ],
                skip_languages: vec![],
            }],
            handler_contracts: vec![HandlerContractConfig {
                trait_name: "Handler".to_string(),
                dispatch_method: "call".to_string(),
                is_async: true,
                wire_request_type: Some("RequestData".to_string()),
                wire_response_type: Some("ResponseData".to_string()),
                optional_overrides: vec![],
                dispatch_extra_params: vec![],
                wire_param_name: None,
                dispatch_return_type: None,
                response_adapter: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn service_extraction_populates_service_def_and_handler_contract() {
        // `add_route<H: IntoHandler>` is dropped by the generic-method guard during
        // the main pass; the service pass recovers it by re-parsing the configured
        // sources, so the config must carry the source path.
        let (_dir, file_path, mut surface) = extract_source_persistent(SERVICE_SOURCE);
        let mut config = make_resolved_config_with_service();
        config.sources = vec![file_path];

        let warnings = extract_services(&mut surface, &config);
        assert!(warnings.is_empty(), "no warnings expected; got {warnings:?}");

        // HandlerContractDef must be populated.
        assert_eq!(
            surface.handler_contracts.len(),
            1,
            "exactly one HandlerContractDef expected"
        );
        let hc = &surface.handler_contracts[0];
        assert_eq!(hc.trait_name, "Handler");
        assert_eq!(hc.dispatch.name, "call");
        assert!(hc.dispatch.is_async, "dispatch method must be detected as async");
        assert_eq!(hc.wire_request_type.as_deref(), Some("RequestData"));
        assert_eq!(hc.wire_response_type.as_deref(), Some("ResponseData"));

        // Handler trait must be marked binding-excluded.
        let handler_type = surface.types.iter().find(|t| t.name == "Handler");
        if let Some(t) = handler_type {
            assert!(t.binding_excluded, "Handler trait must be marked binding_excluded");
        }

        // ServiceDef must be populated.
        assert_eq!(surface.services.len(), 1, "exactly one ServiceDef expected");
        let svc = &surface.services[0];
        assert_eq!(svc.name, "App");
        assert_eq!(
            svc.constructor.name, "new",
            "constructor `new` must be recovered from source"
        );
        assert_eq!(svc.configurators.len(), 1);
        assert_eq!(svc.configurators[0].name, "set_address");

        // The generic registration method was recovered and classified.
        assert_eq!(svc.registrations.len(), 1, "add_route registration must be recovered");
        let reg = &svc.registrations[0];
        assert_eq!(reg.method, "add_route");
        assert_eq!(reg.callback_param, "handler");
        assert_eq!(reg.callback_contract, "Handler");
        assert!(
            reg.metadata_params.iter().all(|p| p.name != "handler"),
            "callback param must be excluded from metadata_params"
        );
        assert!(
            reg.metadata_params.iter().any(|p| p.name == "path"),
            "metadata param `path` expected"
        );

        // Entrypoints
        assert_eq!(svc.entrypoints.len(), 2, "expected run + finalize entrypoints");
        let run_ep = svc
            .entrypoints
            .iter()
            .find(|e| e.method == "run")
            .expect("run entrypoint");
        assert_eq!(run_ep.kind, EntrypointKind::Run);
        assert!(run_ep.is_async, "run must be async");
        let fin_ep = svc
            .entrypoints
            .iter()
            .find(|e| e.method == "into_router")
            .expect("into_router entrypoint");
        assert_eq!(fin_ep.kind, EntrypointKind::Finalize);

        // App type must be marked binding-excluded.
        let app_type = surface.types.iter().find(|t| t.name == "App");
        if let Some(t) = app_type {
            assert!(t.binding_excluded, "App must be marked binding_excluded");
        }
    }

    #[test]
    fn missing_owner_type_returns_warning_not_panic() {
        let mut surface = extract_source("pub struct Unrelated {}");
        let config = crate::core::config::ResolvedCrateConfig {
            name: "test_crate".to_string(),
            services: vec![ServiceConfig {
                owner_type: "NonExistent".to_string(),
                constructor: None,
                configurators: vec![],
                registrations: vec![],
                entrypoints: vec![],
                skip_languages: vec![],
            }],
            ..Default::default()
        };
        let warnings = extract_services(&mut surface, &config);
        assert!(
            !warnings.is_empty(),
            "missing owner type must produce a warning, got none"
        );
        assert!(surface.services.is_empty(), "no ServiceDef must be pushed on failure");
    }

    #[test]
    fn empty_services_config_is_a_no_op() {
        let mut surface = extract_source("pub struct Foo {}");
        let config = crate::core::config::ResolvedCrateConfig {
            name: "test_crate".to_string(),
            ..Default::default()
        };
        let warnings = extract_services(&mut surface, &config);
        assert!(warnings.is_empty());
        assert!(surface.services.is_empty());
        assert!(surface.handler_contracts.is_empty());
    }
}
