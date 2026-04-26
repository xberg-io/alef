//! Ruby (Magnus) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Ruby objects via Magnus `respond_to` checks and `funcall`.

pub use alef_codegen::generators::trait_bridge::find_bridge_param;
use alef_codegen::generators::trait_bridge::{
    bridge_param_type as param_type, visitor_param_type, gen_bridge_all, TraitBridgeGenerator,
    TraitBridgeSpec,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;
use std::fmt::Write;

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> String {
    // Skip if explicitly excluded for Ruby
    if bridge_cfg.exclude_languages.contains(&"ruby".to_string()) {
        return String::new();
    }

    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup
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
        // Visitor pattern: use the old visitor bridge code
        let struct_name = format!("Rb{}Bridge", bridge_cfg.trait_name);
        let mut out = String::with_capacity(8192);
        gen_visitor_bridge(
            &mut out,
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
        out
    } else {
        // Plugin pattern: use the shared TraitBridgeGenerator infrastructure.
        // Use the host crate's canonical error type (e.g. KreuzbergError) so the
        // generated `impl Plugin for ...` matches the trait's actual signature.
        let generator = MagnusBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Rb",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let output = gen_bridge_all(&spec, &generator);
        // Emit trait-bridge specific imports as `use ... as _;` at the top of the
        // bridge block so multiple bridges can share trait imports without name
        // collisions on the same module-level identifier.
        let mut prefixed = String::with_capacity(output.imports.len() * 64 + output.code.len());
        let imports_to_emit: Vec<_> = output.imports.iter()
            .filter(|imp| *imp != "magnus::prelude::*")
            .collect();
        // Emit allow attribute before each import group to suppress unused_imports warnings
        for imp in &imports_to_emit {
            prefixed.push_str("#[allow(unused_imports)]\n");
            prefixed.push_str("use ");
            prefixed.push_str(imp);
            prefixed.push_str(" as _;\n");
        }
        prefixed.push_str(&output.code);
        prefixed
    }
}

/// Generate a visitor-style bridge wrapping a Magnus `magnus::Value`.
///
/// Every trait method checks if the Ruby object responds to a snake_case method,
/// then calls it via `funcall` and maps the return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) {
    // Helper: convert NodeContext to a Ruby hash (magnus::RHash)
    writeln!(out, "fn nodecontext_to_rb_hash(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> magnus::RHash {{").unwrap();
    writeln!(out, "    let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").unwrap();
    writeln!(out, "    let h = ruby.hash_new();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"node_type\"), format!(\"{{:?}}\", ctx.node_type)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"tag_name\"), ctx.tag_name.as_str()).ok();"
    )
    .unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"depth\"), ctx.depth as i64).ok();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"index_in_parent\"), ctx.index_in_parent as i64).ok();"
    )
    .unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"is_inline\"), ctx.is_inline).ok();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"parent_tag\"), ctx.parent_tag.as_deref().map(|s| ruby.str_new(s).as_value())).ok();"
    )
    .unwrap();
    writeln!(out, "    let attrs = ruby.hash_new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(out, "        attrs.aset(ruby.str_new(k), ruby.str_new(v)).ok();").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"attributes\"), attrs).ok();").unwrap();
    writeln!(out, "    h").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    rb_obj: magnus::Value,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Manual Debug impl
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

    // Constructor
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(out, "    pub fn new(rb_obj: magnus::Value) -> Self {{").unwrap();
    writeln!(out, "        Self {{ rb_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_magnus(out, method, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate a single visitor method that checks Ruby respond_to and calls via funcall.
fn gen_visitor_method_magnus(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<String, String>,
) {
    let name = &method.name;
    // Ruby uses snake_case method names (same as Rust)

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

    // Check if the Ruby object responds to this method
    writeln!(
        out,
        "        let responds = self.rb_obj.respond_to(\"{name}\", false).unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !responds {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Build the funcall args tuple
    if method.params.is_empty() {
        writeln!(
            out,
            "        let result: Result<magnus::Value, magnus::Error> = self.rb_obj.funcall(\"{name}\", ());"
        )
        .unwrap();
    } else {
        // Build args as a tuple
        let args_exprs: Vec<String> = method.params.iter().map(build_magnus_arg).collect();
        let args_tuple = if args_exprs.len() == 1 {
            format!("({},)", args_exprs[0])
        } else {
            format!("({})", args_exprs.join(", "))
        };
        writeln!(
            out,
            "        let result: Result<magnus::Value, magnus::Error> = self.rb_obj.funcall(\"{name}\", {args_tuple});"
        )
        .unwrap();
    }

    // Parse result
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(out, "                let s: String = val.to_string();").unwrap();
    writeln!(out, "                match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                    \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                    \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                    \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(out, "                    other => {ret_ty}::Custom(other.to_string()),").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single Magnus funcall arg expression for a visitor method parameter.
fn build_magnus_arg(p: &alef_core::ir::ParamDef) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
            return format!("nodecontext_to_rb_hash({}{})", if p.is_ref { "" } else { "&" }, p.name);
        }
    }
    if p.optional && matches!(&p.ty, TypeRef::String) {
        return format!(
            "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; match {} {{ Some(s) => ruby.str_new(s).as_value(), None => ruby.qnil().as_value() }} }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new({}) }}", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new({}.as_str()) }}", p.name);
    }
    // Vec/slice types: convert to Ruby array
    if matches!(&p.ty, TypeRef::Vec(_)) {
        let ruby = "unsafe { magnus::Ruby::get_unchecked() }";
        return format!(
            "{{ let arr = {ruby}.ary_new_capa({name}.len()); for item in {name} {{ let _ = arr.push(item.to_string()); }} arr }}",
            name = p.name,
        );
    }
    // For primitive types, pass directly — Magnus funcall handles i32, i64, u32, bool natively.
    p.name.to_string()
}

