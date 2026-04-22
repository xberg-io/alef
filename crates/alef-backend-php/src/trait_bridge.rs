//! PHP (ext-php-rs) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to PHP objects via ext-php-rs Zval method calls.

use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec, gen_bridge_all};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;
use std::fmt::Write;

/// PHP-specific trait bridge generator.
/// Implements code generation for bridging PHP objects to Rust traits.
pub struct PhpBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    error_type: error_type.to_string(),
}

impl TraitBridgeGenerator for PhpBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "&mut ext_php_rs::types::ZendObject"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["std::rc::Rc".to_string(), "std::cell::RefCell".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let mut out = String::with_capacity(512);

        // PHP is single-threaded; just call the method directly.
        writeln!(
            out,
            "// SAFETY: PHP objects are single-threaded; method calls are safe within a request."
        )
        .ok();

        let has_args = !method.params.is_empty();
        if has_args {
            writeln!(out, "let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();").ok();
            for p in &method.params {
                writeln!(
                    out,
                    "args.push(ext_php_rs::types::Zval::try_from({}).unwrap_or_default());",
                    p.name
                )
                .ok();
            }
        }

        let args_expr = if has_args {
            "args.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect()"
        } else {
            "vec![]"
        };

        writeln!(out, "let result = self.inner.try_call_method(\"{name}\", {args_expr});").ok();
        writeln!(out, "match result {{").ok();
        writeln!(
            out,
            "    Ok(val) => val.string().unwrap_or_default().parse().unwrap_or_default(),"
        )
        .ok();
        writeln!(out, "    Err(_) => Default::default(),").ok();
        writeln!(out, "}}").ok();

        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let mut out = String::with_capacity(1024);

        writeln!(out, "let inner_obj = self.inner.clone();").ok();
        writeln!(out, "let cached_name = self.cached_name.clone();").ok();

        // Clone params for the blocking closure
        for p in &method.params {
            match &p.ty {
                TypeRef::String => {
                    writeln!(out, "let {} = {}.clone();", p.name, p.name).ok();
                }
                _ => {
                    writeln!(out, "let {} = {};", p.name, p.name).ok();
                }
            }
        }

        writeln!(out).ok();
        writeln!(out, "Box::pin(async move {{").ok();
        writeln!(out, "    // SAFETY: PHP objects are single-threaded within a request.").ok();
        writeln!(out, "    // The block_on executes within the async runtime.").ok();
        writeln!(out, "    let result = WORKER_RUNTIME.block_on(async {{").ok();

        let has_args = !method.params.is_empty();
        if has_args {
            writeln!(out, "        let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();").ok();
            for p in &method.params {
                writeln!(
                    out,
                    "        args.push(ext_php_rs::types::Zval::try_from({}).unwrap_or_default());",
                    p.name
                )
                .ok();
            }
        }

        let args_expr = if has_args {
            "args.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect()"
        } else {
            "vec![]"
        };

        writeln!(
            out,
            "        match inner_obj.try_call_method(\"{name}\", {args_expr}) {{"
        )
        .ok();
        writeln!(
            out,
            "            Ok(val) => val.string().unwrap_or_default().parse().unwrap_or_default(),"
        )
        .ok();
        writeln!(
            out,
            "            Err(e) => Err({}::KreuzbergError::Plugin {{",
            spec.core_import
        )
        .ok();
        writeln!(
            out,
            "                message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),"
        )
        .ok();
        writeln!(out, "                plugin_name: cached_name.clone(),").ok();
        writeln!(out, "            }}),").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }});").ok();
        writeln!(out, "    result").ok();
        writeln!(out, "}})").ok();

        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(out, "    /// Create a new bridge wrapping a PHP object.").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// Validates that the PHP object provides all required methods."
        )
        .ok();
        writeln!(
            out,
            "    pub fn new(php_obj: &mut ext_php_rs::types::ZendObject) -> Self {{"
        )
        .ok();

        // Validate all required methods exist
        for req_method in spec.required_methods() {
            writeln!(
                out,
                "        debug_assert!(php_obj.get_property(\"{}\").is_some(),",
                req_method.name
            )
            .ok();
            writeln!(
                out,
                "            \"PHP object missing required method: {}\");",
                req_method.name
            )
            .ok();
        }

        // Extract and cache name
        writeln!(out, "        let cached_name = php_obj").ok();
        writeln!(out, "            .try_call_method(\"name\", vec![])").ok();
        writeln!(out, "            .ok()").ok();
        writeln!(out, "            .and_then(|v| v.string())").ok();
        writeln!(out, "            .unwrap_or(\"unknown\".into())").ok();
        writeln!(out, "            .to_string();").ok();

        writeln!(out).ok();
        writeln!(out, "        Self {{").ok();
        writeln!(out, "            inner: php_obj,").ok();
        writeln!(out, "            cached_name,").ok();
        writeln!(out, "        }}").ok();
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

        writeln!(out, "#[php_function]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(backend: &mut ext_php_rs::types::ZendObject) -> ext_php_rs::prelude::PhpResult<()> {{"
        )
        .ok();

        // Validate required methods
        let req_methods: Vec<&MethodDef> = spec.required_methods();
        if !req_methods.is_empty() {
            for method in &req_methods {
                writeln!(out, "    if backend.get_property(\"{}\").is_none() {{", method.name).ok();
                writeln!(out, "        return Err(ext_php_rs::exception::PhpException::default(").ok();
                writeln!(
                    out,
                    "            format!(\"Backend missing required method: {{}}\", \"{}\")",
                    method.name
                )
                .ok();
                writeln!(out, "        ).into());").ok();
                writeln!(out, "    }}").ok();
            }
        }

        writeln!(out).ok();
        writeln!(out, "    let wrapper = {wrapper}::new(backend);").ok();
        writeln!(
            out,
            "    let arc: Rc<RefCell<dyn {trait_path}>> = Rc::new(RefCell::new(wrapper));"
        )
        .ok();
        writeln!(out).ok();

        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry;").ok();
        writeln!(
            out,
            "    registry.register(arc).map_err(|e| ext_php_rs::exception::PhpException::default("
        )
        .ok();
        writeln!(out, "        format!(\"Failed to register backend: {{}}\", e)").ok();
        writeln!(out, "    ))?;").ok();
        writeln!(out, "    Ok(())").ok();
        writeln!(out, "}}").ok();

        out
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    api: &ApiSurface,
) -> String {
    // Build type name → rust_path lookup as owned HashMap
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
        let struct_name = format!("Php{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");
        gen_visitor_bridge(trait_type, bridge_cfg, &struct_name, &trait_path, &type_paths)
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = PhpBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Php",
            type_paths,
            error_type: error_type.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    }
}

