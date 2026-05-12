//! WebAssembly (wasm-bindgen) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to JavaScript objects via `js_sys::Reflect` and `js_sys::Function`.

use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    to_camel_case, visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use alef_codegen::generators::trait_bridge::find_bridge_param;

/// Find a bridge config that uses options_field binding and a parameter of the options_type.
/// This complements find_bridge_param which only handles FunctionParam bindings.
pub fn find_options_field_binding<'a>(
    func: &alef_core::ir::FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for bridge in bridges {
        if bridge.bind_via != alef_core::config::BridgeBinding::OptionsField {
            continue;
        }
        if let Some(options_type) = &bridge.options_type {
            for (idx, param) in func.params.iter().enumerate() {
                // Check if param type is Named(options_type) or Optional(Named(options_type))
                let matches = match &param.ty {
                    alef_core::ir::TypeRef::Named(n) => n == options_type,
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            n == options_type
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if matches {
                    return Some((idx, bridge));
                }
            }
        }
    }
    None
}

/// WASM-specific trait bridge generator.
/// Implements code generation for bridging JavaScript objects to Rust traits.
pub struct WasmBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for WasmBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "wasm_bindgen::JsValue"
    }

    /// WASM is single-threaded; trait futures must not require `Send`.
    fn async_trait_is_send(&self) -> bool {
        false
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "wasm_bindgen::prelude::*".to_string(),
            "js_sys".to_string(),
            "std::rc::Rc".to_string(),
            "std::cell::RefCell".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let js_name = to_camel_case(name);
        let has_error = method.error_type.is_some();
        let ret_ty = self.extract_ty(&method.return_type);

        let error_expr = spec.make_error(&format!(
            "format!(\"Method '{{}}' not found on JS object\", \"{name}\")"
        ));
        let error_get_method = spec.make_error(&format!("format!(\"Failed to get method '{{}}'\", \"{name}\")"));
        let error_dyn_into = spec.make_error(&format!("format!(\"Method '{{}}' is not a function\", \"{name}\")"));
        let error_apply = spec.make_error(&format!("format!(\"Failed to call method '{{}}'\", \"{name}\")"));
        let error_string_conv = spec.make_error("\"Expected string return\".to_string()");
        let error_result_conv = spec.make_error("\"Failed to convert result\".to_string()");
        let error_deser = spec.make_error("format!(\"Failed to deserialize result: {}\", e)");

        let params: Vec<String> = method.params.iter().map(|p| build_wasm_arg(p, spec.bridge_config)).collect();

        let return_unit = matches!(method.return_type, TypeRef::Unit);
        let return_string = matches!(method.return_type, TypeRef::String);

        let ctx = minijinja::context! {
            js_name => js_name,
            has_error => has_error,
            error_expr => error_expr,
            error_get_method => error_get_method,
            error_dyn_into => error_dyn_into,
            error_apply => error_apply,
            error_string_conv => error_string_conv,
            error_result_conv => error_result_conv,
            error_deser => error_deser,
            params => params,
            ret_ty => ret_ty,
            return_unit => return_unit,
            return_string => return_string,
        };
        crate::template_env::render("gen_sync_method_body", ctx)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // The #[async_trait] macro will wrap this body in a pinned future.
        // Do NOT add Box::pin(async move { ... }) here — that causes double-boxing.
        let name = &method.name;
        let js_name = to_camel_case(name);
        let has_error = method.error_type.is_some();
        let ret_ty = self.extract_ty(&method.return_type);

        let error_expr = spec.make_error(&format!(
            "format!(\"Method '{{}}' not found on JS object\", \"{name}\")"
        ));
        let error_get_method = spec.make_error(&format!("format!(\"Failed to get method '{{}}'\", \"{name}\")"));
        let error_dyn_into = spec.make_error(&format!("format!(\"Method '{{}}' is not a function\", \"{name}\")"));
        let error_apply = spec.make_error(&format!("format!(\"Failed to call method '{{}}'\", \"{name}\")"));
        let error_promise = spec.make_error(&format!("format!(\"Method '{{}}' did not return a Promise\", \"{name}\")"));
        let error_promise_rejected = spec.make_error("format!(\"Promise rejected: {:?}\", e)");
        let error_string_conv = spec.make_error("\"Expected string return\".to_string()");
        let error_result_conv = spec.make_error("\"Failed to convert result\".to_string()");
        let error_deser = spec.make_error("format!(\"Failed to deserialize result: {}\", e)");

        let params: Vec<String> = method.params.iter().map(|p| build_wasm_arg(p, spec.bridge_config)).collect();

        let return_unit = matches!(method.return_type, TypeRef::Unit);
        let return_string = matches!(method.return_type, TypeRef::String);

        let ctx = minijinja::context! {
            js_name => js_name,
            has_error => has_error,
            error_expr => error_expr,
            error_get_method => error_get_method,
            error_dyn_into => error_dyn_into,
            error_apply => error_apply,
            error_promise => error_promise,
            error_promise_rejected => error_promise_rejected,
            error_string_conv => error_string_conv,
            error_result_conv => error_result_conv,
            error_deser => error_deser,
            params => params,
            ret_ty => ret_ty,
            return_unit => return_unit,
            return_string => return_string,
        };
        crate::template_env::render("gen_async_method_body", ctx)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods: Vec<_> = spec
            .required_methods()
            .iter()
            .map(|m| {
                minijinja::context! {
                    name => m.name.clone(),
                    js_name => to_camel_case(&m.name),
                }
            })
            .collect();
        let ctx = minijinja::context! {
            wrapper => wrapper,
            required_methods => required_methods,
        };
        crate::template_env::render("gen_constructor", ctx)
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        let camel = to_camel_case(unregister_fn);
        let ctx = minijinja::context! {
            camel => camel.clone(),
            unregister_fn => unregister_fn.to_string(),
            host_path => host_path,
        };
        crate::template_env::render("gen_unregistration_fn", ctx)
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let camel = to_camel_case(clear_fn);
        let ctx = minijinja::context! {
            camel => camel.clone(),
            clear_fn => clear_fn.to_string(),
            host_path => host_path,
        };
        crate::template_env::render("gen_clear_fn", ctx)
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

        let camel = to_camel_case(register_fn);
        let required_methods: Vec<_> = spec
            .required_methods()
            .iter()
            .map(|m| {
                minijinja::context! {
                    js_name => to_camel_case(&m.name),
                }
            })
            .collect();
        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        let ctx = minijinja::context! {
            camel => camel,
            register_fn => register_fn.to_string(),
            required_methods => required_methods,
            wrapper => wrapper,
            trait_path => trait_path,
            registry_getter => registry_getter.to_string(),
            extra => extra,
        };
        crate::template_env::render("gen_registration_fn", ctx)
    }
}