// ---------------------------------------------------------------------------
// Plugin-pattern bridge generator (shared TraitBridgeGenerator implementation)
// ---------------------------------------------------------------------------

/// Magnus-specific trait bridge generator.
/// Implements code generation for bridging Ruby objects to Rust traits.
struct MagnusBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    type_paths: HashMap<String, String>,
    /// Canonical error type for the host crate (e.g. `"KreuzbergError"`).
    /// Used to construct Result return types matching the trait's signature.
    error_type: String,
    /// Error constructor template (e.g. `"KreuzbergError::Plugin {{ message: {msg}, plugin_name: String::new() }}"`).
    error_constructor: String,
}

impl MagnusBridgeGenerator {
    /// Build the fully-qualified error path (`{core_import}::{error_type}` unless already qualified).
    fn error_path(&self) -> String {
        if self.error_type.contains("::") || self.error_type.contains('<') {
            self.error_type.clone()
        } else {
            format!("{}::{}", self.core_import, self.error_type)
        }
    }

    /// Build an error construction expression from a message expression.
    fn make_error(&self, msg_expr: &str) -> String {
        self.error_constructor.replace("{msg}", msg_expr)
    }
}

impl TraitBridgeGenerator for MagnusBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "magnus::value::Opaque<magnus::Value>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        // Keep this list small. `Arc` is already imported globally at file scope by
        // the magnus gen_bindings pipeline. Trait-only imports are emitted as `use ... as _`
        // by `gen_trait_bridge` so multiple bridges can co-exist without name collisions.
        vec![
            "magnus::value::InnerValue".to_string(),
            "magnus::TryConvert".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);
        let mut out = String::with_capacity(512);

        // Magnus requires holding the GVL. Caller is on a Ruby thread because the bridge
        // is registered from one. catch_unwind guards against Ruby exceptions panicking.
        writeln!(out, "// SAFETY: bridge methods are only invoked from threads holding the GVL").ok();
        writeln!(out, "let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
        writeln!(out, "let value = self.inner.get_inner_with(&ruby);").ok();

        // Build funcall args
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| self.ruby_arg_expr(p))
            .collect();

        let call = if args.is_empty() {
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", ())")
        } else {
            let args_tuple = if args.len() == 1 {
                format!("({},)", args[0])
            } else {
                format!("({})", args.join(", "))
            };
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", {args_tuple})")
        };

        writeln!(out, "let val: magnus::Value = match {call} {{").ok();
        writeln!(out, "    Ok(v) => v,").ok();
        writeln!(out, "    Err(e) => {{").ok();
        if has_error {
            let err_expr = self.make_error(&format!(
                "format!(\"Ruby method '{name}' failed: {{}}\", e)"
            ));
            writeln!(out, "        return Err({err_expr});").ok();
        } else {
            writeln!(out, "        let _ = e;").ok();
            writeln!(out, "        return Default::default();").ok();
        }
        writeln!(out, "    }}").ok();
        writeln!(out, "}};").ok();

        if is_unit {
            writeln!(out, "let _ = val;").ok();
            if has_error {
                writeln!(out, "Ok(())").ok();
            }
        } else {
            self.write_return_conversion(&mut out, method, has_error);
        }

        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);
        let mut out = String::with_capacity(1024);

        // async_trait wraps the body in `Pin<Box<dyn Future + Send>>`, so anything
        // captured into the future must be Send. magnus::Value is !Send, so we
        // capture only the Send wrappers (Opaque<Value>, owned param copies),
        // then dereference inside spawn_blocking which holds GVL on the worker thread.

        writeln!(out, "let inner = self.inner;").ok();
        writeln!(out, "let cached_name = self.cached_name.clone();").ok();
        // cached_name is referenced both inside the spawn_blocking closure and after
        // the await for the JoinError fallback, so clone once for each consumer.
        writeln!(out, "let cached_name_for_blocking = cached_name.clone();").ok();
        let _ = name;

        // Clone params into Send-safe owned copies for the blocking task.
        for p in &method.params {
            match (&p.ty, p.is_ref) {
                (TypeRef::String, true) => {
                    writeln!(out, "let {0}_owned = {0}.to_string();", p.name).ok();
                }
                (TypeRef::Bytes, true) => {
                    writeln!(out, "let {0}_owned = {0}.to_vec();", p.name).ok();
                }
                (TypeRef::Path, true) => {
                    writeln!(out, "let {0}_owned = {0}.to_path_buf();", p.name).ok();
                }
                _ => {
                    writeln!(out, "let {0}_owned = {0}.clone();", p.name).ok();
                }
            }
        }
        writeln!(out).ok();

        let return_type_rust = if is_unit {
            "()".to_string()
        } else {
            self.return_rust_type(&method.return_type)
        };
        let err_path = self.error_path();
        let result_ty = if has_error {
            format!("std::result::Result<{return_type_rust}, {err_path}>")
        } else {
            return_type_rust.clone()
        };

        // The shared generator emits `async fn ...` with `#[async_trait]`; the body is
        // the function body. async_trait wraps it into `Pin<Box<dyn Future + Send>>`.
        writeln!(
            out,
            "let join: std::result::Result<{result_ty}, tokio::task::JoinError> ="
        )
        .ok();
        writeln!(
            out,
            "        tokio::task::spawn_blocking(move || -> {result_ty} {{"
        )
        .ok();
        writeln!(out, "            // SAFETY: spawn_blocking thread acquires GVL via Ruby::get_unchecked").ok();
        writeln!(out, "            let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
        writeln!(out, "            let value = inner.get_inner_with(&ruby);").ok();

        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_name = if matches!(&p.ty, TypeRef::String) && p.is_ref {
                    format!("{}_owned.as_str()", p.name)
                } else {
                    format!("{}_owned", p.name)
                };
                self.ruby_arg_expr_custom(&p.ty, &param_name)
            })
            .collect();

        let call = if args.is_empty() {
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", ())")
        } else {
            let args_tuple = if args.len() == 1 {
                format!("({},)", args[0])
            } else {
                format!("({})", args.join(", "))
            };
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", {args_tuple})")
        };

        writeln!(out, "            let val: magnus::Value = match {call} {{").ok();
        writeln!(out, "                Ok(v) => v,").ok();
        writeln!(out, "                Err(e) => {{").ok();
        if has_error {
            let err_expr = self.make_error(&format!(
                "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name_for_blocking, e)"
            ));
            writeln!(out, "                    return Err({err_expr});").ok();
        } else {
            writeln!(out, "                    let _ = e;").ok();
            writeln!(out, "                    return Default::default();").ok();
        }
        writeln!(out, "                }}").ok();
        writeln!(out, "            }};").ok();

        if is_unit {
            writeln!(out, "            let _ = val;").ok();
            if has_error {
                writeln!(out, "            Ok(())").ok();
            }
        } else {
            self.write_async_return_conversion(&mut out, method, has_error);
        }

        writeln!(out, "        }}).await;").ok();
        writeln!(out).ok();
        writeln!(out, "match join {{").ok();
        writeln!(out, "    Ok(v) => v,").ok();
        if has_error {
            // Need to escape braces in the format string: in the generated code, we want
            // format!("...", ...) which means we need to pass the literal string with single braces.
            // To include single braces in the final generated code, we use double braces here,
            // which will be processed by writeln! to become single braces.
            let msg_expr = format!(
                "format!(\"spawn_blocking failed for '{{}}': {{}}\", cached_name, e)"
            );
            let err_expr = self.make_error(&msg_expr);
            writeln!(out, "    Err(e) => Err({err_expr}),").ok();
        } else {
            writeln!(out, "    Err(_) => Default::default(),").ok();
        }
        writeln!(out, "}}").ok();
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(
            out,
            "    /// Create a new bridge wrapping a Ruby object."
        )
        .ok();
        writeln!(
            out,
            "    /// Validates that the Ruby object responds to all required methods."
        )
        .ok();
        writeln!(
            out,
            "    pub fn new(rb_obj: magnus::Value, name: String) -> Result<Self, magnus::Error> {{"
        )
        .ok();

        // Validate required methods respond_to?
        for req_method in spec.required_methods() {
            writeln!(
                out,
                "        if !rb_obj.respond_to(\"{}\", false).unwrap_or(false) {{",
                req_method.name
            )
            .ok();
            let ruby = "unsafe { magnus::Ruby::get_unchecked() }";
            writeln!(out, "            let ruby = {ruby};").ok();
            writeln!(
                out,
                "            return Err(magnus::Error::new("
            )
            .ok();
            writeln!(
                out,
                "                ruby.exception_runtime_error(),"
            )
            .ok();
            writeln!(
                out,
                "                format!(\"Ruby object missing required method: {{}}\", \"{}\"),",
                req_method.name
            )
            .ok();
            writeln!(out, "            ));").ok();
            writeln!(out, "        }}").ok();
        }

        writeln!(out).ok();
        writeln!(out, "        Ok(Self {{").ok();
        writeln!(out, "            inner: magnus::value::Opaque::from(rb_obj),").ok();
        writeln!(out, "            cached_name: name,").ok();
        writeln!(out, "        }})").ok();
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
        let core_import = spec.core_import;

        let mut out = String::with_capacity(1024);

        // Free function the main #[magnus::init] block can register via
        // `module.define_module_function("{register_fn}", function!({register_fn}, 2))?`.
        // We do NOT emit our own #[magnus::init] — there is exactly one per cdylib and
        // the gen_bindings init owns it.
        writeln!(
            out,
            "pub fn {register_fn}(rb_obj: magnus::Value, name: String) -> Result<(), magnus::Error> {{"
        )
        .ok();

        // Validate required methods exist on the Ruby object
        let req_methods: Vec<_> = spec.required_methods();
        if !req_methods.is_empty() {
            writeln!(
                out,
                "    let required_methods = [{}];",
                req_methods
                    .iter()
                    .map(|m| format!("\"{}\"", m.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .ok();
            writeln!(out, "    for method in &required_methods {{").ok();
            writeln!(out, "        if !rb_obj.respond_to(*method, false).unwrap_or(false) {{").ok();
            writeln!(out, "            let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
            writeln!(out, "            return Err(magnus::Error::new(").ok();
            writeln!(out, "                ruby.exception_runtime_error(),").ok();
            writeln!(out, "                format!(\"Backend missing required method: {{}}\", method),").ok();
            writeln!(out, "            ));").ok();
            writeln!(out, "        }}").ok();
            writeln!(out, "    }}").ok();
            writeln!(out).ok();
        }

        writeln!(out, "    let wrapper = {wrapper}::new(rb_obj, name)?;").ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(wrapper);").ok();

        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();
        writeln!(
            out,
            "    {registry_getter}().write().register(arc{extra}).map_err(|e| {{"
        )
        .ok();
        writeln!(out, "        let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
        writeln!(out, "        magnus::Error::new(ruby.exception_runtime_error(), format!(\"register failed: {{}}\", e))").ok();
        writeln!(out, "    }})?;").ok();
        writeln!(out, "    Ok(())").ok();
        writeln!(out, "}}").ok();

        let _ = core_import;
        out
    }
}

impl MagnusBridgeGenerator {
    /// The fully-qualified Rust return type as it appears in the trait method
    /// signature — uses `core_import::Foo` for Named types.
    fn return_rust_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => {
                use alef_core::ir::PrimitiveType::*;
                match p {
                    Bool => "bool",
                    U8 => "u8",
                    U16 => "u16",
                    U32 => "u32",
                    U64 => "u64",
                    I8 => "i8",
                    I16 => "i16",
                    I32 => "i32",
                    I64 => "i64",
                    F32 => "f32",
                    F64 => "f64",
                    Usize => "usize",
                    Isize => "isize",
                }
                .to_string()
            }
            TypeRef::String => "String".to_string(),
            TypeRef::Bytes => "Vec<u8>".to_string(),
            TypeRef::Vec(inner) => format!("Vec<{}>", self.return_rust_type(inner)),
            TypeRef::Optional(inner) => format!("Option<{}>", self.return_rust_type(inner)),
            TypeRef::Named(name) => self
                .type_paths
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(|| format!("{}::{}", self.core_import, name)),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Map(k, v) => format!(
                "std::collections::HashMap<{}, {}>",
                self.return_rust_type(k),
                self.return_rust_type(v)
            ),
            TypeRef::Json => "serde_json::Value".to_string(),
            TypeRef::Duration => "std::time::Duration".to_string(),
            TypeRef::Char => "char".to_string(),
            TypeRef::Path => "std::path::PathBuf".to_string(),
        }
    }

    /// Whether converting `ty` from a Ruby `magnus::Value` requires a JSON round-trip.
    /// True for any Named type or composite that contains a Named type — magnus's
    /// `TryConvert` is only implemented for primitives, String, Vec<T: TryConvert>,
    /// HashMap with TryConvert keys/values, and a few container types.
    fn needs_json_marshalling(&self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(_) | TypeRef::Json => true,
            TypeRef::Vec(inner) | TypeRef::Optional(inner) => self.needs_json_marshalling(inner),
            TypeRef::Map(k, v) => self.needs_json_marshalling(k) || self.needs_json_marshalling(v),
            _ => false,
        }
    }

    /// Emit code that converts the Ruby `val` (in scope) into the Rust return type
    /// and either returns it (if has_error: false) or wraps it in `Ok(...)` (if has_error: true).
    /// For sync bodies — no leading whitespace.
    fn write_return_conversion(&self, out: &mut String, method: &MethodDef, has_error: bool) {
        let rust_ty = self.return_rust_type(&method.return_type);
        if self.needs_json_marshalling(&method.return_type) {
            // Ruby callback should return either a Hash/Array (we'll JSON.dump it)
            // or a JSON String we parse directly. Try string first, fall back to to_json.
            writeln!(out, "let json_str: String = if let Ok(s) = <String as magnus::TryConvert>::try_convert(val) {{").ok();
            writeln!(out, "    s").ok();
            writeln!(out, "}} else {{").ok();
            writeln!(out, "    match val.funcall::<_, _, String>(\"to_json\", ()) {{").ok();
            writeln!(out, "        Ok(s) => s,").ok();
            writeln!(out, "        Err(e) => {{").ok();
            if has_error {
                let err_expr = self.make_error(&format!(
                    "format!(\"Ruby method '{}' returned non-JSON value: {{}}\", e)",
                    method.name
                ));
                writeln!(out, "            return Err({err_expr});").ok();
            } else {
                writeln!(out, "            let _ = e;").ok();
                writeln!(out, "            return Default::default();").ok();
            }
            writeln!(out, "        }}").ok();
            writeln!(out, "    }}").ok();
            writeln!(out, "}};").ok();
            if has_error {
                let err_expr = self.make_error(&format!(
                    "format!(\"Failed to deserialize Ruby '{}' return value: {{}}\", e)",
                    method.name
                ));
                writeln!(out, "serde_json::from_str::<{rust_ty}>(&json_str)").ok();
                writeln!(out, "    .map_err(|e| {err_expr})").ok();
            } else {
                writeln!(out, "serde_json::from_str::<{rust_ty}>(&json_str).unwrap_or_default()").ok();
            }
        } else {
            // Direct TryConvert path for primitives, String, etc.
            if has_error {
                let err_expr = self.make_error(&format!(
                    "format!(\"Failed to convert Ruby '{}' return value: {{}}\", e)",
                    method.name
                ));
                writeln!(out, "<{rust_ty} as magnus::TryConvert>::try_convert(val)").ok();
                writeln!(out, "    .map_err(|e| {err_expr})").ok();
            } else {
                writeln!(
                    out,
                    "<{rust_ty} as magnus::TryConvert>::try_convert(val).unwrap_or_default()"
                )
                .ok();
            }
        }
    }

    /// Same as `write_return_conversion` but indented for use inside spawn_blocking closure.
    fn write_async_return_conversion(&self, out: &mut String, method: &MethodDef, has_error: bool) {
        let rust_ty = self.return_rust_type(&method.return_type);
        if self.needs_json_marshalling(&method.return_type) {
            writeln!(out, "            let json_str: String = if let Ok(s) = <String as magnus::TryConvert>::try_convert(val) {{").ok();
            writeln!(out, "                s").ok();
            writeln!(out, "            }} else {{").ok();
            writeln!(out, "                match val.funcall::<_, _, String>(\"to_json\", ()) {{").ok();
            writeln!(out, "                    Ok(s) => s,").ok();
            writeln!(out, "                    Err(e) => {{").ok();
            if has_error {
                let err_expr = self.make_error(&format!(
                    "format!(\"Ruby method '{}' returned non-JSON value: {{}}\", e)",
                    method.name
                ));
                writeln!(out, "                        return Err({err_expr});").ok();
            } else {
                writeln!(out, "                        let _ = e;").ok();
                writeln!(out, "                        return Default::default();").ok();
            }
            writeln!(out, "                    }}").ok();
            writeln!(out, "                }}").ok();
            writeln!(out, "            }};").ok();
            if has_error {
                let err_expr = self.make_error(&format!(
                    "format!(\"Failed to deserialize Ruby '{}' return value: {{}}\", e)",
                    method.name
                ));
                writeln!(out, "            serde_json::from_str::<{rust_ty}>(&json_str)").ok();
                writeln!(out, "                .map_err(|e| {err_expr})").ok();
            } else {
                writeln!(
                    out,
                    "            serde_json::from_str::<{rust_ty}>(&json_str).unwrap_or_default()"
                )
                .ok();
            }
        } else if has_error {
            let err_expr = self.make_error(&format!(
                "format!(\"Failed to convert Ruby '{}' return value: {{}}\", e)",
                method.name
            ));
            writeln!(out, "            <{rust_ty} as magnus::TryConvert>::try_convert(val)").ok();
            writeln!(out, "                .map_err(|e| {err_expr})").ok();
        } else {
            writeln!(
                out,
                "            <{rust_ty} as magnus::TryConvert>::try_convert(val).unwrap_or_default()"
            )
            .ok();
        }
    }

    /// Build a Ruby arg expression for funcall given a Rust parameter.
    fn ruby_arg_expr(&self, p: &alef_core::ir::ParamDef) -> String {
        self.ruby_arg_expr_custom(&p.ty, &p.name)
    }

    /// Build a Ruby arg expression for funcall given a type and variable name.
    /// Wraps `var` in deref/borrow as needed so the expression always type-checks
    /// regardless of whether `var` is owned (`String`, `Vec<u8>`, ...) or borrowed.
    fn ruby_arg_expr_custom(&self, ty: &TypeRef, var: &str) -> String {
        match ty {
            // str_new takes Into<&str>; AsRef<str> covers both String and &str.
            TypeRef::String => format!("{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(AsRef::<str>::as_ref(&{var})).as_value() }}"),
            // String::from_utf8_lossy needs &[u8]; AsRef<[u8]> covers both Vec<u8> and &[u8].
            TypeRef::Bytes => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(String::from_utf8_lossy(AsRef::<[u8]>::as_ref(&{var})).as_ref()).as_value() }}"
            ),
            // serde_json::to_string takes &T; the macro `&{var}` is fine for both owned and ref.
            TypeRef::Named(_) | TypeRef::Json => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value()) }}"
            ),
            TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_) => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value()) }}"
            ),
            // Both PathBuf (owned) and &Path (borrowed) coerce via AsRef<Path>; pin
            // the AsRef target type explicitly so type inference doesn't fail.
            TypeRef::Path => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(<_ as AsRef<std::path::Path>>::as_ref(&{var}).to_string_lossy().as_ref()).as_value() }}"
            ),
            _ => var.to_string(),
        }
    }
}

