//! Elixir (Rustler) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Elixir module-based callbacks via Rustler term dispatch.
//!
//! Two patterns are supported:
//!
//! 1. **Visitor bridge** (per-call, all methods have defaults): Accepts an Elixir map
//!    (`rustler::Term`) that encodes visitor overrides as function references
//!    (anonymous functions / `fn/arity` captures). Called via `rustler::Env::run_gc()`.
//!    Bridge param becomes `Option<rustler::Term<'_>>`.
//!
//! 2. **Plugin bridge** (registered, cached, async-friendly): Uses `LocalPid` to enable
//!    message passing to a GenServer-backed Elixir implementation. The bridge stores only
//!    a `LocalPid` (which is Copy + Send + Sync) and dispatches via channels to satisfy
//!    `Plugin: Send + Sync + 'static` bounds. Supports both sync (via `block_on`) and
//!    async dispatch to Elixir callbacks.

pub use alef_codegen::generators::trait_bridge::find_bridge_param;
use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Rustler-specific trait bridge generator.
/// Implements code generation for bridging Elixir modules to Rust traits via NIFs.
pub struct RustlerBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for RustlerBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "rustler::LocalPid"
    }

    fn bridge_imports(&self) -> Vec<String> {
        // async_trait is needed because the trait impls may have async methods.
        // We import the prelude to ensure the async_trait attribute is available.
        vec!["async_trait::async_trait".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let has_error = method.error_type.is_some();

        // Build clone_params array
        let clone_params: Vec<minijinja::Value> = method
            .params
            .iter()
            .filter(|p| p.is_ref || matches!(&p.ty, TypeRef::String))
            .map(|p| {
                minijinja::context! {
                    name => p.name.clone()
                }
            })
            .collect();

        // Build params array with json_expr
        let params: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                let json_expr = build_json_arg(p, spec.bridge_config);
                minijinja::context! {
                    name => p.name.clone(),
                    json_expr => json_expr
                }
            })
            .collect();

        // Build error constructors
        let error_deser = spec
            .error_constructor
            .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
        let error_msg = spec.error_constructor.replace("{msg}", "msg");
        let error_closed = spec
            .error_constructor
            .replace("{msg}", "\"Channel closed before reply received\".to_string()");

        let ctx = minijinja::context! {
            clone_params => clone_params,
            params => params,
            method_name => method.name,
            has_error => has_error,
            error_deser => error_deser,
            error_msg => error_msg,
            error_closed => error_closed
        };

        crate::template_env::render("sync_method_body.rs.jinja", ctx)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let has_error = method.error_type.is_some();

        // Build param_clones array
        let param_clones: Vec<minijinja::Value> = method
            .params
            .iter()
            .filter(|p| p.is_ref || matches!(&p.ty, TypeRef::String))
            .map(|p| {
                minijinja::context! {
                    name => p.name.clone()
                }
            })
            .collect();

        // Build args_json array with name and expr
        let args_json: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                let expr = build_json_arg(p, spec.bridge_config);
                minijinja::context! {
                    name => p.name.clone(),
                    expr => expr
                }
            })
            .collect();

        // Build error constructors
        let error_deser = spec
            .error_constructor
            .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
        let error_msg = spec.error_constructor.replace("{msg}", "msg");
        let error_closed = spec
            .error_constructor
            .replace("{msg}", "\"Channel closed before reply received\".to_string()");

        let ctx = minijinja::context! {
            param_clones => param_clones,
            args_json => args_json,
            method_name => method.name,
            has_error => has_error,
            error_deser => error_deser,
            error_msg => error_msg,
            error_closed => error_closed
        };

        crate::template_env::render("trait_async_method_body.rs.jinja", ctx)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let ctx = minijinja::context! {
            wrapper_name => wrapper
        };
        crate::template_env::render("trait_constructor.rs.jinja", ctx)
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        let ctx = minijinja::context! {
            unregister_fn => unregister_fn,
            host_path => host_path
        };
        crate::template_env::render("trait_unregistration_fn.rs.jinja", ctx)
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let ctx = minijinja::context! {
            clear_fn => clear_fn,
            host_path => host_path
        };
        crate::template_env::render("trait_clear_fn.rs.jinja", ctx)
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(register_fn) = spec.bridge_config.register_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let wrapper = spec.wrapper_name();
        let trait_path = spec.trait_path();

        // Register in plugin registry, including any extra arguments (e.g., priority for PostProcessor)
        let extra_args = spec.bridge_config.register_extra_args.as_deref().unwrap_or_default();

        let ctx = minijinja::context! {
            register_fn => register_fn,
            wrapper_name => wrapper,
            trait_path => trait_path,
            registry_getter => registry_getter,
            extra_args => extra_args
        };
        crate::template_env::render("trait_registration_fn.rs.jinja", ctx)
    }
}