impl WasmBridgeGenerator {
    /// Extract the Rust type that corresponds to a TypeRef.
    fn extract_ty(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => self.prim(p).to_string(),
            TypeRef::String => "String".into(),
            TypeRef::Path | TypeRef::Char => "String".into(),
            TypeRef::Bytes => "Vec<u8>".into(),
            TypeRef::Vec(inner) => format!("Vec<{}>", self.extract_ty(inner)),
            TypeRef::Optional(inner) => format!("Option<{}>", self.extract_ty(inner)),
            TypeRef::Named(name) => {
                // Qualify Named types with core crate path if available in type_paths
                self.type_paths
                    .get(name.as_str())
                    .map(|p| p.replace('-', "_"))
                    .unwrap_or_else(|| format!("{}::{}", self.core_import, name))
            }
            TypeRef::Unit => "()".into(),
            TypeRef::Map(k, v) => format!(
                "std::collections::HashMap<{}, {}>",
                self.extract_ty(k),
                self.extract_ty(v)
            ),
            TypeRef::Json => "String".into(),
            TypeRef::Duration => "u64".into(),
        }
    }

    /// Get the Rust string representation of a primitive type.
    fn prim(&self, p: &alef_core::ir::PrimitiveType) -> &'static str {
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
    // Build type name → rust_path lookup, converting to owned HashMap<String, String>
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

    let bridge = if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        let struct_name = format!("Wasm{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");
        gen_visitor_bridge(
            &mut out,
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
        BridgeOutput {
            imports: vec![],
            code: out,
        }
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure for plugin pattern
        let generator = WasmBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Wasm",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    };

    // Gate the entire bridge on wasm32 target. The bridge implements traits that may use
    // `#[cfg_attr(not(target_arch = "wasm32"), async_trait)]`, so on host targets the
    // trait would have async_trait-rewritten signatures while the impl uses bare `async fn`,
    // producing E0195 lifetime mismatches. The bridge is also conceptually wasm-only:
    // it wraps `wasm_bindgen::JsValue` and is invoked from the JS register_* entry point.
    let mod_name = format!("__alef_wasm_bridge_{}", bridge_cfg.trait_name.to_lowercase());
    let gated = format!(
        "#[cfg(target_arch = \"wasm32\")]\nmod {mod_name} {{\n    use super::*;\n\n{body}\n}}\n#[cfg(target_arch = \"wasm32\")]\npub use {mod_name}::*;",
        mod_name = mod_name,
        body = bridge.code,
    );

    BridgeOutput {
        imports: bridge.imports,
        code: gated,
    }
}