/// Generate a Magnus free function that has one parameter replaced by `magnus::Value` (a trait
/// bridge). The bridge is constructed before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &std::collections::HashSet<&str>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Rb{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<magnus::Value>", p.name));
            } else {
                sig_parts.push(format!("{}: magnus::Value", p.name));
            }
        } else {
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            // default_types are passed as JSON strings at the NIF boundary
            let is_default_type = match &p.ty {
                TypeRef::Named(n) => default_types.contains(n.as_str()),
                TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), TypeRef::Named(n) if default_types.contains(n.as_str()))
                }
                _ => false,
            };
            let ty = if is_default_type {
                if p.optional || promoted {
                    "Option<String>".to_string()
                } else {
                    "String".to_string()
                }
            } else if p.optional || promoted {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    // Magnus functions with errors always return Result
    let has_error = func.error_type.is_some();
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), e.to_string()))";

    // Bridge wrapping code
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_nil() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name});\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
             }};"
        )
    };

    // Serde let bindings for non-bridge Named params
    let serde_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == bridge_param_idx {
                return false;
            }
            let named = match &p.ty {
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
            named.is_some_and(|n| !opaque_types.contains(n))
        })
        .map(|(_, p)| {
            let name = &p.name;
            let core_path = format!(
                "{core_import}::{}",
                match &p.ty {
                    TypeRef::Named(n) => n.clone(),
                    TypeRef::Optional(inner) =>
                        if let TypeRef::Named(n) = inner.as_ref() {
                            n.clone()
                        } else {
                            String::new()
                        },
                    _ => String::new(),
                }
            );
            if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                format!(
                    "let {name}_core: Option<{core_path}> = {name}.as_deref().filter(|s| *s != \"nil\").map(|s| serde_json::from_str(s){err_conv}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_core: {core_path} = serde_json::from_str(&{name}){err_conv}?;\n    "
                )
            }
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
            match &p.ty {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
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
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    writeln!(out, "#[allow(unused_variables)]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
