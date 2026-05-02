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
use std::fmt::Write;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use alef_codegen::generators::trait_bridge::find_bridge_param;

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
        let mut out = String::with_capacity(512);

        writeln!(out, "let key = wasm_bindgen::JsValue::from_str(\"{js_name}\");").ok();
        writeln!(
            out,
            "let has_method = js_sys::Reflect::has(&self.inner, &key).unwrap_or(false);"
        )
        .ok();
        writeln!(out, "if !has_method {{").ok();
        if has_error {
            let err_expr = spec.make_error(&format!(
                "format!(\"Method '{{}}' not found on JS object\", \"{name}\")"
            ));
            writeln!(out, "    return Err({err_expr});").ok();
        } else {
            writeln!(out, "    return Default::default();").ok();
        }
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        if has_error {
            writeln!(out, "let func_val = js_sys::Reflect::get(&self.inner, &key)").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Failed to get method '{{}}'\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let func_val = match js_sys::Reflect::get(&self.inner, &key) {{").ok();
            writeln!(out, "    Ok(f) => f,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        if has_error {
            writeln!(out, "let func: js_sys::Function = func_val.dyn_into()").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Method '{{}}' is not a function\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let func: js_sys::Function = match func_val.dyn_into() {{").ok();
            writeln!(out, "    Ok(f) => f,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        // Build args array
        writeln!(out, "let args = js_sys::Array::new();").ok();
        for p in &method.params {
            let arg_val = build_wasm_arg(p);
            writeln!(out, "args.push(&{arg_val});").ok();
        }
        writeln!(out).ok();

        // Call the function
        if has_error {
            writeln!(out, "let result = func.apply(&self.inner, &args)").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Failed to call method '{{}}'\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let result = match func.apply(&self.inner, &args) {{").ok();
            writeln!(out, "    Ok(r) => r,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        // Convert result
        let ret_ty = self.extract_ty(&method.return_type);
        if matches!(method.return_type, TypeRef::Unit) {
            if has_error {
                writeln!(out, "Ok(())").ok();
            } else {
                writeln!(out, "()").ok();
            }
        } else if matches!(method.return_type, TypeRef::String) {
            if has_error {
                writeln!(out, "result.as_string()").ok();
                writeln!(
                    out,
                    "    .ok_or_else(|| {})",
                    spec.make_error("\"Expected string return\".to_string()")
                )
                .ok();
            } else {
                writeln!(out, "result.as_string().unwrap_or_default()").ok();
            }
        } else {
            if has_error {
                writeln!(out, "// Convert JS result to {ret_ty}").ok();
                writeln!(out, "result.as_string()").ok();
                writeln!(
                    out,
                    "    .ok_or_else(|| {})",
                    spec.make_error("\"Failed to convert result\".to_string()")
                )
                .ok();
                writeln!(
                    out,
                    "    .and_then(|s| serde_json::from_str::<{ret_ty}>(&s).map_err(|e| {}))",
                    spec.make_error("format!(\"Failed to deserialize result: {}\", e)")
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "result.as_string().and_then(|s| serde_json::from_str::<{ret_ty}>(&s).ok()).unwrap_or_default()"
                )
                .ok();
            }
        }
        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let js_name = to_camel_case(name);
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(1024);

        // WASM is single-threaded, so we just call the function synchronously.
        // The #[async_trait] macro will wrap this body in a pinned future.
        // Do NOT add Box::pin(async move { ... }) here — that causes double-boxing.

        writeln!(out, "let key = wasm_bindgen::JsValue::from_str(\"{js_name}\");").ok();
        writeln!(
            out,
            "let has_method = js_sys::Reflect::has(&self.inner, &key).unwrap_or(false);"
        )
        .ok();
        writeln!(out, "if !has_method {{").ok();
        if has_error {
            let err_expr = spec.make_error(&format!(
                "format!(\"Method '{{}}' not found on JS object\", \"{name}\")"
            ));
            writeln!(out, "    return Err({err_expr});").ok();
        } else {
            writeln!(out, "    return Default::default();").ok();
        }
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        if has_error {
            writeln!(out, "let func_val = js_sys::Reflect::get(&self.inner, &key)").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Failed to get method '{{}}'\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let func_val = match js_sys::Reflect::get(&self.inner, &key) {{").ok();
            writeln!(out, "    Ok(f) => f,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        if has_error {
            writeln!(out, "let func: js_sys::Function = func_val.dyn_into()").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Method '{{}}' is not a function\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let func: js_sys::Function = match func_val.dyn_into() {{").ok();
            writeln!(out, "    Ok(f) => f,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        writeln!(out, "let args = js_sys::Array::new();").ok();
        for p in &method.params {
            let arg_val = build_wasm_arg(p);
            writeln!(out, "args.push(&{arg_val});").ok();
        }
        writeln!(out).ok();

        if has_error {
            writeln!(out, "let result = func.apply(&self.inner, &args)").ok();
            writeln!(
                out,
                "    .map_err(|_| {})?;",
                spec.make_error(&format!("format!(\"Failed to call method '{{}}'\", \"{name}\")"))
            )
            .ok();
        } else {
            writeln!(out, "let result = match func.apply(&self.inner, &args) {{").ok();
            writeln!(out, "    Ok(r) => r,").ok();
            writeln!(out, "    Err(_) => return Default::default(),").ok();
            writeln!(out, "}};").ok();
        }
        writeln!(out).ok();

        // Convert result
        if matches!(method.return_type, TypeRef::Unit) {
            if has_error {
                writeln!(out, "Ok(())").ok();
            } else {
                writeln!(out, "()").ok();
            }
        } else if matches!(method.return_type, TypeRef::String) {
            if has_error {
                writeln!(out, "result.as_string()").ok();
                writeln!(
                    out,
                    "    .ok_or_else(|| {})",
                    spec.make_error("\"Expected string return\".to_string()")
                )
                .ok();
            } else {
                writeln!(out, "result.as_string().unwrap_or_default()").ok();
            }
        } else {
            let ret_ty = self.extract_ty(&method.return_type);
            if has_error {
                writeln!(out, "result.as_string()").ok();
                writeln!(
                    out,
                    "    .ok_or_else(|| {})",
                    spec.make_error("\"Failed to convert result\".to_string()")
                )
                .ok();
                writeln!(
                    out,
                    "    .and_then(|s| serde_json::from_str::<{ret_ty}>(&s).map_err(|e| {}))",
                    spec.make_error("format!(\"Failed to deserialize result: {}\", e)")
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "result.as_string().and_then(|s| serde_json::from_str::<{ret_ty}>(&s).ok()).unwrap_or_default()"
                )
                .ok();
            }
        }
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(out, "    /// Create a new bridge wrapping a JS object.").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// Validates that the JS object provides all required methods."
        )
        .ok();
        writeln!(
            out,
            "    pub fn new(js_obj: wasm_bindgen::JsValue) -> Result<Self, String> {{"
        )
        .ok();

        // Validate all required methods exist
        for req_method in spec.required_methods() {
            let js_name = to_camel_case(&req_method.name);
            writeln!(
                out,
                "        if !js_sys::Reflect::has(&js_obj, &wasm_bindgen::JsValue::from_str(\"{js_name}\")).unwrap_or(false) {{"
            )
            .ok();
            writeln!(
                out,
                "            return Err(format!(\"JS object missing required method: {{}}\", \"{}\"));",
                req_method.name
            )
            .ok();
            writeln!(out, "        }}").ok();
        }

        writeln!(out).ok();
        writeln!(out, "        Ok(Self {{").ok();
        writeln!(out, "            inner: js_obj,").ok();
        writeln!(out, "            cached_name: \"wasm_bridge\".to_string(),").ok();
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

        let mut out = String::with_capacity(1024);

        writeln!(out, "#[wasm_bindgen]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(backend: wasm_bindgen::JsValue) -> Result<(), wasm_bindgen::JsValue> {{"
        )
        .ok();

        // Validate required methods
        let req_methods: Vec<&MethodDef> = spec.required_methods();
        if !req_methods.is_empty() {
            writeln!(out, "    let required_methods = vec![").ok();
            for m in &req_methods {
                writeln!(out, "        \"{}\",", to_camel_case(&m.name)).ok();
            }
            writeln!(out, "    ];").ok();
            writeln!(out).ok();
            writeln!(out, "    for method_name in required_methods {{").ok();
            writeln!(
                out,
                "        if !js_sys::Reflect::has(&backend, &wasm_bindgen::JsValue::from_str(method_name)).unwrap_or(false) {{"
            )
            .ok();
            writeln!(
                out,
                "            return Err(wasm_bindgen::JsValue::from_str(&format!(\"Backend missing required method: {{}}\", method_name)));"
            )
            .ok();
            writeln!(out, "        }}").ok();
            writeln!(out, "    }}").ok();
        }

        writeln!(out).ok();
        writeln!(out, "    let wrapper = {wrapper}::new(backend)").ok();
        writeln!(out, "        .map_err(|e| wasm_bindgen::JsValue::from_str(&e))?;").ok();
        writeln!(
            out,
            "    let arc: std::sync::Arc<dyn {trait_path}> = std::sync::Arc::new(wrapper);"
        )
        .ok();
        writeln!(out).ok();

        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write();").ok();
        writeln!(out, "    registry.register(arc{extra})").ok();
        writeln!(
            out,
            "        .map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))"
        )
        .ok();
        writeln!(out, "}}").ok();
        out
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
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
) {
    // Helper: convert NodeContext to a JS object via js_sys::Object
    writeln!(out, "fn nodecontext_to_js_value(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> wasm_bindgen::JsValue {{").unwrap();
    writeln!(out, "    let obj = js_sys::Object::new();").unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"nodeType\"), &wasm_bindgen::JsValue::from_str(&format!(\"{{:?}}\", ctx.node_type))).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"tagName\"), &wasm_bindgen::JsValue::from_str(&ctx.tag_name)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"depth\"), &wasm_bindgen::JsValue::from_f64(ctx.depth as f64)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"indexInParent\"), &wasm_bindgen::JsValue::from_f64(ctx.index_in_parent as f64)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"isInline\"), &wasm_bindgen::JsValue::from_bool(ctx.is_inline)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    let parent_tag_val = match &ctx.parent_tag {{\n        Some(s) => wasm_bindgen::JsValue::from_str(s),\n        None => wasm_bindgen::JsValue::null(),\n    }};"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"parentTag\"), &parent_tag_val).ok();"
    )
    .unwrap();
    writeln!(out, "    let attrs = js_sys::Object::new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(
        out,
        "        js_sys::Reflect::set(&attrs, &wasm_bindgen::JsValue::from_str(k), &wasm_bindgen::JsValue::from_str(v)).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"attributes\"), &attrs).ok();"
    )
    .unwrap();
    writeln!(out, "    obj.into()").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    js_obj: wasm_bindgen::JsValue,").unwrap();
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
    writeln!(out, "    pub fn new(js_obj: wasm_bindgen::JsValue) -> Self {{").unwrap();
    writeln!(out, "        Self {{ js_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_wasm(out, method, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_wasm(out: &mut String, method: &MethodDef, type_paths: &HashMap<String, String>) {
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

    writeln!(out, "    fn {name}({sig}) -> {ret_ty} {{").unwrap();

    // Check if the JS object has the method via Reflect
    writeln!(out, "        let key = wasm_bindgen::JsValue::from_str(\"{js_name}\");").unwrap();
    writeln!(
        out,
        "        let has_method = js_sys::Reflect::has(&self.js_obj, &key).unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !has_method {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Get the JS function
    writeln!(
        out,
        "        let func_val = match js_sys::Reflect::get(&self.js_obj, &key) {{"
    )
    .unwrap();
    writeln!(out, "            Ok(f) => f,").unwrap();
    writeln!(out, "            Err(_) => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();
    writeln!(out, "        let func: js_sys::Function = match func_val.dyn_into() {{").unwrap();
    writeln!(out, "            Ok(f) => f,").unwrap();
    writeln!(out, "            Err(_) => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();

    // Build args array
    writeln!(out, "        let args = js_sys::Array::new();").unwrap();
    for p in &method.params {
        let arg_val = build_wasm_arg(p);
        writeln!(out, "        args.push(&{arg_val});").unwrap();
    }

    // Call the function
    writeln!(out, "        let result = func.apply(&self.js_obj, &args);").unwrap();

    // Parse result
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(out, "                if let Some(s) = val.as_string() {{").unwrap();
    writeln!(out, "                    match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                        \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                        \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                        \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(
        out,
        "                        other => {ret_ty}::Custom(other.to_string()),"
    )
    .unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                }} else if val.is_object() {{").unwrap();
    writeln!(
        out,
        "                    let custom_key = wasm_bindgen::JsValue::from_str(\"custom\");"
    )
    .unwrap();
    writeln!(
        out,
        "                    let error_key = wasm_bindgen::JsValue::from_str(\"error\");"
    )
    .unwrap();
    writeln!(
        out,
        "                    if let Ok(cv) = js_sys::Reflect::get(&val, &custom_key) {{"
    )
    .unwrap();
    writeln!(out, "                        if let Some(s) = cv.as_string() {{").unwrap();
    writeln!(out, "                            return {ret_ty}::Custom(s);").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(
        out,
        "                    if let Ok(ev) = js_sys::Reflect::get(&val, &error_key) {{"
    )
    .unwrap();
    writeln!(out, "                        if ev.as_string().is_some() {{").unwrap();
    writeln!(out, "                            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                    {ret_ty}::Continue").unwrap();
    writeln!(out, "                }} else {{").unwrap();
    writeln!(out, "                    {ret_ty}::Continue").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single wasm arg expression for a visitor method parameter.
fn build_wasm_arg(p: &alef_core::ir::ParamDef) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
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
    format!("wasm_bindgen::JsValue::from_str(&format!(\"{{:?}}\", {}))", p.name)
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
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    // Bridge type is gated to wasm32 (see gen_trait_bridge); functions that
    // construct it must be too, otherwise host builds fail to resolve the type.
    writeln!(out, "#[cfg(target_arch = \"wasm32\")]").ok();
    writeln!(out, "#[wasm_bindgen{js_name_attr}]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