/// Generate a visitor-style bridge wrapping a `wasm_bindgen::JsValue` object.
///
/// Every trait method checks if the JS object has a matching camelCase property,
/// then calls it via `js_sys::Reflect` and maps the return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
) {
    let methods: Vec<_> = trait_type
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none())
        .map(|method| {
            let mut method_out = String::new();
            gen_visitor_method_wasm(&mut method_out, method, bridge_cfg, type_paths);
            minijinja::context! {
                code => method_out,
            }
        })
        .collect();

    let ctx = minijinja::context! {
        core_crate => core_crate.to_string(),
        struct_name => struct_name.to_string(),
        trait_path => trait_path.to_string(),
        methods => methods,
    };
    let rendered = crate::template_env::render("gen_visitor_bridge", ctx);
    out.push_str(&rendered);
    out.push('\n');
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_wasm(out: &mut String, method: &MethodDef, bridge_cfg: &TraitBridgeConfig, type_paths: &HashMap<String, String>) {
    let name = &method.name;
    let js_name = to_camel_case(name);

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

    let params: Vec<String> = method.params.iter().map(|p| build_wasm_arg(p, bridge_cfg)).collect();

    let ctx = minijinja::context! {
        name => name.clone(),
        sig => sig,
        ret_ty => ret_ty.clone(),
        js_name => js_name,
        params => params,
    };
    let rendered = crate::template_env::render("gen_visitor_method_wasm", ctx);
    let indented = rendered
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("    {}", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str(&indented);
    out.push('\n');
}

/// Build a single wasm arg expression for a visitor method parameter.
fn build_wasm_arg(p: &alef_core::ir::ParamDef, bridge_cfg: &TraitBridgeConfig) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
            return format!("nodecontext_to_js_value({}{})", if p.is_ref { "" } else { "&" }, p.name);
        }
    }
    // Optional &str must be checked before non-optional &str — otherwise Option<&str>
    // would be passed to JsValue::from_str which expects &str, causing a type error.
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {} {{ Some(s) => wasm_bindgen::JsValue::from_str(s), None => wasm_bindgen::JsValue::null() }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("wasm_bindgen::JsValue::from_str({})", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("wasm_bindgen::JsValue::from_str({}.as_str())", p.name);
    }
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("wasm_bindgen::JsValue::from_bool({})", p.name);
    }
    // Handle byte slices: &[u8] or Vec<u8>
    if matches!(&p.ty, TypeRef::Bytes) {
        let deref = if p.is_ref { "" } else { ".as_slice()" };
        return format!("js_sys::Uint8Array::from({}{}).into()", p.name, deref);
    }
    // For any remaining complex type (Named, Vec, Map, etc.), serialize with serde
    let borrow = if p.is_ref { "" } else { "&" };
    format!(
        "serde_wasm_bindgen::to_value({}{}).unwrap_or(wasm_bindgen::JsValue::NULL)",
        borrow, p.name
    )
}