/// Generate a visitor-style bridge wrapping a PHP `Zval` object reference.
///
/// Every trait method checks if the PHP object has a matching camelCase method,
/// then calls it and maps the PHP return value to `VisitResult`.
fn gen_visitor_bridge(
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(4096);
    let core_crate = trait_path
        .split("::")
        .next()
        .unwrap_or("html_to_markdown_rs")
        .to_string();
    // Helper: convert NodeContext to a PHP array (Zval)
    writeln!(out, "fn nodecontext_to_php_array(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendHashTable> {{").unwrap();
    writeln!(out, "    let mut arr = ext_php_rs::types::ZendHashTable::new();").unwrap();
    writeln!(
        out,
        "    arr.insert(\"nodeType\", ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", ctx.node_type)).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"tagName\", ext_php_rs::types::Zval::try_from(ctx.tag_name.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"depth\", ext_php_rs::types::Zval::try_from(ctx.depth as i64).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"indexInParent\", ext_php_rs::types::Zval::try_from(ctx.index_in_parent as i64).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"isInline\", ext_php_rs::types::Zval::try_from(ctx.is_inline).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    if let Some(ref pt) = ctx.parent_tag {{").unwrap();
    writeln!(
        out,
        "        arr.insert(\"parentTag\", ext_php_rs::types::Zval::try_from(pt.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    let mut attrs = ext_php_rs::types::ZendHashTable::new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(
        out,
        "        attrs.insert(k.as_str(), ext_php_rs::types::Zval::try_from(v.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    let mut attrs_zval = ext_php_rs::types::Zval::new();").unwrap();
    writeln!(out, "    attrs_zval.set_hashtable(attrs);").unwrap();
    writeln!(out, "    arr.insert(\"attributes\", attrs_zval).ok();").unwrap();
    writeln!(out, "    arr").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct — stores a reference to the PHP object.
    // The reference is valid for the duration of the PHP function call that
    // created the bridge, which spans the entire Rust trait method dispatch.
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    php_obj: *mut ext_php_rs::types::ZendObject,").unwrap();
    writeln!(out, "    cached_name: String,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // SAFETY: The raw pointer is only used while the PHP call stack frame is
    // alive. The bridge is consumed before the PHP function returns.
    writeln!(out, "// SAFETY: PHP objects are single-threaded; the bridge is used").unwrap();
    writeln!(out, "// only within a single PHP request, never across threads.").unwrap();
    writeln!(out, "unsafe impl Send for {struct_name} {{}}").unwrap();
    writeln!(out, "unsafe impl Sync for {struct_name} {{}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "impl Clone for {struct_name} {{").unwrap();
    writeln!(out, "    fn clone(&self) -> Self {{").unwrap();
    writeln!(out, "        Self {{").unwrap();
    writeln!(out, "            php_obj: self.php_obj,").unwrap();
    writeln!(out, "            cached_name: self.cached_name.clone(),").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
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

    // Constructor takes &mut ZendObject, which is what ext-php-rs exposes via
    // FromZvalMut. We store the raw pointer; the caller guarantees the object
    // outlives this bridge.
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(
        out,
        "    pub fn new(php_obj: &mut ext_php_rs::types::ZendObject) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        let cached_name = php_obj").unwrap();
    writeln!(out, "            .try_call_method(\"name\", vec![])").unwrap();
    writeln!(out, "            .ok()").unwrap();
    writeln!(out, "            .and_then(|v| v.string())").unwrap();
    writeln!(out, "            .unwrap_or(\"unknown\".into())").unwrap();
    writeln!(out, "            .to_string();").unwrap();
    writeln!(out, "        Self {{ php_obj: php_obj as *mut _, cached_name }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_php(&mut out, method, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    out
}

/// Map a visitor method parameter type to the correct Rust type string.
fn visitor_param_type(ty: &TypeRef, is_ref: bool, optional: bool, tp: &HashMap<String, String>) -> String {
    if optional && matches!(ty, TypeRef::String) && is_ref {
        return "Option<&str>".to_string();
    }
    if is_ref {
        if let TypeRef::Vec(inner) = ty {
            let inner_str = param_type(inner, "", false, tp);
            return format!("&[{inner_str}]");
        }
    }
    param_type(ty, "", is_ref, tp)
}

/// Generate a single visitor method that checks for a camelCase PHP method and calls it.
fn gen_visitor_method_php(out: &mut String, method: &MethodDef, type_paths: &HashMap<String, String>) {
    let name = &method.name;
    let php_name = to_camel_case(name);

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths.get(n).cloned().unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    writeln!(out, "    fn {name}({sig}) -> {ret_ty} {{").unwrap();

    // SAFETY: php_obj pointer is valid for the lifetime of the PHP call frame.
    writeln!(
        out,
        "        // SAFETY: php_obj is a valid ZendObject pointer for the duration of this call."
    )
    .unwrap();
    writeln!(out, "        let php_obj_ref = unsafe {{ &mut *self.php_obj }};").unwrap();

    // Build args array
    let has_args = !method.params.is_empty();
    if has_args {
        writeln!(out, "        let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();").unwrap();
        for p in &method.params {
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    writeln!(
                        out,
                        "        let ctx_arr = nodecontext_to_php_array({}{});",
                        if p.is_ref { "" } else { "&" },
                        p.name
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::convert::IntoZval::into_zval(ctx_arr, false).unwrap_or_default());"
                    )
                    .unwrap();
                    continue;
                }
            }
            // Check optional string ref BEFORE non-optional string, since visitor_param_type
            // returns Option<&str> for optional string ref params.
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                writeln!(
                    out,
                    "        args.push(match {0} {{ Some(s) => ext_php_rs::types::Zval::try_from(s.to_string()).unwrap_or_default(), None => ext_php_rs::types::Zval::new() }});",
                    p.name
                )
                .unwrap();
                continue;
            }
            if matches!(&p.ty, TypeRef::String) {
                if p.is_ref {
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::types::Zval::try_from({}.to_string()).unwrap_or_default());",
                        p.name
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::types::Zval::try_from({}.clone()).unwrap_or_default());",
                        p.name
                    )
                    .unwrap();
                }
                continue;
            }
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
                writeln!(
                    out,
                    "        {{ let mut _zv = ext_php_rs::types::Zval::new(); _zv.set_bool({}); args.push(_zv); }}",
                    p.name
                )
                .unwrap();
                continue;
            }
            // Default: format as string
            writeln!(
                out,
                "        args.push(ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default());",
                p.name
            )
            .unwrap();
        }
    }

    // Call the PHP method via try_call_method which takes Vec<&dyn IntoZvalDyn>.
    // If the method does not exist, try_call_method returns Err(Error::Callable),
    // which we treat as a "no-op, return Continue" (same as the default impl).
    if has_args {
        writeln!(
            out,
            "        let dyn_args: Vec<&dyn ext_php_rs::convert::IntoZvalDyn> = args.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect();"
        )
        .unwrap();
    }
    let args_expr = if has_args { "dyn_args" } else { "vec![]" };
    writeln!(
        out,
        "        let result = php_obj_ref.try_call_method(\"{php_name}\", {args_expr});"
    )
    .unwrap();

    // Parse result — try_call_method returns Result<Zval> (not Result<Option<Zval>>)
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(
        out,
        "                let s = val.string().unwrap_or_default().to_lowercase();"
    )
    .unwrap();
    writeln!(out, "                match s.as_str() {{").unwrap();
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