impl RustlerBridgeGenerator {
    /// Generate support NIFs for completing trait calls from Elixir.
    pub fn gen_support_nifs(&self) -> String {
        let ctx = minijinja::context! {};
        crate::template_env::render("trait_support_nifs.rs.jinja", ctx)
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> BridgeOutput {
    // Build type name → rust_path lookup: convert to owned HashMap<String, String>
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (e.g. `&InternalDocument`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");

        gen_visitor_bridge(
            &mut out,
            &VisitorBridgeCtx {
                trait_type,
                struct_name: &struct_name,
                trait_path: &trait_path,
                core_crate: core_import,
                type_paths: &type_paths,
                bridge_cfg,
            },
        );
        BridgeOutput {
            imports: vec![],
            code: out,
        }
    } else {
        // Plugin-style bridge: use the IR-driven TraitBridgeGenerator infrastructure
        let generator = RustlerBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Rustler",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let output = gen_bridge_all(&spec, &generator);
        // Note: trait support NIFs (complete_trait_call/fail_trait_call) must be emitted
        // only once, not per-bridge. They are now emitted in gen_bindings/mod.rs after
        // trait bridge generation to avoid duplicate NIF definitions.
        output
    }
}

/// Parameters for [`gen_visitor_bridge`], grouped to keep argument count under the lint limit.
struct VisitorBridgeCtx<'a> {
    trait_type: &'a TypeDef,
    struct_name: &'a str,
    trait_path: &'a str,
    core_crate: &'a str,
    type_paths: &'a std::collections::HashMap<String, String>,
    bridge_cfg: &'a alef_core::config::TraitBridgeConfig,
}

/// Generate a visitor-style bridge wrapping a `rustler::OwnedEnv` + `rustler::Term`.
///
/// This generates an async message-passing bridge. When `convert_with_visitor` is called,
/// it spawns a system thread that runs the conversion. Each visitor callback sends a
/// `{:visitor_callback, ref_id, callback_name, args_json}` message to the calling Elixir
/// process and blocks on a channel waiting for the reply from `visitor_reply/2`.
/// When conversion finishes, the thread sends `{:ok, result_json}` or `{:error, reason}`
/// to the caller.
fn gen_visitor_bridge(out: &mut String, ctx: &VisitorBridgeCtx<'_>) {
    let VisitorBridgeCtx {
        trait_type,
        struct_name,
        trait_path,
        core_crate,
        type_paths,
        bridge_cfg,
    } = ctx;
    // Helper: convert NodeContext to a Rustler NifMap term inside an OwnedEnv
    let ctx_helper = minijinja::context! {
        core_crate => core_crate
    };
    out.push_str(&crate::template_env::render(
        "visitor_bridge_helper.rs.jinja",
        ctx_helper,
    ));

    // Global channel registry: maps ref_id -> SyncSender so visitor_reply can unblock the bridge.
    out.push_str(&crate::template_env::render(
        "visitor_bridge_globals.rs.jinja",
        minijinja::context! {},
    ));

    // Bridge struct: holds the caller PID and the visitor term in its OwnedEnv.
    // Both OwnedEnv and SavedTerm are Send, so the bridge can be moved to a system thread.
    let ctx_struct = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::template_env::render(
        "visitor_bridge_struct.rs.jinja",
        ctx_struct,
    ));

