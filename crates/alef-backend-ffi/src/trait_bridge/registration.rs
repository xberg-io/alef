//! Constructor generation and extern "C" registration/unregistration functions.

use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use alef_core::ir::{MethodDef, TypeRef};
use std::fmt::Write;

use super::FfiBridgeGenerator;

impl FfiBridgeGenerator {
    /// Generate the `impl {Bridge} { pub unsafe fn new(...) }` constructor block.
    pub(super) fn gen_constructor_impl(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {bridge} {{").ok();
        writeln!(
            out,
            "    /// Create a new bridge from a vtable and opaque user_data pointer."
        )
        .ok();
        writeln!(out, "    ///").ok();
        writeln!(out, "    /// # Safety").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// `vtable` must remain valid for the lifetime of the returned bridge."
        )
        .ok();
        writeln!(
            out,
            "    /// `user_data` must be valid for any thread that calls methods on this bridge."
        )
        .ok();
        writeln!(out, "    /// All required fn pointers in `vtable` must be non-null.").ok();
        writeln!(
            out,
            "    pub unsafe fn new(name: String, vtable: {vtable}, user_data: *const std::ffi::c_void) -> Self {{"
        )
        .ok();
        writeln!(
            out,
            "        Self {{ vtable, user_data, cached_name: name, cached_version: String::new() }}"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    /// Generate the `extern "C"` register and unregister functions.
    pub(super) fn gen_registration_fn_impl(&self, spec: &TraitBridgeSpec) -> String {
        let register_fn_name = spec
            .bridge_config
            .register_fn
            .as_deref()
            .expect("gen_registration_fn called without register_fn");
        let registry_getter = spec
            .bridge_config
            .registry_getter
            .as_deref()
            .expect("gen_registration_fn called without registry_getter");

        let prefix = &self.prefix;
        let trait_snake = spec.trait_snake();
        let vtable = self.vtable_name(spec);
        let bridge = self.bridge_name(spec);
        let trait_path = spec.trait_path();
        let full_register_name = format!("{prefix}_{register_fn_name}");
        let full_unregister_name = format!("{prefix}_unregister_{trait_snake}");

        let mut out = String::with_capacity(2048);

        // --- register function ---
        writeln!(
            out,
            "/// Register a C plugin implementing `{}` via a vtable.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Parameters").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// - `name`: null-terminated UTF-8 plugin name. Must not be null."
        )
        .ok();
        writeln!(
            out,
            "/// - `vtable`: vtable with function pointers implementing the trait."
        )
        .ok();
        writeln!(
            out,
            "/// - `user_data`: opaque pointer forwarded to every vtable function."
        )
        .ok();
        writeln!(
            out,
            "/// - `out_error`: receives a heap-allocated error string on failure."
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Safety").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// All function pointers in `vtable` must remain valid until the plugin is"
        )
        .ok();
        writeln!(
            out,
            "/// unregistered. `user_data` must be safe to use from any thread that calls"
        )
        .ok();
        writeln!(out, "/// into the plugin.").ok();
        writeln!(out, "#[unsafe(no_mangle)]").ok();
        writeln!(out, "pub unsafe extern \"C\" fn {full_register_name}(").ok();
        writeln!(out, "    name: *const std::ffi::c_char,").ok();
        writeln!(out, "    vtable: {vtable},").ok();
        writeln!(out, "    user_data: *const std::ffi::c_void,").ok();
        writeln!(out, "    out_error: *mut *mut std::ffi::c_char,").ok();
        writeln!(out, ") -> i32 {{").ok();
        writeln!(out, "    if name.is_null() {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, \"name must not be null\");").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();

        // Validate required fn pointers (non-default methods must be non-null)
        for method in spec.required_methods() {
            writeln!(out, "    if vtable.{}.is_none() {{", method.name).ok();
            writeln!(
                out,
                "        ffi_set_out_error(out_error, \"vtable.{} must not be null\");",
                method.name
            )
            .ok();
            writeln!(out, "        return 1;").ok();
            writeln!(out, "    }}").ok();
        }

        writeln!(
            out,
            "    // SAFETY: name is non-null (checked above); it points to a valid C string."
        )
        .ok();
        writeln!(
            out,
            "    let plugin_name = match unsafe {{ std::ffi::CStr::from_ptr(name) }}.to_str() {{"
        )
        .ok();
        writeln!(out, "        Ok(s) => s.to_owned(),").ok();
        writeln!(out, "        Err(_) => {{").ok();
        writeln!(
            out,
            "            ffi_set_out_error(out_error, \"name is not valid UTF-8\");"
        )
        .ok();
        writeln!(out, "            return 1;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }};").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "    // SAFETY: vtable and user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "    let bridge = unsafe {{ {bridge}::new(plugin_name, vtable, user_data) }};"
        )
        .ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(bridge);").ok();
        writeln!(out).ok();
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write();").ok();

        // Generate register call with optional extra args (e.g., priority for PostProcessor)
        let register_call = if let Some(extra_args) = &spec.bridge_config.register_extra_args {
            format!("registry.register(arc, {extra_args})")
        } else {
            "registry.register(arc)".to_string()
        };

        writeln!(out, "    if let Err(e) = {register_call} {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, &e.to_string());").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    0").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        // --- unregister function ---
        writeln!(out, "/// Unregister a previously registered C plugin by name.").ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Parameters").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// - `name`: null-terminated UTF-8 plugin name. Must not be null."
        )
        .ok();
        writeln!(
            out,
            "/// - `out_error`: receives a heap-allocated error string on failure."
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Safety").ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// `name` must point to a valid null-terminated C string.").ok();
        writeln!(out, "#[unsafe(no_mangle)]").ok();
        writeln!(out, "pub unsafe extern \"C\" fn {full_unregister_name}(").ok();
        writeln!(out, "    name: *const std::ffi::c_char,").ok();
        writeln!(out, "    out_error: *mut *mut std::ffi::c_char,").ok();
        writeln!(out, ") -> i32 {{").ok();
        writeln!(out, "    if name.is_null() {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, \"name must not be null\");").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();
        writeln!(
            out,
            "    // SAFETY: name is non-null (checked above); it points to a valid C string."
        )
        .ok();
        writeln!(
            out,
            "    let plugin_name = match unsafe {{ std::ffi::CStr::from_ptr(name) }}.to_str() {{"
        )
        .ok();
        writeln!(out, "        Ok(s) => s,").ok();
        writeln!(out, "        Err(_) => {{").ok();
        writeln!(
            out,
            "            ffi_set_out_error(out_error, \"name is not valid UTF-8\");"
        )
        .ok();
        writeln!(out, "            return 1;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }};").ok();
        writeln!(out).ok();
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write();").ok();
        writeln!(out, "    if let Err(e) = registry.remove(plugin_name) {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, &e.to_string());").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    0").ok();
        writeln!(out, "}}").ok();

        out
    }
}