/// Convert snake_case to camelCase.
fn to_camel_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut capitalize_next = false;
    for (i, c) in name.chars().enumerate() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(c.to_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.extend(c.to_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Map TypeRef to a Rust type string.
fn param_type(ty: &TypeRef, ci: &str, is_ref: bool, tp: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Bytes if is_ref => "&[u8]".into(),
        TypeRef::Bytes => "Vec<u8>".into(),
        TypeRef::String if is_ref => "&str".into(),
        TypeRef::String => "String".into(),
        TypeRef::Path if is_ref => "&std::path::Path".into(),
        TypeRef::Path => "std::path::PathBuf".into(),
        TypeRef::Named(n) => {
            let qualified = tp.get(n).cloned().unwrap_or_else(|| format!("{ci}::{n}"));
            if is_ref { format!("&{qualified}") } else { qualified }
        }
        TypeRef::Vec(inner) => format!("Vec<{}>", param_type(inner, ci, false, tp)),
        TypeRef::Optional(inner) => format!("Option<{}>", param_type(inner, ci, false, tp)),
        TypeRef::Primitive(p) => prim(p).into(),
        TypeRef::Unit => "()".into(),
        TypeRef::Char => "char".into(),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            param_type(k, ci, false, tp),
            param_type(v, ci, false, tp)
        ),
        TypeRef::Json => "serde_json::Value".into(),
        TypeRef::Duration => "std::time::Duration".into(),
    }
}

