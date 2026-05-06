//! PHP (ext-php-rs) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to PHP objects via ext-php-rs Zval method calls.

use std::fmt::Write;
use minijinja::context;

use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use alef_codegen::generators::trait_bridge::find_bridge_param;

/// PHP-specific trait bridge generator.
/// Implements code generation for bridging PHP objects to Rust traits.
pub struct PhpBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for PhpBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "*mut ext_php_rs::types::ZendObject"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        let has_args = !method.params.is_empty();
        let args_expr = if has_args {
            let mut args_parts = Vec::new();
            for p in &method.params {
                let arg_expr = match &p.ty {
                    TypeRef::String => format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name),
                    TypeRef::Path => format!(
                        "ext_php_rs::types::Zval::try_from({}.to_string_lossy().to_string()).unwrap_or_default()",
                        p.name
                    ),
                    TypeRef::Bytes => format!(
                        "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                        p.name
                    ),
                    TypeRef::Named(_) => {
                        format!(
                            "ext_php_rs::types::Zval::try_from(serde_json::to_string(&{}).unwrap_or_default()).unwrap_or_default()",
                            p.name
                        )
                    }
                    TypeRef::Primitive(_) => {
                        format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name)
                    }
                    _ => format!(
                        "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                        p.name
                    ),
                };
                args_parts.push(arg_expr);
            }
            let args_array = format!("[{}]", args_parts.join(", "));
            format!(
                "{}.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect()",
                args_array
            )
        } else {
            "vec![]".to_string()
        };

        let is_result_type = method.error_type.is_some();
        let is_unit_return = matches!(method.return_type, TypeRef::Unit);

        crate::template_env::render("sync_method_body.jinja", context! {
            method_name => name,
            args_expr => args_expr,
            is_result_type => is_result_type,
            is_unit_return => is_unit_return,
            core_import => &self.core_import,
        })
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        let string_params: Vec<String> = method
            .params
            .iter()
            .filter(|p| matches!(&p.ty, TypeRef::String))
            .map(|p| p.name.clone())
            .collect();

        let has_args = !method.params.is_empty();
        let args_expr = if has_args {
            let mut args_parts = Vec::new();
            for p in &method.params {
                let arg_expr = match &p.ty {
                    TypeRef::String => format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name),
                    TypeRef::Path => format!(
                        "ext_php_rs::types::Zval::try_from({}.to_string_lossy().to_string()).unwrap_or_default()",
                        p.name
                    ),
                    TypeRef::Bytes => format!(
                        "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                        p.name
                    ),
                    TypeRef::Named(_) => {
                        format!(
                            "ext_php_rs::types::Zval::try_from(serde_json::to_string(&{}).unwrap_or_default()).unwrap_or_default()",
                            p.name
                        )
                    }
                    TypeRef::Primitive(_) => {
                        format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name)
                    }
                    _ => format!(
                        "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                        p.name
                    ),
                };
                args_parts.push(arg_expr);
            }
            let args_array = format!("[{}]", args_parts.join(", "));
            format!(
                "{}.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect()",
                args_array
            )
        } else {
            "vec![]".to_string()
        };

        let is_result_type = method.error_type.is_some();

        crate::template_env::render("async_method_body.jinja", context! {
            method_name => name,
            args_expr => args_expr,
            string_params => string_params,
            is_result_type => is_result_type,
            core_import => &spec.core_import,
        })
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();

        crate::template_env::render("bridge_constructor.jinja", context! {
            wrapper => &wrapper,
        })
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);

        crate::template_env::render("bridge_unregister_fn.jinja", context! {
            unregister_fn => unregister_fn,
            host_path => &host_path,
        })
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, clear_fn);

        crate::template_env::render("bridge_clear_fn.jinja", context! {
            clear_fn => clear_fn,
            host_path => &host_path,
        })
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

        let req_methods: Vec<&MethodDef> = spec.required_methods();
        let required_methods: Vec<minijinja::Value> = req_methods
            .iter()
            .map(|m| {
                minijinja::context! {
                    name => m.name.as_str(),
                }
            })
            .collect();

        let extra_args = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        crate::template_env::render("bridge_registration_fn.jinja", context! {
            register_fn => register_fn,
            required_methods => required_methods,
            wrapper => &wrapper,
            trait_path => &trait_path,
            registry_getter => registry_getter,
            extra_args => &extra_args,
        })
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
        let code = gen_visitor_bridge(trait_type, bridge_cfg, &struct_name, &trait_path, &type_paths);
        BridgeOutput { imports: vec![], code }
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
            error_constructor: error_constructor.to_string(),
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
    out.push_str(&crate::template_env::render("visitor_nodecontext_helper.jinja", context! {
        core_crate => &core_crate,
    }));
    out.push_str("\n");

    // Helper: map a PHP return Zval to VisitResult.
    out.push_str(&crate::template_env::render("visitor_zval_to_visitresult.jinja", context! {
        core_crate => &core_crate,
    }));
    out.push_str("\n");

    // Helper: apply {param_name} template substitution to Custom visit results.
    // Use push_str to avoid writeln! format-string interpretation of braces.
    out.push_str(&format!(
        "fn php_visit_result_with_template(val: &ext_php_rs::types::Zval, tmpl_vars: &[(&str, &str)]) -> {core_crate}::VisitResult {{\n"
    ));
    out.push_str("    let base = php_zval_to_visit_result(val);\n");
    out.push_str(&format!(
        "    if let {core_crate}::VisitResult::Custom(tmpl) = base {{\n"
    ));
    out.push_str("        let mut s = tmpl;\n");
    out.push_str("        for (k, v) in tmpl_vars {\n");
    // Generates: s = s.replace(&format!("{{{}}}", k), v);
    // where "{{{}}}" is a format literal: {{ → {, {} → k, }} → } giving "{k}"
    out.push_str("            s = s.replace(&format!(\"{{{}}}\", k), v);\n");
    out.push_str("        }\n");
    out.push_str(&format!("        {core_crate}::VisitResult::Custom(s)\n"));
    out.push_str("    } else {\n");
    out.push_str("        base\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // Bridge struct — stores a reference to the PHP object.
    out.push_str(&crate::template_env::render("visitor_bridge_struct.jinja", context! {
        struct_name => struct_name,
    }));
    out.push_str("\n");

    // Trait impl
    out.push_str(&format!("impl {trait_path} for {struct_name} {{\n"));
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_php(&mut out, method, type_paths);
    }
    out.push_str("}}\n");
    out.push_str("\n");

    out
}