/// Generate a WASM free function that has one parameter replaced by
/// `wasm_bindgen::JsValue` (a trait bridge).
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    prefix: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Wasm{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<wasm_bindgen::JsValue>", p.name));
            } else {
                sig_parts.push(format!("{}: wasm_bindgen::JsValue", p.name));
            }
        } else {
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

    let err_conv = ".map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))";

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

    // From conversion let bindings for non-bridge Named params.
    // Uses the generated From<WasmType> impl to convert binding types to core types,
    // which avoids requiring serde::Serialize on WASM binding types (many contain JsValue
    // which cannot be serialized).
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
                format!("let {name}_core: Option<{core_path}> = {name}.map({core_path}::from);\n    ")
            } else {
                format!("let {name}_core: {core_path} = {core_path}::from({name});\n    ")
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
            format!("{prefix}{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let js_name = to_camel_case(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
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
    let has_error = func.error_type.is_some();

    let ctx = minijinja::context! {
        func_name => func_name.clone(),
        params_str => params_str,
        ret => ret,
        body => body,
        has_error => has_error,
        js_name_attr => js_name_attr,
    };
    crate::template_env::render("gen_bridge_function", ctx)
}

/// Generate a wrapper function for options-field binding (bridge visitor injection).
/// This function accepts the visitor as a separate parameter, wraps it as a VisitorHandle,
/// injects it into the options struct, and calls the core function.
pub fn gen_options_field_bridge_function(
    func: &alef_core::ir::FunctionDef,
    options_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    prefix: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Wasm{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let options_param = &func.params[options_param_idx];
    let options_name = &options_param.name;

    // Whether the IR already marks the options param as Optional<T>.
    let ir_param_optional = matches!(&options_param.ty, TypeRef::Optional(_));

    // Name of the visitor parameter that will be appended to the function signature.
    let visitor_kwarg = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    let field_name = bridge_cfg.resolved_options_field().unwrap_or(visitor_kwarg);

    // Build parameter list; force the options param to Option<T> if the IR didn't already,
    // and append the visitor parameter.
    let params_str = {
        let mut sig_parts = vec![];
        for (i, p) in func.params.iter().enumerate() {
            let ty = mapper.map_type(&p.ty);
            if i == options_param_idx && !ir_param_optional {
                sig_parts.push(format!("{}: Option<{ty}>", p.name));
            } else {
                sig_parts.push(format!("{}: {ty}", p.name));
            }
        }
        // Append visitor parameter (optional for WASM compatibility)
        sig_parts.push(format!("{visitor_kwarg}: Option<wasm_bindgen::JsValue>"));
        sig_parts.join(", ")
    };

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| wasm_bindgen::JsError::new(&e.to_string()).into())";

    // Generate visitor wrapping (wrap the visitor parameter into a VisitorHandle).
    let visitor_wrap = format!(
        "let {visitor_kwarg}_handle: Option<{handle_path}> = {visitor_kwarg}.map(|v| {{\n    \
         let bridge = {struct_name}::new(v);\n    \
         std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n\
         }});"
    );

    // Generate options conversion with visitor injection.
    let options_convert = format!(
        "let {options_name}_core: Option<{core_import}::ConversionOptions> = {options_name}.map(|mut o| {{\n    \
         o.{field_name} = None;\n    \
         let mut result: {core_import}::ConversionOptions = o.into();\n    \
         result.{field_name} = {visitor_kwarg}_handle.clone();\n    \
         result\n    \
         }}).or_else(|| {{\n    \
         if {visitor_kwarg}_handle.is_some() {{\n    \
         let mut opts = {core_import}::ConversionOptions::default();\n    \
         opts.{field_name} = {visitor_kwarg}_handle.clone();\n    \
         Some(opts)\n    \
         }} else {{\n    \
         None\n    \
         }}\n    \
         }});"
    );

    // Build call args, replacing options param with the _core version
    let call_args: String = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == options_param_idx {
                format!("{options_name}_core")
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                        if p.optional {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("&{}.inner", p.name)
                        }
                    }
                    TypeRef::Named(_) => format!("{}.into()", p.name),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            if opaque_types.contains(n.as_str()) {
                                format!("{}.as_ref().map(|v| &v.inner)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
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
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{prefix}{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    // Build function body with visitor wrapping and options conversion.
    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{visitor_wrap}\n    {options_convert}\n    {core_call}{err_conv}")
        } else {
            format!("{visitor_wrap}\n    {options_convert}\n    {core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{visitor_wrap}\n    {options_convert}\n    {core_call}")
    };

    let func_name = &func.name;
    let has_error = func.error_type.is_some();
    let js_name = to_camel_case(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let ctx = minijinja::context! {
        func_name => func_name.clone(),
        params_str => params_str,
        ret => ret,
        body => body,
        has_error => has_error,
        js_name_attr => js_name_attr,
    };
    crate::template_env::render("gen_bridge_function", ctx)
}