fn prim(p: &alef_core::ir::PrimitiveType) -> &'static str {
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
}

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub fn find_bridge_param<'a>(
    func: &alef_core::ir::FunctionDef,
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
            if let Some(type_name) = named {
                if bridge.type_alias.as_deref() == Some(type_name) {
                    return Some((idx, bridge));
                }
            }
            if bridge.param_name.as_deref() == Some(param.name.as_str()) {
                return Some((idx, bridge));
            }
        }
    }
    None
}

/// Generate a PHP static method that has one parameter replaced by
/// `Option<ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendObject>>` (a trait bridge).
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Php{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list, hiding bridge params from signature
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            // Bridge param: &mut ZendObject implements FromZvalMut in ext-php-rs 0.15,
            // allowing PHP to pass any object. ZBox<ZendObject> does NOT implement
            // FromZvalMut, so we must use the reference form here.
            let php_obj_ty = "&mut ext_php_rs::types::ZendObject";
            if is_optional {
                sig_parts.push(format!("{}: Option<{php_obj_ty}>", p.name));
            } else {
                sig_parts.push(format!("{}: {php_obj_ty}", p.name));
            }
        } else {
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            let base = mapper.map_type(&p.ty);
            // #[php_class] types (non-opaque Named) only implement FromZvalMut for &mut T,
            // not for owned T — so we must use &mut T in the function signature.
            let ty = match &p.ty {
                TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => {
                    if p.optional || promoted {
                        format!("Option<&mut {base}>")
                    } else {
                        format!("&mut {base}")
                    }
                }
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if !opaque_types.contains(n.as_str()) {
                            format!("Option<&mut {base}>")
                        } else if p.optional || promoted {
                            format!("Option<{base}>")
                        } else {
                            base
                        }
                    } else if p.optional || promoted {
                        format!("Option<{base}>")
                    } else {
                        base
                    }
                }
                _ => {
                    if p.optional || promoted {
                        format!("Option<{base}>")
                    } else {
                        base
                    }
                }
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))";

    // Bridge wrapping code
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name} = {param_name}.map(|v| {{\n        \
             let bridge = {struct_name}::new(v);\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
             }});"
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
                    "let {name}_core: Option<{core_path}> = {name}.map(|v| {{\n        \
                     let json = serde_json::to_string(&v){err_conv}?;\n        \
                     serde_json::from_str(&json){err_conv}\n    \
                     }}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_json = serde_json::to_string(&{name}){err_conv}?;\n    \
                     let {name}_core: {core_path} = serde_json::from_str(&{name}_json){err_conv}?;\n    "
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
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
