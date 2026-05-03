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
use std::fmt::Write;

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
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(512);

        // Clone params for the blocking closure
        for p in &method.params {
            if p.is_ref || matches!(&p.ty, TypeRef::String) {
                writeln!(out, "let {0} = {0}.clone();", p.name).ok();
            }
        }

        writeln!(out).ok();
        writeln!(
            out,
            "let reply_id = TRAIT_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);"
        )
        .ok();
        writeln!(
            out,
            "let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();"
        )
        .ok();
        writeln!(out, "TRAIT_REPLY_CHANNELS.lock().unwrap().insert(reply_id, tx);").ok();
        writeln!(out).ok();

        writeln!(out, "let pid = self.inner;").ok();

        // Build args JSON from parameters
        writeln!(out, "let args_json = {{").ok();
        writeln!(out, "    let mut args = serde_json::Map::new();").ok();
        for p in &method.params {
            let json_expr = build_json_arg(p);
            writeln!(out, "    args.insert(\"{0}\".to_string(), {1});", p.name, json_expr).ok();
        }
        writeln!(out, "    serde_json::Value::Object(args).to_string()").ok();
        writeln!(out, "}};").ok();
        writeln!(out).ok();

        writeln!(out, "let method = \"{}\";", name).ok();
        writeln!(out).ok();

        writeln!(out, "tokio::task::spawn_blocking(move || {{").ok();
        writeln!(out, "    let mut env = rustler::OwnedEnv::new();").ok();
        writeln!(out, "    let _ = env.send_and_clear(&pid, |env| {{").ok();
        writeln!(
            out,
            "        (rustler::types::atom::Atom::from_str(env, \"trait_call\").unwrap(),\
             method, args_json.as_str(), reply_id).encode(env)"
        )
        .ok();
        writeln!(out, "    }});").ok();
        writeln!(out, "}});").ok();
        writeln!(out).ok();

        writeln!(out, "match rx.blocking_recv() {{").ok();
        if has_error {
            let err_deser = spec
                .error_constructor
                .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
            let line = format!(
                "    Ok(Ok(json)) => serde_json::from_str(&json).map_err(|_e| {}),",
                err_deser
            );
            out.push_str(&line);
            out.push('\n');
            let err_msg = spec.error_constructor.replace("{msg}", "msg");
            let line = format!("    Ok(Err(msg)) => Err({}),", err_msg);
            out.push_str(&line);
            out.push('\n');
            let err_closed = spec
                .error_constructor
                .replace("{msg}", "\"Channel closed before reply received\".to_string()");
            let line = format!("    Err(_) => Err({})", err_closed);
            out.push_str(&line);
            out.push('\n');
        } else {
            writeln!(
                out,
                "    Ok(Ok(json)) => serde_json::from_str(&json).unwrap_or_default(),"
            )
            .ok();
            writeln!(out, "    _ => Default::default()").ok();
        }
        writeln!(out, "}}").ok();

        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(512);

        // Clone params so they can be moved into the blocking task
        for p in &method.params {
            if p.is_ref || matches!(&p.ty, TypeRef::String) {
                writeln!(out, "let {0} = {0}.clone();", p.name).ok();
            }
        }

        writeln!(out).ok();
        writeln!(
            out,
            "let reply_id = TRAIT_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);"
        )
        .ok();
        writeln!(
            out,
            "let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();"
        )
        .ok();
        writeln!(out, "TRAIT_REPLY_CHANNELS.lock().unwrap().insert(reply_id, tx);").ok();
        writeln!(out).ok();

        writeln!(out, "let pid = self.inner;").ok();

        // Build args JSON from parameters
        writeln!(out, "let args_json = {{").ok();
        writeln!(out, "    let mut args = serde_json::Map::new();").ok();
        for p in &method.params {
            let json_expr = build_json_arg(p);
            writeln!(out, "    args.insert(\"{0}\".to_string(), {1});", p.name, json_expr).ok();
        }
        writeln!(out, "    serde_json::Value::Object(args).to_string()").ok();
        writeln!(out, "}};").ok();
        writeln!(out).ok();

        writeln!(out, "let method = \"{}\";", name).ok();
        writeln!(out).ok();

        writeln!(out, "tokio::task::spawn_blocking(move || {{").ok();
        writeln!(out, "    let mut env = rustler::OwnedEnv::new();").ok();
        writeln!(out, "    let _ = env.send_and_clear(&pid, |env| {{").ok();
        writeln!(
            out,
            "        (rustler::types::atom::Atom::from_str(env, \"trait_call\").unwrap(),\
             method, args_json.as_str(), reply_id).encode(env)"
        )
        .ok();
        writeln!(out, "    }});").ok();
        writeln!(out, "}}).await;").ok();
        writeln!(out).ok();

        writeln!(out, "match rx.await {{").ok();
        if has_error {
            let err_deser = spec
                .error_constructor
                .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
            let line = format!(
                "    Ok(Ok(json)) => serde_json::from_str(&json).map_err(|_e| {}),",
                err_deser
            );
            out.push_str(&line);
            out.push('\n');
            let err_msg = spec.error_constructor.replace("{msg}", "msg");
            let line = format!("    Ok(Err(msg)) => Err({}),", err_msg);
            out.push_str(&line);
            out.push('\n');
            let err_closed = spec
                .error_constructor
                .replace("{msg}", "\"Channel closed before reply received\".to_string()");
            let line = format!("    Err(_) => Err({})", err_closed);
            out.push_str(&line);
            out.push('\n');
        } else {
            writeln!(
                out,
                "    Ok(Ok(json)) => serde_json::from_str(&json).unwrap_or_default(),"
            )
            .ok();
            writeln!(out, "    _ => Default::default()").ok();
        }
        writeln!(out, "}}").ok();

        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(out, "    /// Create a new bridge wrapping an Elixir GenServer PID.").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// The PID is copied (LocalPid is Copy + Send + Sync) and used to send"
        )
        .ok();
        writeln!(
            out,
            "    /// messages to the backing GenServer. The plugin_name is cached for fast"
        )
        .ok();
        writeln!(out, "    /// Plugin::name() lookups.").ok();
        writeln!(
            out,
            "    pub fn new(pid: rustler::LocalPid, plugin_name: String) -> Self {{"
        )
        .ok();
        writeln!(out, "        Self {{").ok();
        writeln!(out, "            inner: pid,").ok();
        writeln!(out, "            cached_name: plugin_name,").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        let mut out = String::with_capacity(512);
        writeln!(out, "#[rustler::nif]").ok();
        writeln!(
            out,
            "pub fn {unregister_fn}(env: rustler::Env<'_>, name: String) -> rustler::Atom {{"
        )
        .ok();
        writeln!(out, "    match {host_path}(&name) {{").ok();
        writeln!(
            out,
            "        Ok(_) => rustler::types::atom::Atom::from_str(env, \"ok\").unwrap(),"
        )
        .ok();
        writeln!(
            out,
            "        Err(_) => rustler::types::atom::Atom::from_str(env, \"error\").unwrap(),"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let mut out = String::with_capacity(512);
        writeln!(out, "#[rustler::nif]").ok();
        writeln!(out, "pub fn {clear_fn}(env: rustler::Env<'_>) -> rustler::Atom {{").ok();
        writeln!(out, "    match {host_path}() {{").ok();
        writeln!(
            out,
            "        Ok(_) => rustler::types::atom::Atom::from_str(env, \"ok\").unwrap(),"
        )
        .ok();
        writeln!(
            out,
            "        Err(_) => rustler::types::atom::Atom::from_str(env, \"error\").unwrap(),"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
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

        let mut out = String::with_capacity(1024);

        writeln!(out, "#[rustler::nif]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(env: rustler::Env<'_>, genserver_pid: rustler::LocalPid, plugin_name: String) -> rustler::Atom {{"
        )
        .ok();

        writeln!(out).ok();
        writeln!(out, "    let bridge = {wrapper}::new(genserver_pid, plugin_name);").ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(bridge);").ok();
        writeln!(out).ok();

        // Register in plugin registry, including any extra arguments (e.g., priority for PostProcessor)
        let extra_args = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    match registry.write().register(arc{extra_args}) {{").ok();
        writeln!(
            out,
            "        Ok(_) => rustler::types::atom::Atom::from_str(env, \"ok\").unwrap(),"
        )
        .ok();
        writeln!(
            out,
            "        Err(_) => rustler::types::atom::Atom::from_str(env, \"error\").unwrap(),"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }
}

impl RustlerBridgeGenerator {
    /// Generate support NIFs for completing trait calls from Elixir.
    pub fn gen_support_nifs(&self) -> String {
        let mut out = String::with_capacity(1024);

        writeln!(out, "/// Complete a pending trait call with a successful JSON result.").ok();
        writeln!(
            out,
            "/// Called from Elixir GenServer after handling a trait method call."
        )
        .ok();
        writeln!(out, "#[rustler::nif]").ok();
        writeln!(
            out,
            "pub fn complete_trait_call(env: rustler::Env, reply_id: u64, result_json: String) -> rustler::Atom {{"
        )
        .ok();
        writeln!(
            out,
            "    if let Some(tx) = TRAIT_REPLY_CHANNELS.lock().unwrap().remove(&reply_id) {{"
        )
        .ok();
        writeln!(out, "        let _ = tx.send(Ok(result_json));").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    rustler::types::atom::ok()").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        writeln!(out, "/// Fail a pending trait call with an error message.").ok();
        writeln!(out, "/// Called from Elixir GenServer if handling fails.").ok();
        writeln!(out, "#[rustler::nif]").ok();
        writeln!(
            out,
            "pub fn fail_trait_call(env: rustler::Env, reply_id: u64, error_message: String) -> rustler::Atom {{"
        )
        .ok();
        writeln!(
            out,
            "    if let Some(tx) = TRAIT_REPLY_CHANNELS.lock().unwrap().remove(&reply_id) {{"
        )
        .ok();
        writeln!(out, "        let _ = tx.send(Err(error_message));").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    rustler::types::atom::ok()").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        out
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
        let mut output = gen_bridge_all(&spec, &generator);

        // Append the support NIFs for completing trait calls from Elixir
        output.code.push_str("\n\n");
        output.code.push_str(&generator.gen_support_nifs());

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
    } = ctx;
    // Helper: convert NodeContext to a Rustler NifMap term inside an OwnedEnv
    writeln!(out, "fn nodecontext_to_elixir_map<'a>(").unwrap();
    writeln!(out, "    env: rustler::Env<'a>,").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> rustler::Term<'a> {{").unwrap();
    writeln!(
        out,
        "    let mut pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = Vec::new();"
    )
    .unwrap();
    // Encode node_type as a snake_case atom (e.g. DefinitionList -> :definition_list)
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        let node_type_debug = format!(\"{{:?}}\", ctx.node_type);").unwrap();
    writeln!(
        out,
        "        let node_type_snake: String = node_type_debug.chars().enumerate()"
    )
    .unwrap();
    writeln!(out, "            .flat_map(|(i, c)| {{").unwrap();
    writeln!(
        out,
        "                if c.is_uppercase() && i > 0 {{ vec!['_', c.to_lowercase().next().unwrap()] }}"
    )
    .unwrap();
    writeln!(
        out,
        "                else if c.is_uppercase() {{ vec![c.to_lowercase().next().unwrap()] }}"
    )
    .unwrap();
    writeln!(out, "                else {{ vec![c] }}").unwrap();
    writeln!(out, "            }}).collect();").unwrap();
    writeln!(
        out,
        "        pairs.push((rustler::types::atom::Atom::from_str(env, \"node_type\").unwrap().to_term(env), rustler::types::atom::Atom::from_str(env, &node_type_snake).unwrap().to_term(env)));"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"tag_name\").unwrap().to_term(env), ctx.tag_name.encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"depth\").unwrap().to_term(env), (ctx.depth as i64).encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"index_in_parent\").unwrap().to_term(env), (ctx.index_in_parent as i64).encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"is_inline\").unwrap().to_term(env), ctx.is_inline.encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    let parent_tag_term = match &ctx.parent_tag {{ Some(s) => s.encode(env), None => rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env) }};"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"parent_tag\").unwrap().to_term(env), parent_tag_term));"
    )
    .unwrap();
    writeln!(
        out,
        "    let attrs_pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = ctx.attributes.iter().map(|(k, v)| (k.encode(env), v.encode(env))).collect();"
    )
    .unwrap();
    writeln!(
        out,
        "    let attrs_map = rustler::Term::map_from_pairs(env, &attrs_pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"attributes\").unwrap().to_term(env), attrs_map));"
    )
    .unwrap();
    writeln!(
        out,
        "    rustler::Term::map_from_pairs(env, &pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env))"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Global channel registry: maps ref_id -> SyncSender so visitor_reply can unblock the bridge.
    writeln!(
        out,
        "static VISITOR_REPLY_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);"
    )
    .unwrap();
    writeln!(
        out,
        "static VISITOR_CHANNELS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, std::sync::mpsc::SyncSender<Option<String>>>>> ="
    )
    .unwrap();
    writeln!(
        out,
        "    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));"
    )
    .unwrap();
    writeln!(out).unwrap();

    // Bridge struct: holds the caller PID and the visitor term in its OwnedEnv.
    // Both OwnedEnv and SavedTerm are Send, so the bridge can be moved to a system thread.
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    caller_pid: rustler::types::LocalPid,").unwrap();
    writeln!(out, "    visitor_env: rustler::OwnedEnv,").unwrap();
    writeln!(out, "    visitor_saved: rustler::env::SavedTerm,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Manual Debug impl (required by HtmlVisitor bound: std::fmt::Debug)
    writeln!(out, "impl std::fmt::Debug for {struct_name} {{").unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
    writeln!(out, "        write!(f, \"{struct_name}\")").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Constructor (called from BEAM thread — saves visitor term into an OwnedEnv)
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(
        out,
        "    pub fn new(env: rustler::Env<'_>, caller_pid: rustler::types::LocalPid, visitor_term: rustler::Term<'_>) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        let owned = rustler::OwnedEnv::new();").unwrap();
    writeln!(out, "        let saved = owned.save(visitor_term);").unwrap();
    writeln!(
        out,
        "        Self {{ caller_pid, visitor_env: owned, visitor_saved: saved }}"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    // Constructor from pre-built OwnedEnv + SavedTerm (called from worker thread where
    // the OwnedEnv was already populated on the BEAM thread before spawning).
    writeln!(
        out,
        "    pub fn new_from_saved(caller_pid: rustler::types::LocalPid, visitor_env: rustler::OwnedEnv, visitor_saved: rustler::env::SavedTerm) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        Self {{ caller_pid, visitor_env, visitor_saved }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Helper: send a visitor callback message and block waiting for the reply.
    // Encoded as: {:visitor_callback, ref_id, callback_atom, args_json_string}
    writeln!(
        out,
        "fn visitor_send_and_wait(bridge: &{struct_name}, callback_name: &str, args_json: String) -> Option<String> {{"
    )
    .unwrap();
    writeln!(
        out,
        "    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<String>>(1);"
    )
    .unwrap();
    writeln!(
        out,
        "    let ref_id = VISITOR_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);"
    )
    .unwrap();
    writeln!(out, "    VISITOR_CHANNELS.lock().unwrap().insert(ref_id, tx);").unwrap();
    writeln!(out, "    let pid = bridge.caller_pid;").unwrap();
    writeln!(out, "    let cb_name = callback_name.to_string();").unwrap();
    writeln!(out, "    let mut msg_env = rustler::OwnedEnv::new();").unwrap();
    writeln!(out, "    let _ = msg_env.send_and_clear(&pid, |env| {{").unwrap();
    writeln!(
        out,
        "        let tag = rustler::types::atom::Atom::from_str(env, \"visitor_callback\").unwrap().to_term(env);"
    )
    .unwrap();
    writeln!(out, "        let ref_term = ref_id.encode(env);").unwrap();
    writeln!(
        out,
        "        let name_term = rustler::types::atom::Atom::from_str(env, &cb_name).unwrap().to_term(env);"
    )
    .unwrap();
    writeln!(out, "        let args_term = args_json.encode(env);").unwrap();
    writeln!(
        out,
        "        rustler::types::tuple::make_tuple(env, &[tag, ref_term, name_term, args_term])"
    )
    .unwrap();
    writeln!(out, "    }});").unwrap();
    writeln!(out, "    let result = rx.recv().ok().flatten();").unwrap();
    writeln!(out, "    VISITOR_CHANNELS.lock().unwrap().remove(&ref_id);").unwrap();
    writeln!(out, "    result").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // visitor_reply NIF: called by Elixir to unblock a waiting visitor callback.
    // Returns () which Rustler encodes as :ok.
    writeln!(out, "#[rustler::nif]").unwrap();
    writeln!(out, "pub fn visitor_reply(ref_id: u64, result: Option<String>) {{").unwrap();
    writeln!(
        out,
        "    if let Some(tx) = VISITOR_CHANNELS.lock().unwrap().get(&ref_id) {{"
    )
    .unwrap();
    writeln!(out, "        let _ = tx.send(result);").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl — each method sends callback message and waits for reply.
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_async(out, method, type_paths, struct_name);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
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

    writeln!(out, "    fn {name}({sig}) -> {ret_ty} {{").unwrap();

    // Build a JSON object string from the method parameters for Elixir to decode.
    writeln!(out, "        let mut args_map = serde_json::Map::new();").unwrap();
    for p in &method.params {
        let json_expr = build_json_arg(p);
        writeln!(
            out,
            "        args_map.insert(\"{0}\".to_string(), {1});",
            p.name, json_expr
        )
        .unwrap();
    }
    writeln!(
        out,
        "        let args_json = serde_json::Value::Object(args_map).to_string();"
    )
    .unwrap();

    // Send callback and wait for reply.
    writeln!(
        out,
        "        let result = visitor_send_and_wait(self, \"{name}\", args_json);"
    )
    .unwrap();

    // Parse the string reply into a VisitResult.
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            None => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Some(s) => match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(out, "                other => {ret_ty}::Custom(other.to_string()),").unwrap();
    writeln!(out, "            }},").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a serde_json::Value expression for a visitor method parameter (for the args JSON object).
fn build_json_arg(p: &alef_core::ir::ParamDef) -> String {
    // NodeContext: serialize as a JSON object via serde_json.
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
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
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new(env, env.pid(), {param_name});\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
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
    writeln!(out, "#[rustler::nif]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

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

        writeln!(out).ok();
        writeln!(
            out,
            "// Async visitor variant: spawns a system thread, sends result as a message."
        )
        .ok();
        writeln!(out, "#[rustler::nif]").ok();
        writeln!(
            out,
            "pub fn {func_name}_with_visitor({with_params_str}) -> Result<(), String> {{"
        )
        .ok();
        writeln!(out, "    let pid = env.pid();").ok();
        writeln!(out, "    {with_deser}").ok();

        // Save visitor term + build owned env before spawning (must happen on BEAM thread)
        writeln!(out, "    let visitor_owned_env = rustler::OwnedEnv::new();").ok();
        writeln!(out, "    let visitor_saved = visitor_owned_env.save({param_name});").ok();
        writeln!(out, "    {clone_stmts}").ok();

        writeln!(out, "    std::thread::spawn(move || {{").ok();
        writeln!(
            out,
            "        let bridge = {struct_name}::new_from_saved(pid, visitor_owned_env, visitor_saved);"
        )
        .ok();
        writeln!(
            out,
            "        let {param_name}: Option<{handle_path}> = Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path});"
        )
        .ok();
        writeln!(out, "        let mut result_env = rustler::OwnedEnv::new();").ok();
        writeln!(out, "        let _ = result_env.send_and_clear(&pid, |env| {{").ok();
        writeln!(out, "            match {core_fn_path}({with_call_args_str}) {{").ok();
        writeln!(out, "                Ok(val) => {{").ok();
        writeln!(out, "                    let result: ConversionResult = val.into();").ok();
        writeln!(
            out,
            "                    let ok_atom = rustler::types::atom::Atom::from_str(env, \"ok\").unwrap().to_term(env);"
        )
        .ok();
        writeln!(out, "                    let result_term = result.encode(env);").ok();
        writeln!(
            out,
            "                    rustler::types::tuple::make_tuple(env, &[ok_atom, result_term])"
        )
        .ok();
        writeln!(out, "                }},").ok();
        writeln!(out, "                Err(e) => {{").ok();
        writeln!(
            out,
            "                    let err_atom = rustler::types::atom::Atom::from_str(env, \"error\").unwrap().to_term(env);"
        )
        .ok();
        writeln!(out, "                    let reason = e.to_string().encode(env);").ok();
        writeln!(
            out,
            "                    rustler::types::tuple::make_tuple(env, &[err_atom, reason])"
        )
        .ok();
        writeln!(out, "                }},").ok();
        writeln!(out, "            }}").ok();
        writeln!(out, "        }});").ok();
        writeln!(out, "    }});").ok();
        writeln!(out, "    Ok(())").ok();
        writeln!(out, "}}").ok();
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
    writeln!(out, "#[rustler::nif(schedule = \"DirtyCpu\")]").ok();
    writeln!(out, "pub fn {func_name}({plain_params_str}) -> {ret} {{").ok();
    writeln!(out, "    {plain_body}").ok();
    writeln!(out, "}}").ok();

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
         {options_param}_core.{field_name} = Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path});"
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

    writeln!(out).ok();
    writeln!(
        out,
        "// Async visitor variant: pops visitor from options, builds bridge, spawns thread."
    )
    .ok();
    writeln!(out, "#[rustler::nif]").ok();
    writeln!(
        out,
        "pub fn {func_name}_with_visitor({vis_params_str}) -> Result<(), String> {{"
    )
    .ok();
    writeln!(out, "    let pid = env.pid();").ok();
    writeln!(out, "    let visitor_owned_env = rustler::OwnedEnv::new();").ok();
    writeln!(out, "    let visitor_saved = visitor_owned_env.save(visitor);").ok();
    writeln!(out, "    {clone_stmts}").ok();
    writeln!(out, "    std::thread::spawn(move || {{").ok();
    writeln!(out, "        visitor_owned_env.run(|env| {{").ok();
    writeln!(out, "            let visitor_term = visitor_saved.load(env);").ok();
    writeln!(out, "            {deser_stmts}").ok();
    writeln!(
        out,
        "            // Run conversion, capture result, and send back to BEAM"
    )
    .ok();
    writeln!(
        out,
        "            let conversion_result = match {core_fn_path}({vis_call_args_str}) {{"
    )
    .ok();
    writeln!(out, "                Ok(val) => {{").ok();
    writeln!(
        out,
        "                    let result: ConversionResult = val.into();  // Convert from core::ConversionResult to NIF::ConversionResult"
    )
    .ok();
    writeln!(out, "                    Ok(result)").ok();
    writeln!(out, "                }},").ok();
    writeln!(out, "                Err(e) => Err(e.to_string()),").ok();
    writeln!(out, "            }};").ok();
    writeln!(out, "            let mut result_env = rustler::OwnedEnv::new();").ok();
    writeln!(out, "            let _ = result_env.send_and_clear(&pid, |env| {{").ok();
    writeln!(out, "                match conversion_result {{").ok();
    writeln!(out, "                    Ok(result) => {{").ok();
    writeln!(
        out,
        "                        let ok_atom = rustler::types::atom::Atom::from_str(env, \"ok\").unwrap().to_term(env);"
    )
    .ok();
    writeln!(
        out,
        "                        rustler::types::tuple::make_tuple(env, &[ok_atom, result.encode(env)])"
    )
    .ok();
    writeln!(out, "                    }},").ok();
    writeln!(out, "                    Err(reason) => {{").ok();
    writeln!(
        out,
        "                        let err_atom = rustler::types::atom::Atom::from_str(env, \"error\").unwrap().to_term(env);"
    )
    .ok();
    writeln!(out, "                        let reason_term = reason.encode(env);").ok();
    writeln!(
        out,
        "                        rustler::types::tuple::make_tuple(env, &[err_atom, reason_term])"
    )
    .ok();
    writeln!(out, "                    }},").ok();
    writeln!(out, "                }}").ok();
    writeln!(out, "            }});").ok();
    writeln!(out, "        }});").ok();
    writeln!(out, "    }});").ok();
    writeln!(out, "    Ok(())").ok();
    writeln!(out, "}}").ok();

    out
}