    // Manual Debug impl (required by HtmlVisitor bound: std::fmt::Debug)
    let ctx_debug = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::template_env::render("visitor_bridge_debug.rs.jinja", ctx_debug));

    // Constructor (called from BEAM thread — saves visitor term into an OwnedEnv)
    let ctx_constructors = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::template_env::render(
        "visitor_bridge_constructors.rs.jinja",
        ctx_constructors,
    ));

    // Helper: send a visitor callback message and block waiting for the reply.
    // Encoded as: {:visitor_callback, ref_id, callback_atom, args_json_string}
    let ctx_send_wait = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::template_env::render(
        "visitor_send_and_wait.rs.jinja",
        ctx_send_wait,
    ));

    // visitor_reply NIF: called by Elixir to unblock a waiting visitor callback.
    // Returns () which Rustler encodes as :ok.
    out.push_str(&crate::template_env::render(
        "visitor_reply_nif.rs.jinja",
        minijinja::context! {},
    ));

    // Trait impl — each method sends callback message and waits for reply.
    out.push_str(&crate::template_env::render(
        "trait_impl_header.jinja",
        minijinja::context! {
            trait_path => trait_path,
            struct_name => struct_name,
        },
    ));
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_async(out, method, type_paths, struct_name, bridge_cfg);
    }
    out.push_str("}\n");
    out.push('\n');
}

/// Generate a single async visitor method that sends a callback message to the Elixir
/// process and blocks on an mpsc channel waiting for the reply from `visitor_reply/2`.
///
/// Arguments are serialized to a JSON object string so the Elixir side can decode them
/// with Jason without depending on Rustler types.
fn gen_visitor_method_async(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<String, String>,
    _struct_name: &str,
    bridge_cfg: &alef_core::config::TraitBridgeConfig,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    // Convert method name from visit_* to handle_* for Elixir convention.
    // E.g., "visit_audio" -> "handle_audio"
    let handle_name = if let Some(suffix) = name.strip_prefix("visit_") {
        format!("handle_{suffix}")
    } else {
        name.clone()
    };

    // Build args for the template
    let args: Vec<minijinja::Value> = method
        .params
        .iter()
        .map(|p| {
            let json_expr = build_json_arg(p, bridge_cfg);
            minijinja::context! {
                key => p.name.clone(),
                expr => json_expr
            }
        })
        .collect();

    let ctx = minijinja::context! {
        method_name => name,
        sig => sig,
        ret_ty => ret_ty,
        handle_name => handle_name,
        args => args
    };
    out.push_str(&crate::template_env::render("visitor_method.rs.jinja", ctx));
}

/// Build a serde_json::Value expression for a visitor method parameter (for the args JSON object).
fn build_json_arg(p: &alef_core::ir::ParamDef, bridge_cfg: &alef_core::config::TraitBridgeConfig) -> String {
    // context_type param: serialize as a JSON object via serde_json.
    if let TypeRef::Named(n) = &p.ty {
        if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
            let ref_expr = if p.is_ref {
                p.name.clone()
            } else {
                format!("&{}", p.name)
            };
            return format!("serde_json::to_value({ref_expr}).unwrap_or(serde_json::Value::Null)");
        }
    }
    // Optional string params (must check before non-optional: Option<&str> also has is_ref=true)
    if p.optional && matches!(&p.ty, TypeRef::String) {
        return format!(
            "match {0} {{ Some(s) => serde_json::Value::String(s.to_string()), None => serde_json::Value::Null }}",
            p.name
        );
    }
    // String params
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("serde_json::Value::String({}.to_string())", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("serde_json::Value::String({}.clone())", p.name);
    }
    // Bool params
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("serde_json::Value::Bool({})", p.name);
    }
    // Slice params (e.g. &[String])
    if matches!(&p.ty, TypeRef::Vec(_)) && p.is_ref {
        return format!("serde_json::to_value({}).unwrap_or(serde_json::Value::Null)", p.name);
    }
    // usize / u32 numeric params
    if matches!(
        &p.ty,
        TypeRef::Primitive(
            alef_core::ir::PrimitiveType::Usize
                | alef_core::ir::PrimitiveType::U8
                | alef_core::ir::PrimitiveType::U16
                | alef_core::ir::PrimitiveType::U32
                | alef_core::ir::PrimitiveType::U64
        )
    ) {
        return format!("serde_json::Value::Number(serde_json::Number::from({} as u64))", p.name);
    }
    // i64 / isize numeric params
    if matches!(
        &p.ty,
        TypeRef::Primitive(
            alef_core::ir::PrimitiveType::I8
                | alef_core::ir::PrimitiveType::I16
                | alef_core::ir::PrimitiveType::I32
                | alef_core::ir::PrimitiveType::I64
                | alef_core::ir::PrimitiveType::Isize
        )
    ) {
        return format!("serde_json::Value::Number(serde_json::Number::from({} as i64))", p.name);
    }
    // Fallback: debug-print as string
    format!("serde_json::Value::String(format!(\"{{:?}}\", {}))", p.name)
}