/// Generate a single visitor method that checks for a snake_case PHP method and calls it.
fn gen_visitor_method_php(out: &mut String, method: &MethodDef, type_paths: &HashMap<String, String>) {
    let name = &method.name;

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

    out.push_str(&format!("    fn {name}({sig}) -> {ret_ty} {{\n"));

    // SAFETY: php_obj pointer is valid for the lifetime of the PHP call frame.
    out.push_str("        // SAFETY: php_obj is a valid ZendObject pointer for the duration of this call.\n");
    out.push_str("        let php_obj_ref = unsafe {{ &mut *self.php_obj }};\n");

    // Build args array
    let has_args = !method.params.is_empty();
    if has_args {
        out.push_str("        let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();\n");
        for p in &method.params {
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    out.push_str(&format!(
                        "        let ctx_arr = nodecontext_to_php_array({}{});\n",
                        if p.is_ref { "" } else { "&" },
                        p.name
                    ));
                    out.push_str("        args.push(ext_php_rs::convert::IntoZval::into_zval(ctx_arr, false).unwrap_or_default());\n");
                    continue;
                }
            }
            // Check optional string ref BEFORE non-optional string, since visitor_param_type
            // returns Option<&str> for optional string ref params.
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                out.push_str(&format!(
                    "        args.push(match {0} {{ Some(s) => ext_php_rs::types::Zval::try_from(s.to_string()).unwrap_or_default(), None => ext_php_rs::types::Zval::new() }});\n",
                    p.name
                ));
                continue;
            }
            if matches!(&p.ty, TypeRef::String) {
                if p.is_ref {
                    out.push_str(&format!(
                        "        args.push(ext_php_rs::types::Zval::try_from({}.to_string()).unwrap_or_default());\n",
                        p.name
                    ));
                } else {
                    out.push_str(&format!(
                        "        args.push(ext_php_rs::types::Zval::try_from({}.clone()).unwrap_or_default());\n",
                        p.name
                    ));
                }
                continue;
            }
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
                out.push_str(&format!(
                    "        {{ let mut _zv = ext_php_rs::types::Zval::new(); _zv.set_bool({}); args.push(_zv); }}\n",
                    p.name
                ));
                continue;
            }
            // Default: format as string
            out.push_str(&format!(
                "        args.push(ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default());\n",
                p.name
            ));
        }
    }

    // Call the PHP method via try_call_method which takes Vec<&dyn IntoZvalDyn>.
    // If the method does not exist, try_call_method returns Err(Error::Callable),
    // which we treat as a "no-op, return Continue" (same as the default impl).
    if has_args {
        out.push_str("        let dyn_args: Vec<&dyn ext_php_rs::convert::IntoZvalDyn> = args.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect();\n");
    }
    let args_expr = if has_args { "dyn_args" } else { "vec![]" };
    out.push_str(&format!("        let result = php_obj_ref.try_call_method(\"{name}\", {args_expr});\n"));

    // Build template vars for {param_name} → value substitution in Custom results.
    // Each non-ctx param gets an owned String so we can take &str references.
    let mut tmpl_var_names: Vec<String> = Vec::new();
    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if n == "NodeContext" {
                continue;
            }
        }
        // Skip Vec/slice params — no Display impl; not useful in templates.
        if matches!(&p.ty, TypeRef::Vec(_)) {
            continue;
        }
        // Strip leading underscore from param name for the template key (e.g. _src → src)
        let key = p.name.strip_prefix('_').unwrap_or(&p.name);
        let owned_var = format!("_{key}_s");
        let expr: String = if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
            format!("{}.map(|s| s.to_string()).unwrap_or_default()", p.name)
        } else if matches!(&p.ty, TypeRef::String) && p.is_ref {
            format!("{}.to_string()", p.name)
        } else if matches!(&p.ty, TypeRef::String) {
            format!("{}.clone()", p.name)
        } else if matches!(&p.ty, TypeRef::Optional(_)) {
            format!("{}.map(|v| v.to_string()).unwrap_or_default()", p.name)
        } else {
            format!("{}.to_string()", p.name)
        };
        writeln!(out, "        let {owned_var}: String = {expr};").unwrap();
        tmpl_var_names.push(format!("(\"{key}\", {owned_var}.as_str())"));
    }
    let tmpl_vars_expr = if tmpl_var_names.is_empty() {
        "&[]".to_string()
    } else {
        format!("&[{}]", tmpl_var_names.join(", "))
    };

    // Parse result — try_call_method returns Result<Zval> (not Result<Option<Zval>>)
    out.push_str("        match result {{\n");
    out.push_str(&format!("            Err(_) => {ret_ty}::Continue,\n"));
    out.push_str(&format!("            Ok(val) => php_visit_result_with_template(&val, {tmpl_vars_expr}),\n"));
    out.push_str("        }}\n");
    out.push_str("    }}\n");
    out.push('\n');
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
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str(&format!("pub fn {func_name}({params_str}) -> {ret} {{\n"));
    out.push_str(&format!("    {body}\n"));
    out.push_str("}}\n");

    out
}
