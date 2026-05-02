//! Vtable struct, bridge struct, Drop impl, and Plugin super-trait impl generation.

use alef_codegen::generators::trait_bridge::TraitBridgeSpec;
use alef_core::ir::MethodDef;
use std::fmt::Write;

use super::FfiBridgeGenerator;

impl FfiBridgeGenerator {
    /// Generate the vtable struct definition.
    pub(super) fn gen_vtable_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        writeln!(
            out,
            "/// VTable for C plugin bridges implementing the `{}` trait.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Safety").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// All function pointers must be valid for the lifetime of any bridge created from"
        )
        .ok();
        writeln!(
            out,
            "/// this vtable.  `free_user_data`, when non-null, is called once with `user_data`"
        )
        .ok();
        writeln!(out, "/// when the bridge is dropped.").ok();
        writeln!(out, "#[derive(Copy, Clone)]").ok();
        writeln!(out, "#[repr(C)]").ok();
        writeln!(out, "pub struct {vtable} {{").ok();

        // Super-trait methods (Plugin: name, version, initialize, shutdown)
        if spec.bridge_config.super_trait.is_some() {
            writeln!(
                out,
                "    /// Return a null-terminated UTF-8 name string into `out_name`."
            )
            .ok();
            writeln!(
                out,
                "    pub name_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_name: *mut *mut std::ffi::c_char)>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Return a null-terminated UTF-8 version string into `out_version`."
            )
            .ok();
            writeln!(
                out,
                "    pub version_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_version: *mut *mut std::ffi::c_char)>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Initialise the plugin; return 0 on success, non-zero on failure (error text in `out_error`)."
            )
            .ok();
            writeln!(
                out,
                "    pub initialize_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_error: *mut *mut std::ffi::c_char) -> i32>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Shut down the plugin; return 0 on success, non-zero on failure (error text in `out_error`)."
            )
            .ok();
            writeln!(
                out,
                "    pub shutdown_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_error: *mut *mut std::ffi::c_char) -> i32>,"
            )
            .ok();
        }

        // One field per trait method (own methods only; super-trait methods are covered above)
        let own_methods: Vec<_> = spec
            .trait_def
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .collect();

        for method in &own_methods {
            if !method.doc.is_empty() {
                for line in method.doc.lines() {
                    let stripped = line.trim_start_matches("///").trim_start();
                    if stripped.is_empty() {
                        writeln!(out, "    ///").ok();
                    } else {
                        writeln!(out, "    /// {stripped}").ok();
                    }
                }
            }
            writeln!(out, "{}", self.vtable_fn_ptr_field(method)).ok();
        }

        // free_user_data destructor
        writeln!(
            out,
            "    /// Optional destructor: called once with `user_data` when the bridge is dropped."
        )
        .ok();
        writeln!(
            out,
            "    pub free_user_data: Option<unsafe extern \"C\" fn(*mut std::ffi::c_void)>,"
        )
        .ok();

        writeln!(out, "}}").ok();
        let vtable = self.vtable_name(spec);
        writeln!(
            out,
            "// SAFETY: all fields are function pointers and free_user_data, which are Send + Sync."
        )
        .ok();
        writeln!(out, "unsafe impl Send for {vtable} {{}}").ok();
        writeln!(out, "unsafe impl Sync for {vtable} {{}}").ok();
        out
    }

    /// Generate the bridge struct with `vtable`, `user_data`, `cached_name`, and `cached_version`.
    pub(super) fn gen_bridge_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let bridge = self.bridge_name(spec);
        let mut out = String::with_capacity(512);

        writeln!(
            out,
            "/// Rust-side bridge that holds a C vtable pointer and opaque `user_data`."
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// Implements `{}` by forwarding calls through the vtable.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "pub struct {bridge} {{").ok();
        writeln!(out, "    vtable: {vtable},").ok();
        writeln!(out, "    user_data: *const std::ffi::c_void,").ok();
        writeln!(out, "    cached_name: String,").ok();
        writeln!(out, "    cached_version: String,").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        // The vtable contains raw fn pointers which are not Debug, so we implement manually.
        writeln!(out, "impl std::fmt::Debug for {bridge} {{").ok();
        writeln!(
            out,
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
        )
        .ok();
        writeln!(out, "        f.debug_struct(\"{bridge}\")").ok();
        writeln!(out, "            .field(\"cached_name\", &self.cached_name)").ok();
        writeln!(out, "            .field(\"cached_version\", &self.cached_version)").ok();
        writeln!(out, "            .finish_non_exhaustive()").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "// SAFETY: The caller is responsible for ensuring `user_data` is safe to send across"
        )
        .ok();
        writeln!(
            out,
            "// thread boundaries. This is documented in `{vtable}` and the registration function."
        )
        .ok();
        writeln!(out, "unsafe impl Send for {bridge} {{}}").ok();
        writeln!(out, "unsafe impl Sync for {bridge} {{}}").ok();
        out
    }

    /// Generate the `Drop` impl that calls `free_user_data` if non-null.
    pub(super) fn gen_bridge_drop(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);
        let mut out = String::with_capacity(256);

        writeln!(out, "impl Drop for {bridge} {{").ok();
        writeln!(out, "    fn drop(&mut self) {{").ok();
        writeln!(out, "        if let Some(free_fn) = self.vtable.free_user_data {{").ok();
        writeln!(
            out,
            "            // SAFETY: free_fn is a valid function pointer; user_data is the pointer"
        )
        .ok();
        writeln!(
            out,
            "            // originally provided at registration. Called exactly once here."
        )
        .ok();
        writeln!(
            out,
            "            unsafe {{ free_fn(self.user_data as *mut std::ffi::c_void) }}"
        )
        .ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    /// Generate the `impl Plugin for FfiBridge` block using vtable fn pointers.
    pub(super) fn gen_ffi_plugin_impl(&self, spec: &TraitBridgeSpec) -> Option<String> {
        let super_trait_name = spec.bridge_config.super_trait.as_deref()?;
        let bridge = self.bridge_name(spec);
        let core_import = &self.core_import;

        let super_trait_path = if super_trait_name.contains("::") {
            super_trait_name.to_string()
        } else {
            format!("{core_import}::{super_trait_name}")
        };

        let mut out = String::with_capacity(1024);
        writeln!(out, "impl {super_trait_path} for {bridge} {{").ok();

        // name() — uses cached_name
        writeln!(out, "    fn name(&self) -> &str {{").ok();
        writeln!(out, "        &self.cached_name").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // version() — calls vtable.version_fn, returns String
        writeln!(out, "    fn version(&self) -> String {{").ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.version_fn else {{ return String::new() }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(out, "        unsafe {{ fp(self.user_data, &mut _out) }};").ok();
        writeln!(
            out,
            "        if _out.is_null() {{ return self.cached_version.clone(); }}"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: _out is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(out, "        let cs = unsafe {{ std::ffi::CString::from_raw(_out) }};").ok();
        writeln!(out, "        cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // initialize()
        let error_type = &self.error_type;
        writeln!(
            out,
            "    fn initialize(&self) -> std::result::Result<(), {core_import}::{error_type}> {{"
        )
        .ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.initialize_fn else {{ return Ok(()); }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "        let rc = unsafe {{ fp(self.user_data, &mut _out_error) }};"
        )
        .ok();
        writeln!(out, "        if rc != 0 {{").ok();
        writeln!(
            out,
            "            let msg = if _out_error.is_null() {{ format!(\"initialize returned {{}}\", rc) }} else {{"
        )
        .ok();
        writeln!(
            out,
            "                // SAFETY: _out_error is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(
            out,
            "                let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
        )
        .ok();
        writeln!(out, "                cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "            }};").ok();
        writeln!(
            out,
            "            return Err(kreuzberg::KreuzbergError::Plugin {{ message: msg, plugin_name: String::new() }});"
        )
        .ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        Ok(())").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // shutdown()
        writeln!(
            out,
            "    fn shutdown(&self) -> std::result::Result<(), {core_import}::{error_type}> {{"
        )
        .ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.shutdown_fn else {{ return Ok(()); }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "        let rc = unsafe {{ fp(self.user_data, &mut _out_error) }};"
        )
        .ok();
        writeln!(out, "        if rc != 0 {{").ok();
        writeln!(
            out,
            "            let msg = if _out_error.is_null() {{ format!(\"shutdown returned {{}}\", rc) }} else {{"
        )
        .ok();
        writeln!(
            out,
            "                // SAFETY: _out_error is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(
            out,
            "                let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
        )
        .ok();
        writeln!(out, "                cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "            }};").ok();
        writeln!(
            out,
            "            return Err(kreuzberg::KreuzbergError::Plugin {{ message: msg, plugin_name: String::new() }});"
        )
        .ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        Ok(())").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        Some(out)
    }

    /// Build the vtable function pointer field signature for one method.
    pub(super) fn vtable_fn_ptr_field(&self, method: &MethodDef) -> String {
        let mut params = vec!["user_data: *const std::ffi::c_void".to_string()];

        for p in &method.params {
            let cty = Self::c_param_type(&p.ty);
            params.push(format!("{}: {}", p.name, cty));
        }

        let has_error = method.error_type.is_some();
        let (out_params, ret_ty) = Self::c_return_convention(&method.return_type, has_error);
        params.extend(out_params);

        let params_str = params.join(", ");
        format!(
            "    pub {name}: Option<unsafe extern \"C\" fn({params_str}) -> {ret_ty}>,",
            name = method.name,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
    use std::collections::HashMap;

    fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,
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

    fn make_method(name: &str, return_type: TypeRef, has_error: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
            is_async: false,
            is_static: false,
            error_type: if has_error {
                Some("Box<dyn std::error::Error + Send + Sync>".to_string())
            } else {
                None
            },
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
    fn vtable_struct_is_repr_c() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![make_method("run", TypeRef::Unit, false)]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_vtable_struct(&spec);
        assert!(out.contains("#[repr(C)]"), "vtable must be #[repr(C)]");
        assert!(out.contains("pub struct MlBackendVTable"), "vtable name must match");
    }

    #[test]
    fn vtable_struct_has_free_user_data() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![make_method("run", TypeRef::Unit, false)]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_vtable_struct(&spec);
        assert!(out.contains("pub free_user_data:"), "vtable must have free_user_data");
    }

    #[test]
    fn vtable_struct_send_sync() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_vtable_struct(&spec);
        assert!(
            out.contains("unsafe impl Send for MlBackendVTable"),
            "vtable must be Send"
        );
        assert!(
            out.contains("unsafe impl Sync for MlBackendVTable"),
            "vtable must be Sync"
        );
    }

    #[test]
    fn bridge_struct_has_required_fields() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_bridge_struct(&spec);
        assert!(out.contains("vtable: MlBackendVTable"), "must hold vtable");
        assert!(
            out.contains("user_data: *const std::ffi::c_void"),
            "must hold user_data"
        );
        assert!(out.contains("cached_name: String"), "must hold cached_name");
    }

    #[test]
    fn bridge_drop_calls_free_user_data() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_bridge_drop(&spec);
        assert!(out.contains("impl Drop for MlBackendBridge"), "must impl Drop");
        assert!(out.contains("free_user_data"), "Drop must invoke free_user_data");
    }

    #[test]
    fn gen_ffi_plugin_impl_returns_none_without_super_trait() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Backend");
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        assert!(
            generator.gen_ffi_plugin_impl(&spec).is_none(),
            "must return None when no super_trait configured"
        );
    }

    #[test]
    fn gen_ffi_plugin_impl_generates_methods_with_super_trait() {
        let generator = make_generator();
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "Backend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: None,
            register_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_ffi_plugin_impl(&spec).expect("must produce Some");
        assert!(
            out.contains("impl my_lib::Plugin for MlBackendBridge"),
            "must impl correct path"
        );
        assert!(out.contains("fn name(&self)"), "must have name()");
        assert!(out.contains("fn initialize(&self)"), "must have initialize()");
        assert!(out.contains("fn shutdown(&self)"), "must have shutdown()");
    }
}