/// Generate a Rustler NIF function that has one parameter replaced by
/// `Option<rustler::Term<'_>>` (a trait bridge). The bridge is constructed before
/// calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list — Rustler NIFs always have `env: rustler::Env<'_>` as first param
    let mut sig_parts = vec!["env: rustler::Env<'_>".to_string()];
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<rustler::Term<'_>>", p.name));
            } else {
                sig_parts.push(format!("{}: rustler::Term<'_>", p.name));
            }
        } else {
            // Use the same type mapping as gen_nif_function
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    let promoted = idx > bridge_param_idx;
                    if promoted || p.optional {
                        sig_parts.push(format!("{}: Option<rustler::ResourceArc<{}>>", p.name, n));
                    } else {
                        sig_parts.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                    }
                    continue;
                }
                if default_types.contains(n) {
                    sig_parts.push(format!("{}: Option<String>", p.name));
                    continue;
                }
            }
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            let ty = if p.optional || promoted {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| e.to_string())";

    // Bridge wrapping code
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(term) if term.atom_to_string().ok().as_deref() != Some(\"nil\") => {{\n            \
             let bridge = {struct_name}::new(env, env.pid(), term);\n            \
             Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new(env, env.pid(), {param_name});\n        \
             std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
             }};"
        )
    };

    // Let bindings for non-bridge params that need deserialization
    let deser_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == bridge_param_idx {
                return false;
            }
            match &p.ty {
                TypeRef::Named(n) => !opaque_types.contains(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        !opaque_types.contains(n.as_str())
                    } else {
                        false
                    }
                }
                _ => false,
            }
        })
        .map(|(_, p)| {
            let name = &p.name;
            if let TypeRef::Named(n) = &p.ty {
                if default_types.contains(n) {
                    let core_ty = format!("{core_import}::{n}");
                    return format!(
                        "let {name}_core: Option<{core_ty}> = {name}.map(|s| serde_json::from_str::<{core_ty}>(&s){err_conv}).transpose(){err_conv}?;\n    "
                    );
                }
                let core_ty = format!("{core_import}::{n}");
                if p.optional {
                    return format!("let {name}_core: Option<{core_ty}> = {name}.map(Into::into);\n    ");
                }
                return format!("let {name}_core: {core_ty} = {name}.into();\n    ");
            }
            String::new()
        })
        .collect();

    // Build call args
    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == bridge_param_idx {
                return p.name.clone();
            }
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n.as_str()) {
                    if p.optional {
                        return format!("{}.as_ref().map(|v| &v.inner)", p.name);
                    }
                    return format!("&{}.inner", p.name);
                }
                if default_types.contains(n) {
                    return format!("{}_core", p.name);
                }
            }
            match &p.ty {
                TypeRef::Named(_) => format!("{}_core", p.name),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if opaque_types.contains(n.as_str()) {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect();
    let call_args_str = call_args.join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args_str})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("rustler::ResourceArc::new({name}Inner {{ inner: std::sync::Arc::new(val) }})")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {deser_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {deser_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {deser_bindings}{core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(2048);
    let ctx = minijinja::context! {
        func_name => func_name,
        params_str => params_str,
        ret => ret,
        body => body
    };
    out.push_str(&crate::template_env::render("nif_function.rs.jinja", ctx));

    // Generate the async visitor NIF only when the bridge parameter is the visitor.
    // This NIF spawns a system thread, builds the bridge from the caller PID + visitor term,
    // runs conversion, and sends the result back as a {:ok, result} / {:error, reason} message.
    if is_optional {
        // Build the non-bridge params signature for convert_with_visitor
        // (bridge param replaced by the concrete Elixir term — never nil here).
        let mut with_sig_parts = vec!["env: rustler::Env<'_>".to_string()];
        for (idx, p) in func.params.iter().enumerate() {
            if idx == bridge_param_idx {
                // visitor is required (not optional) in convert_with_visitor
                with_sig_parts.push(format!("{}: rustler::Term<'_>", p.name));
            } else if let TypeRef::Named(n) = &p.ty {
                if default_types.contains(n) {
                    with_sig_parts.push(format!("{}: Option<String>", p.name));
                } else {
                    let mapped = mapper.map_type(&p.ty);
                    if p.optional {
                        with_sig_parts.push(format!("{}: Option<{}>", p.name, mapped));
                    } else {
                        with_sig_parts.push(format!("{}: {}", p.name, mapped));
                    }
                }
            } else {
                let mapped = mapper.map_type(&p.ty);
                if p.optional {
                    with_sig_parts.push(format!("{}: Option<{}>", p.name, mapped));
                } else {
                    with_sig_parts.push(format!("{}: {}", p.name, mapped));
                }
            }
        }
        let with_params_str = with_sig_parts.join(", ");

        // Build the deser bindings string for non-bridge, non-opaque named params.
        let with_deser: String = func
            .params
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != bridge_param_idx)
            .filter_map(|(_, p)| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        return Some(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s).map_err(|e| e.to_string())).transpose().map_err(|e| e.to_string())?;\n    ",
                            p.name, core_ty
                        ));
                    }
                }
                None
            })
            .collect();

        // Build the call args, replacing the bridge param with the VisitorHandle.
        let with_call_args: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                if idx == bridge_param_idx {
                    // visitor_handle is built inside the thread closure
                    p.name.clone()
                } else if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        format!("{}_core", p.name)
                    } else {
                        match &p.ty {
                            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                            _ => p.name.clone(),
                        }
                    }
                } else {
                    match &p.ty {
                        TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                        _ => p.name.clone(),
                    }
                }
            })
            .collect();
        let with_call_args_str = with_call_args.join(", ");

        // Clone non-bridge params before moving into the thread.
        let clone_stmts: String = func
            .params
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != bridge_param_idx)
            .map(|(_, p)| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        return format!("let {0}_core = {0}_core;\n    ", p.name);
                    }
                }
                match &p.ty {
                    TypeRef::String | TypeRef::Char => format!("let {0} = {0}.clone();\n    ", p.name),
                    _ => String::new(),
                }
            })
            .collect();

        out.push('\n');
        let ctx = minijinja::context! {
            func_name => func_name,
            with_params_str => with_params_str,
            with_deser => with_deser,
            param_name => param_name,
            clone_stmts => clone_stmts,
            struct_name => struct_name,
            handle_path => handle_path,
            core_fn_path => core_fn_path,
            with_call_args_str => with_call_args_str
        };
        out.push_str(&crate::template_env::render(
            "nif_with_visitor_async_body.rs.jinja",
            ctx,
        ));
    }

    out
}

