//! Vtable struct, bridge struct, Drop impl, and Plugin super-trait impl generation.

use alef_codegen::generators::trait_bridge::TraitBridgeSpec;

use super::FfiBridgeGenerator;

impl FfiBridgeGenerator {
    /// Generate the vtable struct definition.
    pub(super) fn gen_vtable_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        out.push_str(&crate::template_env::render(
            "vtable_struct_header.jinja",
            minijinja::context! {
                trait_name => &spec.trait_def.name,
                vtable_name => &vtable,
            },
        ));

        // Super-trait methods (Plugin: name, version, initialize, shutdown)
        if spec.bridge_config.super_trait.is_some() {
            out.push_str(&crate::template_env::render(
                "vtable_super_trait_methods.jinja",
                minijinja::context! {},
            ));
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
                        out.push_str(
                            "    ///
",
                        );
                    } else {
                        out.push_str(&crate::template_env::render(
                            "vtable_method_doc_line.jinja",
                            minijinja::context! {
                                doc_line => stripped,
                            },
                        ));
                    }
                }
            }
            // Build params and return type inline for the template
            let mut params = vec!["user_data: *const std::ffi::c_void".to_string()];
            for p in &method.params {
                let cty = Self::c_param_type(&p.ty);
                params.push(format!("{}: {}", p.name, cty));
            }
            let has_error = method.error_type.is_some();
            let (out_params, ret_ty) = Self::c_return_convention(&method.return_type, has_error);
            params.extend(out_params);

            out.push_str(&crate::template_env::render(
                "vtable_method_field.jinja",
                minijinja::context! {
                    method_name => &method.name,
                    params_str => params.join(", "),
                    ret_ty => ret_ty,
                },
            ));
        }

        // free_user_data destructor, struct close, and Send + Sync
        let vtable = self.vtable_name(spec);
        out.push_str(&crate::template_env::render(
            "vtable_free_user_data.jinja",
            minijinja::context! {
                vtable_name => &vtable,
            },
        ));
        out
    }

    /// Generate the bridge struct with `vtable`, `user_data`, `cached_name`, and `cached_version`.
    ///
    /// For required trait methods that return `&[T]` (represented as `TypeRef::Vec(T)` with
    /// `returns_ref = true`), an extra `{method_name}_strs: &'static [&'static str]` field is
    /// emitted.  The values are populated once at construction time and returned directly from the
    /// trait impl — avoiding per-call vtable round-trips and satisfying the borrowed return type.
    pub(super) fn gen_bridge_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let bridge = self.bridge_name(spec);

        // Detect required methods with a `&[T]` return (Vec(T) + returns_ref = true).
        // Only `Vec(String)` → `&[&str]` is supported; other element types degrade gracefully.
        let slice_cache_fields: Vec<String> = spec
            .required_methods()
            .into_iter()
            .filter(|m| m.returns_ref && matches!(&m.return_type, alef_core::ir::TypeRef::Vec(_)))
            .map(|m| format!("    {}_strs: &'static [&'static str],\n", m.name))
            .collect();

        let extra_fields = slice_cache_fields.join("");

        crate::template_env::render(
            "bridge_struct.jinja",
            minijinja::context! {
                trait_name => &spec.trait_def.name,
                bridge_name => &bridge,
                vtable_name => &vtable,
                extra_fields => extra_fields,
            },
        )
    }

    /// Generate the `Drop` impl that calls `free_user_data` if non-null.
    pub(super) fn gen_bridge_drop(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);

        crate::template_env::render(
            "bridge_drop.jinja",
            minijinja::context! {
                bridge_name => &bridge,
            },
        )
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

        let error_type = &self.error_type;

        // plugin_impl_header
        out.push_str(&crate::template_env::render(
            "plugin_impl_header.jinja",
            minijinja::context! {
                super_trait_path => &super_trait_path,
                bridge_name => &bridge,
            },
        ));

        // plugin_impl_version
        out.push_str(&crate::template_env::render(
            "plugin_impl_version.jinja",
            minijinja::context! {},
        ));

        // The configured plugin_error_constructor takes precedence; otherwise
        // fall back to a generic `core_import::error_type::from(msg)` shape
        // (works for any error type that implements `From<String>`).
        let plugin_error_expr = self
            .plugin_error_constructor
            .clone()
            .unwrap_or_else(|| format!("<{core_import}::{error_type} as ::core::convert::From<String>>::from(msg)"));

        // plugin_impl_initialize
        out.push_str(&crate::template_env::render(
            "plugin_impl_initialize.jinja",
            minijinja::context! {
                core_import => core_import,
                error_type => error_type,
                plugin_error_expr => plugin_error_expr,
            },
        ));

        // plugin_impl_shutdown
        out.push_str(&crate::template_env::render(
            "plugin_impl_shutdown.jinja",
            minijinja::context! {
                core_import => core_import,
                error_type => error_type,
                plugin_error_expr => plugin_error_expr,
            },
        ));

        Some(out)
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

            unregister_fn: None,

            clear_fn: None,
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
            plugin_error_constructor: None,
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

            unregister_fn: None,

            clear_fn: None,
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
        // Default (no plugin_error_constructor configured) emits the generic
        // `From<String>` fallback so non-kreuzberg downstreams compile.
        assert!(
            out.contains("<my_lib::MyError as ::core::convert::From<String>>::from(msg)"),
            "default plugin error path must use From<String> fallback;\n\
             actual:\n{out}"
        );
        assert!(
            !out.contains("KreuzbergError::Plugin"),
            "default emission must not embed downstream-specific kreuzberg literals;\n\
             actual:\n{out}"
        );
    }

    /// When the FFI config provides an explicit `plugin_error_constructor`
    /// expression, the plugin shim emits that verbatim instead of the
    /// `From<String>` fallback. This is the kreuzberg compatibility path —
    /// kreuzberg's `KreuzbergError::Plugin` is a struct variant with two
    /// fields and cannot be constructed via `From<String>`.
    #[test]
    fn gen_ffi_plugin_impl_uses_configured_plugin_error_constructor() {
        let generator = FfiBridgeGenerator {
            prefix: "ml".to_string(),
            core_import: "my_lib".to_string(),
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
            plugin_error_constructor: Some(
                "my_lib::MyError::Plugin { message: msg, plugin_name: String::new() }".to_string(),
            ),
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "Backend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: None,
            register_fn: None,
            unregister_fn: None,
            clear_fn: None,
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
            out.contains("my_lib::MyError::Plugin { message: msg, plugin_name: String::new() }"),
            "plugin shim must inline the configured constructor verbatim;\n\
             actual:\n{out}"
        );
        assert!(
            !out.contains("From<String>"),
            "configured constructor takes precedence over the From<String> fallback;\n\
             actual:\n{out}"
        );
    }
}
