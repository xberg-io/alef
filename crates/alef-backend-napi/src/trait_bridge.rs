//! NAPI-RS-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to JavaScript objects via NAPI-RS.

use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    host_function_path, to_camel_case, visitor_param_type,
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

/// NAPI-specific trait bridge generator.
/// Implements code generation for bridging JavaScript objects to Rust traits.
pub struct NapiBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for NapiBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "napi::bindgen_prelude::Object<'static>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "napi::bindgen_prelude::{JsObjectValue, ToNapiValue, Unknown, Object}".to_string(),
            "napi::JsValue".to_string(),
            "std::sync::Arc".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(512);

        // Get the JS function from the object
        let js_args_exprs = build_napi_args(method);
        let inner_tuple_ty = unknown_tuple_type(js_args_exprs.len());
        let args_tuple_ty = if js_args_exprs.is_empty() {
            inner_tuple_ty.clone()
        } else {
            format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
        };

        writeln!(
            out,
            "let func: napi::bindgen_prelude::Function<{args_tuple_ty}, napi::bindgen_prelude::Unknown> = match self.inner.get_named_property(\"{name}\") {{"
        )
        .ok();
        writeln!(out, "    Ok(f) => f,").ok();
        if has_error {
            let err = spec.make_error("format!(\"Method '{}' not found on bridge object: {}\", self.cached_name, e)");
            writeln!(out, "    Err(e) => return Err({err}),").ok();
        } else {
            writeln!(out, "    Err(_) => return Default::default(),").ok();
        }
        writeln!(out, "}};").ok();

        // Build and call with args
        if js_args_exprs.is_empty() {
            writeln!(out, "let result = func.call(());").ok();
        } else {
            let tuple_str = if js_args_exprs.len() == 1 {
                format!("({},)", js_args_exprs[0])
            } else {
                format!("({})", js_args_exprs.join(", "))
            };
            writeln!(
                out,
                "let result = func.call(napi::bindgen_prelude::FnArgs::from({tuple_str}));"
            )
            .ok();
        }

        // Parse result
        if has_error {
            writeln!(out, "match result {{").ok();
            let err = spec.make_error(&format!(
                "format!(\"Plugin '{{}}' method '{}' failed: {{}}\", self.cached_name, e)",
                name
            ));
            writeln!(out, "    Err(e) => Err({err}),").ok();
            writeln!(out, "    Ok(val) => {{").ok();
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "        Ok(())").ok();
            } else {
                // For most return types, attempt string coercion from the JS value
                writeln!(out, "        // Convert JS value to Rust type via string coercion").ok();
                writeln!(out, "        let s = val.coerce_to_string()").ok();
                writeln!(out, "            .and_then(|s| s.into_utf8())").ok();
                writeln!(out, "            .and_then(|s| s.into_owned())").ok();
                writeln!(out, "            .map_err(|e| {{").ok();
                let err = spec.make_error(&format!(
                    "format!(\"Failed to extract return value from method '{}': {{}}\", e)",
                    name
                ));
                writeln!(out, "                {err}").ok();
                writeln!(out, "            }})?;").ok();
                writeln!(out, "        match s.as_str() {{").ok();
                writeln!(out, "            \"\" | \"null\" => Ok(Default::default()),").ok();
                writeln!(out, "            _ => {{").ok();
                writeln!(out, "                serde_json::from_str::<_>(&s).map_err(|_| {{").ok();
                let err = spec.make_error(&format!(
                    "format!(\"Failed to parse return value for method '{}'\", {})",
                    name, "self.cached_name"
                ));
                writeln!(out, "                    {err}").ok();
                writeln!(out, "                }})").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
            }
            writeln!(out, "    }}").ok();
            writeln!(out, "}}").ok();
        } else {
            // Non-Result return: must not use ? or wrap in Ok()
            writeln!(out, "match result {{").ok();
            writeln!(out, "    Err(_) => Default::default(),").ok();
            writeln!(out, "    Ok(val) => {{").ok();
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "        ()").ok();
            } else {
                // For most return types, attempt string coercion from the JS value
                writeln!(out, "        // Convert JS value to Rust type via string coercion").ok();
                writeln!(out, "        let s = val.coerce_to_string()").ok();
                writeln!(out, "            .and_then(|s| s.into_utf8())").ok();
                writeln!(out, "            .and_then(|s| s.into_owned())").ok();
                writeln!(out, "            .unwrap_or_default();").ok();
                writeln!(out, "        match s.as_str() {{").ok();
                writeln!(out, "            \"\" | \"null\" => Default::default(),").ok();
                writeln!(
                    out,
                    "            _ => serde_json::from_str::<_>(&s).unwrap_or_default()"
                )
                .ok();
                writeln!(out, "        }}").ok();
            }
            writeln!(out, "    }}").ok();
            writeln!(out, "}}").ok();
        }
        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let mut out = String::with_capacity(1024);

        // NAPI has native async support via BoxPromise
        writeln!(out, "let cached_name = self.cached_name.clone();").ok();

        // Build the JS function call
        let js_args_exprs = build_napi_args(method);
        let inner_tuple_ty = unknown_tuple_type(js_args_exprs.len());
        let args_tuple_ty = if js_args_exprs.is_empty() {
            inner_tuple_ty.clone()
        } else {
            format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
        };

        writeln!(
            out,
            "let func: napi::bindgen_prelude::Function<{args_tuple_ty}, napi::bindgen_prelude::Unknown> = match self.inner.get_named_property(\"{name}\") {{"
        )
        .ok();
        writeln!(out, "    Ok(f) => f,").ok();
        writeln!(out, "    Err(e) => {{").ok();
        let err = spec.make_error("format!(\"Method '{}' not found on bridge object: {}\", cached_name, e)");
        writeln!(out, "        return Err({err});").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}};").ok();

        let tuple_str = if js_args_exprs.is_empty() {
            "()".to_string()
        } else if js_args_exprs.len() == 1 {
            format!("({},)", js_args_exprs[0])
        } else {
            format!("({})", js_args_exprs.join(", "))
        };

        if js_args_exprs.is_empty() {
            writeln!(out, "let result = func.call({tuple_str});").ok();
        } else {
            writeln!(
                out,
                "let result = func.call(napi::bindgen_prelude::FnArgs::from({tuple_str}));"
            )
            .ok();
        }
        writeln!(out, "match result {{").ok();
        let err = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' failed: {{}}\", cached_name, e)",
            name
        ));
        writeln!(out, "    Err(e) => Err({err}),").ok();
        writeln!(out, "    Ok(val) => {{").ok();
        if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "        Ok(())").ok();
        } else {
            // For most return types, attempt string coercion from the JS value
            writeln!(out, "        // Convert JS value to Rust type via string coercion").ok();
            writeln!(out, "        let s = val.coerce_to_string()").ok();
            writeln!(out, "            .and_then(|s| s.into_utf8())").ok();
            writeln!(out, "            .and_then(|s| s.into_owned())").ok();
            writeln!(out, "            .map_err(|e| {{").ok();
            let err = spec.make_error(&format!(
                "format!(\"Failed to extract return value from method '{}': {{}}\", e)",
                name
            ));
            writeln!(out, "                {err}").ok();
            writeln!(out, "            }})?;").ok();
            // Try JSON parsing first, then default
            writeln!(out, "        match s.as_str() {{").ok();
            writeln!(out, "            \"\" | \"null\" => Ok(Default::default()),").ok();
            writeln!(out, "            _ => serde_json::from_str::<_>(&s).map_err(|_| {{").ok();
            let err = spec.make_error(&format!(
                "\"Failed to parse return value for method '{}'\".to_string()",
                name
            ));
            writeln!(out, "                {err}").ok();
            writeln!(out, "            }})").ok();
            writeln!(out, "        }}").ok();
        }
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(out, "    /// Create a new bridge wrapping a NAPI Object.").ok();
        writeln!(out, "    ///").ok();
        writeln!(out, "    /// Validates that the object provides all required methods.").ok();
        writeln!(
            out,
            "    pub fn new(js_obj: napi::bindgen_prelude::Object<'_>) -> napi::Result<Self> {{"
        )
        .ok();

        // Validate all required methods exist
        for req_method in spec.required_methods() {
            writeln!(
                out,
                "        if !js_obj.has_named_property(\"{}\").unwrap_or(false) {{",
                req_method.name
            )
            .ok();
            writeln!(out, "            return Err(napi::Error::new(").ok();
            writeln!(out, "                napi::Status::GenericFailure,").ok();
            writeln!(
                out,
                "                format!(\"Object missing required method: {{}}\", \"{}\")",
                req_method.name
            )
            .ok();
            writeln!(out, "            ));").ok();
            writeln!(out, "        }}").ok();
        }

        // Transmute Object<'_> to Object<'static> for the stored field
        writeln!(
            out,
            "        // SAFETY: The JS object is owned by the Node.js runtime and lives for"
        )
        .ok();
        writeln!(
            out,
            "        // the duration of the enclosing #[napi] call. The bridge is only used"
        )
        .ok();
        writeln!(
            out,
            "        // synchronously during that same call, so 'static is safe here."
        )
        .ok();
        writeln!(
            out,
            "        let js_obj: napi::bindgen_prelude::Object<'static> = unsafe {{"
        )
        .ok();
        writeln!(out, "            std::mem::transmute(js_obj)").ok();
        writeln!(out, "        }};").ok();

        // Try to extract name from the object
        writeln!(
            out,
            "        let cached_name = match js_obj.get_named_property::<String>(\"name\") {{"
        )
        .ok();
        writeln!(out, "            Ok(n) => n,").ok();
        writeln!(out, "            Err(_) => \"unknown\".to_string(),").ok();
        writeln!(out, "        }};").ok();

        writeln!(out, "        Ok(Self {{").ok();
        writeln!(out, "            inner: js_obj,").ok();
        writeln!(out, "            cached_name,").ok();
        writeln!(out, "        }})").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();
        writeln!(out, "    /// Extract napi::Env from the stored Object.").ok();
        writeln!(out, "    fn env(&self) -> napi::Env {{").ok();
        writeln!(
            out,
            "        // SAFETY: Object<'static> is 3 pointer-sized words; first word is napi_env."
        )
        .ok();
        writeln!(
            out,
            "        let raw: [*mut std::ffi::c_void; 3] = unsafe {{ std::mem::transmute_copy(&self.inner) }};"
        )
        .ok();
        writeln!(out, "        napi::Env::from_raw(raw[0] as napi::sys::napi_env)").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "// SAFETY: The bridge is created from a NAPI Object that is pinned to the"
        )
        .ok();
        writeln!(
            out,
            "// Node.js event loop thread. All access occurs on that thread. Send+Sync"
        )
        .ok();
        writeln!(
            out,
            "// are required by the Plugin trait but the bridge is never actually moved"
        )
        .ok();
        writeln!(out, "// across threads.").ok();
        writeln!(out, "unsafe impl Send for {wrapper} {{}}").ok();
        writeln!(out, "unsafe impl Sync for {wrapper} {{}}").ok();
        out
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, unregister_fn);
        let camel = to_camel_case(unregister_fn);
        let mut out = String::with_capacity(512);
        writeln!(out, "#[napi(js_name = \"{camel}\")]").ok();
        writeln!(out, "pub fn _alef_{unregister_fn}(name: String) -> napi::Result<()> {{").ok();
        writeln!(out, "    {host_path}(&name).map_err(|e| napi::Error::new(").ok();
        writeln!(out, "        napi::Status::GenericFailure,").ok();
        writeln!(out, "        format!(\"{{}}\", e),").ok();
        writeln!(out, "    ))").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, clear_fn);
        let camel = to_camel_case(clear_fn);
        let mut out = String::with_capacity(512);
        writeln!(out, "#[napi(js_name = \"{camel}\")]").ok();
        writeln!(out, "pub fn _alef_{clear_fn}() -> napi::Result<()> {{").ok();
        writeln!(out, "    {host_path}().map_err(|e| napi::Error::new(").ok();
        writeln!(out, "        napi::Status::GenericFailure,").ok();
        writeln!(out, "        format!(\"{{}}\", e),").ok();
        writeln!(out, "    ))").ok();
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

        writeln!(out, "#[napi]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(obj: napi::bindgen_prelude::Object) -> napi::Result<()> {{"
        )
        .ok();

        // Create and validate the bridge
        writeln!(out, "    let bridge = {wrapper}::new(obj)?;").ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(bridge);").ok();

        // Register in the plugin registry (synchronous, no GC needed for NAPI)
        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write();").ok();
        writeln!(out, "    registry.register(arc{extra}).map_err(|e| napi::Error::new(").ok();
        writeln!(out, "        napi::Status::GenericFailure,").ok();
        writeln!(out, "        format!(\"Failed to register backend: {{}}\", e)").ok();
        writeln!(out, "    ))").ok();
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
    error_constructor: &str,
    api: &ApiSurface,
) -> BridgeOutput {
    // Build type name → rust_path lookup (converted to String-owned HashMap)
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
        let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");
        let code = gen_visitor_bridge(
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
        BridgeOutput { imports: vec![], code }
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = NapiBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Js",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    }
}

/// Generate a visitor-style bridge wrapping a `napi::bindgen_prelude::Object`.
///
/// Every trait method checks if the JS object has a matching camelCase property,
/// then calls it with converted arguments and maps the JS return value to `VisitResult`.
fn gen_visitor_bridge(
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(8192);
    // Emit trait imports needed by the generated bridge code.
    // napi::* glob does not re-export JsObjectValue or JsValue from bindgen_prelude.
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(
        out,
        "use napi::bindgen_prelude::{{JsObjectValue, ToNapiValue, Unknown, Object}};"
    )
    .unwrap();
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use napi::JsValue;").unwrap();
    writeln!(out).unwrap();

    // Helper: convert NodeContext to a JS object
    writeln!(out, "fn nodecontext_to_js_object<'e>(").unwrap();
    writeln!(out, "    env: &'e napi::Env,").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> napi::Result<napi::bindgen_prelude::Object<'e>> {{").unwrap();
    writeln!(out, "    let mut obj = napi::bindgen_prelude::Object::new(env)?;").unwrap();
    writeln!(
        out,
        "    obj.set_named_property(\"nodeType\", env.create_string(&format!(\"{{:?}}\", ctx.node_type))?)?;"
    )
    .unwrap();
    writeln!(
        out,
        "    obj.set_named_property(\"tagName\", env.create_string(&ctx.tag_name)?)?;"
    )
    .unwrap();
    writeln!(
        out,
        "    obj.set_named_property(\"depth\", env.create_uint32(ctx.depth as u32)?)?;"
    )
    .unwrap();
    writeln!(
        out,
        "    obj.set_named_property(\"indexInParent\", env.create_uint32(ctx.index_in_parent as u32)?)?;"
    )
    .unwrap();
    writeln!(out, "    obj.set_named_property(\"isInline\", ctx.is_inline)?;").unwrap();
    writeln!(out, "    let parent_tag = match &ctx.parent_tag {{").unwrap();
    writeln!(out, "        Some(s) => env.create_string(s)?.to_unknown(),").unwrap();
    writeln!(out, "        None => {{").unwrap();
    writeln!(
        out,
        "            // SAFETY: napi_get_null returns a valid napi_value for the given env."
    )
    .unwrap();
    writeln!(
        out,
        "            let raw = unsafe {{ napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), napi::bindgen_prelude::Null)? }};"
    )
    .unwrap();
    writeln!(
        out,
        "            unsafe {{ napi::bindgen_prelude::Unknown::from_raw_unchecked(env.raw(), raw) }}"
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }};").unwrap();
    writeln!(out, "    obj.set_named_property(\"parentTag\", parent_tag)?;").unwrap();
    writeln!(out, "    let mut attrs = napi::bindgen_prelude::Object::new(env)?;").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(out, "        attrs.set_named_property(k, env.create_string(v)?)?;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    obj.set_named_property(\"attributes\", attrs)?;").unwrap();
    writeln!(out, "    Ok(obj)").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct: store Object<'static> to avoid Object<'env> lifetime constraints.
    // SAFETY invariant: the Object is kept alive by the JS caller for the duration of the
    // #[napi] function that created the bridge, and by extension for all visitor callbacks.
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    obj: napi::bindgen_prelude::Object<'static>,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Manual Debug impl (Object doesn't implement Debug)
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

    // Constructor: transmute Object<'_> to Object<'static> to bypass the lifetime.
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(
        out,
        "    pub fn new(js_obj: napi::bindgen_prelude::Object<'_>) -> Self {{"
    )
    .unwrap();
    writeln!(
        out,
        "        // SAFETY: The JS object is owned by the Node.js runtime and lives for"
    )
    .unwrap();
    writeln!(
        out,
        "        // the duration of the enclosing #[napi] call. The bridge is only used"
    )
    .unwrap();
    writeln!(
        out,
        "        // synchronously during that same call, so 'static is safe here."
    )
    .unwrap();
    writeln!(
        out,
        "        let obj: napi::bindgen_prelude::Object<'static> = unsafe {{ std::mem::transmute(js_obj) }};"
    )
    .unwrap();
    writeln!(out, "        Self {{ obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    // Helper: extract napi_env from the Object. Object<'static> stores napi_env as its
    // first pointer-sized field. This is an internal layout assumption for napi-rs v3.
    writeln!(out, "    fn env(&self) -> napi::Env {{").unwrap();
    writeln!(
        out,
        "        // SAFETY: Object<'static> is 3 pointer-sized words; first word is napi_env."
    )
    .unwrap();
    writeln!(
        out,
        "        let raw: [*mut std::ffi::c_void; 3] = unsafe {{ std::mem::transmute_copy(&self.obj) }};"
    )
    .unwrap();
    writeln!(out, "        napi::Env::from_raw(raw[0] as napi::sys::napi_env)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_napi(&mut out, method, trait_path, core_crate, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    out
}

/// Build the Function args tuple type string for a given number of Unknown args.
fn unknown_tuple_type(count: usize) -> String {
    if count == 0 {
        return "()".to_string();
    }
    let parts = vec!["napi::bindgen_prelude::Unknown"; count];
    format!("({}{})", parts.join(", "), if count == 1 { "," } else { "" })
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_napi(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    _core_crate: &str,
    type_paths: &HashMap<String, String>,
) {
    let name = &method.name;

    // Convert snake_case method name to camelCase for JS
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

    // Check if JS object has the method
    writeln!(
        out,
        "        let has_method = self.obj.has_named_property(\"{js_name}\").unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !has_method {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Get the JS function with the correct tuple arg type.
    // Use FnArgs<(Unknown, ...)> for N>0 args so the macro-generated JsValuesTupleIntoVec
    // impl is used, which correctly expands each element into a separate NAPI value.
    let arg_count = method.params.len();
    let inner_tuple_ty = unknown_tuple_type(arg_count);
    let args_tuple_ty = if arg_count == 0 {
        inner_tuple_ty.clone()
    } else {
        format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
    };
    writeln!(
        out,
        "        let func: napi::bindgen_prelude::Function<{args_tuple_ty}, napi::bindgen_prelude::Unknown> = match self.obj.get_named_property(\"{js_name}\") {{"
    )
    .unwrap();
    writeln!(out, "            Ok(f) => f,").unwrap();
    writeln!(out, "            Err(_) => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();

    // Build and call with args
    let js_args_exprs = build_napi_args(method);
    if arg_count == 0 {
        writeln!(out, "        let result = func.call(());").unwrap();
    } else {
        // Bind env to a named variable so borrows from it outlive the statement.
        writeln!(out, "        let __env = self.env();").unwrap();
        for (i, expr) in js_args_exprs.iter().enumerate() {
            let expr = expr.replace("self.env()", "__env");
            writeln!(out, "        let arg_{i}: napi::bindgen_prelude::Unknown = {expr};").unwrap();
        }
        let tuple_args: Vec<String> = (0..arg_count).map(|i| format!("arg_{i}")).collect();
        let tuple_str = if arg_count == 1 {
            format!("({},)", tuple_args[0])
        } else {
            format!("({})", tuple_args.join(", "))
        };
        // Wrap in FnArgs so each element is passed as a separate JavaScript argument.
        writeln!(
            out,
            "        let result = func.call(napi::bindgen_prelude::FnArgs::from({tuple_str}));"
        )
        .unwrap();
    }

    // Parse result.
    // Strategy: only inspect {custom}/{error} properties when the value is a
    // plain Object. coerce_to_object() succeeds on string primitives too (JS
    // wraps them in a String object), so get_named_property("custom") on a
    // string returns Ok(undefined), and coerce_to_string(undefined) yields
    // the literal "undefined". Check the actual ValueType first.
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(
        out,
        "                if val.get_type().ok() == Some(napi::bindgen_prelude::ValueType::Object) {{"
    )
    .unwrap();
    writeln!(out, "                    if let Ok(obj) = val.coerce_to_object() {{").unwrap();
    writeln!(out, "                        if let Ok(cv) = obj.get_named_property::<napi::bindgen_prelude::Unknown>(\"custom\") {{").unwrap();
    writeln!(out, "                            if !matches!(cv.get_type().unwrap_or(napi::bindgen_prelude::ValueType::Undefined), napi::bindgen_prelude::ValueType::Undefined | napi::bindgen_prelude::ValueType::Null) {{").unwrap();
    writeln!(out, "                                if let Ok(s) = cv.coerce_to_string().and_then(|s| s.into_utf8()).and_then(|s| s.into_owned()) {{").unwrap();
    writeln!(out, "                                    return {ret_ty}::Custom(s);").unwrap();
    writeln!(out, "                                }}").unwrap();
    writeln!(out, "                            }}").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(
        out,
        "                        if let Ok(ev) = obj.get_named_property::<napi::bindgen_prelude::Unknown>(\"error\") {{"
    )
    .unwrap();
    writeln!(out, "                            if !matches!(ev.get_type().unwrap_or(napi::bindgen_prelude::ValueType::Undefined), napi::bindgen_prelude::ValueType::Undefined | napi::bindgen_prelude::ValueType::Null) {{").unwrap();
    writeln!(out, "                                if let Ok(s) = ev.coerce_to_string().and_then(|s| s.into_utf8()).and_then(|s| s.into_owned()) {{").unwrap();
    writeln!(out, "                                    return {ret_ty}::Error(s);").unwrap();
    writeln!(out, "                                }}").unwrap();
    writeln!(out, "                            }}").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "                if let Ok(s) = val.coerce_to_string().and_then(|s| s.into_utf8()).and_then(|s| s.into_owned()) {{").unwrap();
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
    writeln!(out, "                }} else {{").unwrap();
    writeln!(out, "                    {ret_ty}::Continue").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build NAPI argument expressions for a visitor method.
///
/// Returns one expression per parameter, each producing a `napi::bindgen_prelude::Unknown`.
fn build_napi_args(method: &MethodDef) -> Vec<String> {
    method
        .params
        .iter()

        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    return format!(
                        "match nodecontext_to_js_object(&self.env(), {}{}) {{ Ok(o) => o.to_unknown(), Err(_) => unsafe {{ \
                         let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                         napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                        }}",
                        if p.is_ref { "" } else { "&" },
                        p.name
                    );
                }
            }
            // Option<&str>
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match {name} {{ \
                     Some(s) => match self.env().create_string(s) {{ \
                       Ok(v) => v.to_unknown(), \
                       Err(_) => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                     }}, \
                     None => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // &str
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match self.env().create_string({name}) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // String (owned)
            if matches!(&p.ty, TypeRef::String) {
                return format!(
                    "match self.env().create_string({name}.as_str()) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // Bool
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
                return format!(
                    "unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {name}).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
                    name = p.name
                );
            }
            // u32 / usize: create_uint32 needs a u32; usize requires the cast but u32 does not.
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::U32)) {
                return format!(
                    "match self.env().create_uint32({name}) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Usize)) {
                return format!(
                    "match self.env().create_uint32({name} as u32) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // Vec<String> or &[String] - serialize to JSON string as fallback
            // Default: serialize as debug string
            format!(
                "match self.env().create_string(&format!(\"{{:?}}\", {name})) {{ Ok(s) => s.to_unknown(), Err(_) => unsafe {{ \
                 let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(std::ptr::null_mut(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                 napi::bindgen_prelude::Unknown::from_raw_unchecked(std::ptr::null_mut(), r) }} \
                }}",
                name = p.name
            )
        })
        .collect()
}

/// Generate a NAPI free function that has one parameter replaced by
/// `Option<napi::bindgen_prelude::Object>` (a trait bridge). The bridge is constructed
/// before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    _cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    _adapter_bodies: &alef_codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Check if this is an options_field binding pattern (visitor embedded in options struct)
    let is_options_field_binding = matches!(bridge_cfg.bind_via, alef_core::config::BridgeBinding::OptionsField);

    // Find the options parameter when using options_field binding
    let options_param_idx = if is_options_field_binding {
        func.params.iter().enumerate().find(|(_, p)| {
            matches!(&p.ty, TypeRef::Named(n) if bridge_cfg.options_type.as_ref().is_some_and(|opt_type| n == opt_type))
        }).map(|(i, _)| i)
    } else {
        None
    };

    // Build parameter list: bridge param becomes Option<Object>, no explicit env param
    // (napi v3 does not implement FromNapiValue for Env; env is obtained from the Object)
    let mut sig_parts = vec![];
    for (idx, p) in func.params.iter().enumerate() {
        if is_options_field_binding && Some(idx) == options_param_idx {
            // For options_field binding, visitor is extracted from options, not a separate param
            let ty = if p.optional || (idx > 0 && func.params[..idx].iter().any(|pp| pp.optional)) {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        } else if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<napi::bindgen_prelude::Object>", p.name));
            } else {
                sig_parts.push(format!("{}: napi::bindgen_prelude::Object", p.name));
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

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    // Bridge wrapping code: constructor is infallible (transmute-based).
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

    // Use From/Into for non-bridge Named params — the generated bindings have From impls.
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
                format!("let {name}_core: Option<{core_path}> = {name}.map(|v| v.into());\n    ")
            } else {
                format!("let {name}_core: {core_path} = {name}.into();\n    ")
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

    let js_name = {
        let mut result = String::with_capacity(func.name.len());
        let mut capitalize_next = false;
        for (i, c) in func.name.chars().enumerate() {
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
    };
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    writeln!(out, "#[napi{js_name_attr}]").ok();
    let func_name = &func.name;
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate a NAPI free function where a trait bridge is embedded in an options struct field.
/// The visitor is extracted from options before the Into conversion, wrapped in a bridge,
/// and manually injected back into the converted core options.
#[allow(clippy::too_many_arguments)]
pub fn gen_options_field_bridge_function(
    func: &alef_core::ir::FunctionDef,
    options_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    _cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;
    use std::fmt::Write;

    let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let options_param = &func.params[options_param_idx];
    let options_name = &options_param.name;

    // Bridge functions always treat the options param as optional: callers may pass
    // undefined/null (no options) or an options object (with or without visitor).
    // Even if the IR marks the param as non-optional (e.g. because has_default types
    // get their Option<> stripped during IR parsing), we force Option<T> behavior here.
    let is_param_optional = true;

    // Whether the IR already marks the options param as Optional<T>.
    let ir_param_optional = matches!(&options_param.ty, TypeRef::Optional(_));

    // Build parameter list; force the options param to Option<T> if the IR didn't already.
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
        sig_parts.join(", ")
    };

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    // Generate visitor extraction and bridge creation
    let visitor_extract = if is_param_optional {
        format!(
            "let visitor_handle = {options_name}.as_ref().and_then(|o| o.visitor.clone()).map(|v| {{\n    \
             let bridge = {struct_name}::new(v);\n    \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n\
             }});"
        )
    } else {
        format!(
            "let visitor_handle = {options_name}.visitor.clone().map(|v| {{\n    \
             let bridge = {struct_name}::new(v);\n    \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n\
             }});"
        )
    };

    // Generate options conversion with visitor preservation.
    // To avoid the From impl dropping the visitor field (it's marked as Default::default()),
    // we clear it from the cloned options before conversion, then re-inject the extracted handle.
    // This ensures the bridge wrapper survives the conversion.
    let options_convert = if is_param_optional {
        format!(
            "let mut {options_name}_core: Option<{core_import}::ConversionOptions> = {options_name}.map(|mut o| {{\n    \
             o.visitor = None;\n    \
             let mut result: {core_import}::ConversionOptions = o.into();\n    \
             result.visitor = visitor_handle.clone();\n    \
             result\n    \
             }});"
        )
    } else {
        format!(
            "let mut {options_name}_core: Option<{core_import}::ConversionOptions> = {{\n    \
             let mut o = {options_name}.clone();\n    \
             o.visitor = None;\n    \
             let mut result: {core_import}::ConversionOptions = o.into();\n    \
             result.visitor = visitor_handle.clone();\n    \
             Some(result)\n    \
             }};"
        )
    };

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
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{visitor_extract}\n    {options_convert}\n    {core_call}{err_conv}")
        } else {
            format!("{visitor_extract}\n    {options_convert}\n    {core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{visitor_extract}\n    {options_convert}\n    {core_call}")
    };

    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    writeln!(out, "#[napi]").ok();
    let func_name = &func.name;
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