/// Generate NIF functions for an `options_field` visitor bridge.
///
/// For `options_field` bridges the visitor is embedded in the options struct
/// rather than being a direct function parameter.  We generate two NIFs:
///
/// 1. The plain NIF: `fn convert(html, options)` — no visitor, just deserialises
///    options and calls the core function directly (same as `gen_nif_function`).
///
/// 2. The async visitor NIF: `fn convert_with_visitor(env, html, options, visitor)`
///    — pops the visitor from the Elixir caller, builds the bridge struct, injects
///    it as `options.visitor`, then spawns a system thread, runs conversion, and
///    sends the result as `{:ok, result}` / `{:error, reason}` to the BEAM process.
///
/// The Elixir public-API wrapper in `html_to_markdown.ex` calls
/// `Native.convert_with_visitor(html, clean_opts, visitor)` when a visitor map
/// is present, or falls back to `Native.convert(html, opts_json)` otherwise.
pub fn gen_bridge_field_function(
    func: &alef_core::ir::FunctionDef,
    bridge_match: &alef_codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &crate::type_map::RustlerMapper,
    opaque_types: &ahash::AHashSet<String>,
    _default_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_codegen::type_mapper::TypeMapper;
    use alef_core::ir::TypeRef;

    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let func_name = &func.name;
    let field_name = &bridge_match.field_name;
    let options_param = &bridge_match.param_name;
    let options_type = &bridge_match.options_type;
    let core_options_type = format!("{core_import}::options::{options_type}");

    // Whether the core function expects Option<ConversionOptions> (optional=true).
    let options_param_is_optional = func
        .params
        .iter()
        .find(|p| p.name == *options_param)
        .is_some_and(|p| p.optional || matches!(&p.ty, TypeRef::Optional(_)));

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    // ── 1. Plain NIF (no visitor) ─────────────────────────────────────────────
    // Parameters: all original params. Options type is passed as Option<String> (JSON).
    let mut plain_sig: Vec<String> = Vec::new();
    for p in &func.params {
        let ty = if p.name == *options_param {
            "Option<String>".to_string()
        } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                format!("rustler::ResourceArc<{n}>")
            } else {
                mapper.map_type(&p.ty)
            }
        } else {
            mapper.map_type(&p.ty)
        };
        plain_sig.push(format!("{}: {}", p.name, ty));
    }
    let plain_params_str = plain_sig.join(", ");

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());
    let err_conv = ".map_err(|e| e.to_string())";

    // Build call args for the plain NIF.
    let plain_call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                // Deserialise options JSON → core type.
                // When the core param is Option<T>, keep it wrapped; otherwise unwrap to Default.
                if options_param_is_optional {
                    format!(
                        "{options_param}.map(|s| serde_json::from_str::<{core_options_type}>(&s).unwrap_or_default())"
                    )
                } else {
                    format!(
                        "{options_param}.map(|s| serde_json::from_str::<{core_options_type}>(&s).unwrap_or_default()).unwrap_or_default()"
                    )
                }
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n) => format!("&{}.inner", p.name),
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    _ => p.name.clone(),
                }
            }
        })
        .collect();
    let plain_call_args_str = plain_call_args.join(", ");

    let plain_body = if func.error_type.is_some() {
        format!("{core_fn_path}({plain_call_args_str})\n        .map(|val| val.into()){err_conv}")
    } else {
        format!("{core_fn_path}({plain_call_args_str}).into()")
    };

    let mut out = String::with_capacity(2048);
    let ctx = minijinja::context! {
        func_name => func_name,
        params_str => plain_params_str,
        ret => ret,
        body => plain_body
    };
    out.push_str(&crate::template_env::render("dirty_cpu_nif_function.rs.jinja", ctx));

    // ── 2. Async visitor NIF ──────────────────────────────────────────────────
    // Signature: env + original params + `visitor: Term<'_>` at the end.
    let mut vis_sig: Vec<String> = vec!["env: rustler::Env<'_>".to_string()];
    for p in &func.params {
        let ty = if p.name == *options_param {
            "Option<String>".to_string()
        } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                format!("rustler::ResourceArc<{n}>")
            } else {
                mapper.map_type(&p.ty)
            }
        } else {
            mapper.map_type(&p.ty)
        };
        vis_sig.push(format!("{}: {}", p.name, ty));
    }
    vis_sig.push("visitor: rustler::Term<'_>".to_string());
    let vis_params_str = vis_sig.join(", ");

    // Clone stmts for non-String params that need to move into thread.
    let clone_stmts: String = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                return String::new();
            }
            match &p.ty {
                TypeRef::String | TypeRef::Char => format!("let {} = {}.clone();\n    ", p.name, p.name),
                _ => String::new(),
            }
        })
        .collect();

    // Deser stmts: parse options JSON and set visitor field.
    let deser_stmts = format!(
        "let mut {options_param}_core: {core_options_type} = \
         {options_param}.map(|s| serde_json::from_str::<{core_options_type}>(&s).unwrap_or_default()).unwrap_or_default();\n    \
         let bridge = {struct_name}::new(env, pid, visitor_term);\n    \
         {options_param}_core.{field_name} = Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path});"
    );

    // Build call args for the visitor variant.
    let vis_call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                // Core expects Option<T> when optional=true.
                if options_param_is_optional {
                    format!("Some({options_param}_core)")
                } else {
                    format!("{options_param}_core")
                }
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n) => format!("&{}.inner", p.name),
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    _ => p.name.clone(),
                }
            }
        })
        .collect();
    let vis_call_args_str = vis_call_args.join(", ");

    out.push('\n');
    let ctx = minijinja::context! {
        func_name => func_name,
        vis_params_str => vis_params_str,
        clone_stmts => clone_stmts,
        deser_stmts => deser_stmts,
        core_fn_path => core_fn_path,
        vis_call_args_str => vis_call_args_str
    };
    out.push_str(&crate::template_env::render(
        "nif_with_visitor_field_async_body.rs.jinja",
        ctx,
    ));

    out
}