impl TraitBridgeGenerator for FfiBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        // The "foreign object" in the vtable pattern is opaque — there is no
        // named Rust type for it.  We use this field only as a conceptual handle;
        // the generated struct does NOT follow the standard `inner: T` layout
        // (see `gen_trait_bridge` below which calls the generators individually).
        "*const std::ffi::c_void"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "std::ffi::{c_void, c_char, CStr, CString}".to_string(),
            "std::sync::Arc".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        self.gen_vtable_call_body(method, spec)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // For async methods we block-on inside a spawn_blocking call, mirroring
        // the Go/Java/C# strategy of running synchronous C callbacks on a thread pool.
        // The sync body references `self.vtable.*` and `self.user_data`; inside the
        // closure we have local `vtable` / `user_data` bindings instead.
        let sync_body = self
            .gen_vtable_call_body(method, spec)
            .replace("self.vtable.", "vtable.")
            .replace("self.user_data", "user_data");
        let has_error = method.error_type.is_some();
        let core_import = &self.core_import;
        let method_name = &method.name;
        let cached_name_clone = if has_error {
            "let _cached_name = self.cached_name.clone();\n"
        } else {
            ""
        };

        let _vtable_name = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        // *const c_void is !Send, but the caller guarantees thread-safety via the vtable
        // API contract. Wrap the entire closure in a Send newtype to bypass the check.
        writeln!(out, "struct _SendFn<F>(F);").ok();
        writeln!(
            out,
            "// SAFETY: caller guarantees vtable fn pointers and user_data are valid across threads."
        )
        .ok();
        writeln!(out, "unsafe impl<F> Send for _SendFn<F> {{}}").ok();
        writeln!(out, "impl<F: FnOnce() -> R, R> _SendFn<F> {{").ok();
        writeln!(out, "    fn call(self) -> R {{ (self.0)() }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(out, "{cached_name_clone}let vtable = self.vtable;").ok();
        writeln!(out, "let user_data = self.user_data;").ok();
        for p in &method.params {
            let clone_expr = match &p.ty {
                TypeRef::Path => format!("{}.to_path_buf()", p.name),
                TypeRef::Bytes => format!("{}.to_vec()", p.name),
                _ => format!("{}.clone()", p.name),
            };
            writeln!(out, "let {} = {clone_expr};", p.name).ok();
        }
        writeln!(out).ok();

        writeln!(out, "let _task = _SendFn(move || {{").ok();
        writeln!(out, "    // Inline the sync body:").ok();
        for line in sync_body.lines() {
            writeln!(out, "    {line}").ok();
        }
        writeln!(out, "}});").ok();
        writeln!(out, "tokio::task::spawn_blocking(move || _task.call())").ok();
        writeln!(out, ".await").ok();
        if has_error {
            let inner_error_constructor = spec.make_error("e.to_string()");
            writeln!(
                out,
                ".map_err(|e| {core_import}::KreuzbergError::Plugin {{ message: format!(\"spawn_blocking failed in {method_name}: {{}}\", e), plugin_name: String::new() }})?",
            )
            .ok();
            writeln!(
                out,
                ".map_err(|e: Box<dyn std::error::Error + Send + Sync>| {inner_error_constructor})",
            )
            .ok();
        } else {
            writeln!(out, ".unwrap_or_else(|_| Default::default())").ok();
        }
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        self.gen_constructor_impl(spec)
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        self.gen_registration_fn_impl(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_codegen::generators::trait_bridge::TraitBridgeGenerator;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
    use std::collections::HashMap;

    fn make_bridge_cfg_with_register(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        }
    }

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("my_lib::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }
    }

    fn make_method_required(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn make_generator() -> FfiBridgeGenerator {
        FfiBridgeGenerator {
            prefix: "ml".to_string(),
            core_import: "my_lib".to_string(),
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
        }
    }

    fn make_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "my_lib",
            wrapper_prefix: "Ml",
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
            error_constructor: "MyError::from({msg})".to_string(),
        }
    }

    #[test]
    fn constructor_generates_unsafe_new() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg_with_register("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_constructor(&spec);
        assert!(out.contains("pub unsafe fn new("), "must generate unsafe new");
        assert!(out.contains("impl MlBackendBridge"), "must be in impl block");
    }

    #[test]
    fn register_fn_is_extern_c_no_mangle() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg_with_register("Backend");
        let trait_def = make_trait_def("Backend", vec![make_method_required("run")]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_registration_fn(&spec);
        assert!(out.contains("#[unsafe(no_mangle)]"), "must be no_mangle");
        assert!(
            out.contains("extern \"C\" fn ml_register_backend"),
            "must have correct name"
        );
        assert!(
            out.contains("extern \"C\" fn ml_unregister_backend"),
            "must have unregister fn"
        );
    }

    #[test]
    fn register_fn_validates_null_name() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg_with_register("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_registration_fn(&spec);
        assert!(out.contains("if name.is_null()"), "must check name for null");
    }

    #[test]
    fn register_fn_validates_required_fn_ptrs() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg_with_register("Backend");
        let trait_def = make_trait_def("Backend", vec![make_method_required("run")]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_registration_fn(&spec);
        assert!(out.contains("vtable.run.is_none()"), "must validate required fn ptr");
    }

    #[test]
    fn trait_bridge_generator_foreign_object_type() {
        let generator = make_generator();
        assert_eq!(generator.foreign_object_type(), "*const std::ffi::c_void");
    }
}
