//! Vtable struct, bridge struct, Drop impl, and Plugin super-trait impl generation.

use crate::codegen::generators::trait_bridge::TraitBridgeSpec;

use super::FfiBridgeGenerator;

impl FfiBridgeGenerator {
    /// Generate the vtable struct definition.
    pub(super) fn gen_vtable_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        out.push_str(&crate::backends::ffi::template_env::render(
            "vtable_struct_header.jinja",
            minijinja::context! {
                trait_name => &spec.trait_def.name,
                vtable_name => &vtable,
            },
        ));

        if spec.bridge_config.super_trait.is_some() {
            out.push_str(&crate::backends::ffi::template_env::render(
                "vtable_super_trait_methods.jinja",
                minijinja::context! {},
            ));
        }

        let skip = &spec.bridge_config.ffi_skip_methods;
        let own_methods: Vec<_> = spec
            .trait_def
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none() && !skip.iter().any(|s| s == &m.name))
            .collect();

        for method in &own_methods {
            if !method.doc.is_empty() {
                let method_doc_lines: Vec<&str> = method
                    .doc
                    .lines()
                    .map(|line| line.trim_start_matches("///").trim_start())
                    .collect();
                out.push_str(&crate::backends::ffi::template_env::render(
                    "vtable_method_doc_lines.jinja",
                    minijinja::context! {
                        doc_lines => method_doc_lines,
                    },
                ));
            }
            let mut params = vec!["user_data: *const std::ffi::c_void".to_string()];
            for p in &method.params {
                let cty = Self::c_param_type(&p.ty);
                params.push(format!("{}: {}", p.name, cty));
                if matches!(p.ty, crate::core::ir::TypeRef::Bytes) {
                    params.push(format!("{}_len: usize", p.name));
                }
            }
            let has_error = method.error_type.is_some();
            let (out_params, ret_ty) = Self::c_return_convention(&method.return_type, has_error);
            params.extend(out_params);

            out.push_str(&crate::backends::ffi::template_env::render(
                "vtable_method_field.jinja",
                minijinja::context! {
                    method_name => &method.name,
                    params_str => params.join(", "),
                    ret_ty => ret_ty,
                },
            ));
        }

        let vtable = self.vtable_name(spec);
        out.push_str(&crate::backends::ffi::template_env::render(
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

        let slice_cache_fields: Vec<String> = spec
            .required_methods()
            .into_iter()
            .filter(|m| m.returns_ref && matches!(&m.return_type, crate::core::ir::TypeRef::Vec(_)))
            .map(|m| format!("    {}_strs: &'static [&'static str],\n", m.name))
            .collect();

        let extra_fields = slice_cache_fields.join("");

        crate::backends::ffi::template_env::render(
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

        crate::backends::ffi::template_env::render(
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

        out.push_str(&crate::backends::ffi::template_env::render(
            "plugin_impl_header.jinja",
            minijinja::context! {
                super_trait_path => &super_trait_path,
                bridge_name => &bridge,
            },
        ));

        out.push_str(&crate::backends::ffi::template_env::render(
            "plugin_impl_version.jinja",
            minijinja::context! {},
        ));

        let plugin_error_expr = self
            .plugin_error_constructor
            .clone()
            .unwrap_or_else(|| format!("<{core_import}::{error_type} as ::core::convert::From<String>>::from(msg)"));

        out.push_str(&crate::backends::ffi::template_env::render(
            "plugin_impl_initialize.jinja",
            minijinja::context! {
                core_import => core_import,
                error_type => error_type,
                plugin_error_expr => plugin_error_expr,
            },
        ));

        out.push_str(&crate::backends::ffi::template_env::render(
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
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
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
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_generator() -> FfiBridgeGenerator {
        FfiBridgeGenerator {
            prefix: "ml".to_string(),
            core_import: "my_lib".to_string(),
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
            plugin_error_constructor: None,
            lifetime_type_names: std::collections::HashSet::new(),
        }
    }

    fn make_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "my_lib",
            wrapper_prefix: "Ml",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
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
        assert!(
            out.contains("pub free_string:"),
            "vtable must have callback string destructor"
        );
        assert!(out.contains("pub free_user_data:"), "vtable must have free_user_data");
    }

    #[test]
    fn plugin_string_callbacks_are_status_returning_with_out_error() {
        let generator = make_generator();
        let mut bridge_cfg = make_bridge_cfg("Backend");
        bridge_cfg.super_trait = Some("Plugin".to_string());
        let trait_def = make_trait_def("Backend", vec![]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_vtable_struct(&spec);
        assert!(
            out.contains(
                "pub name_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_name: *mut *mut std::ffi::c_char, out_error: *mut *mut std::ffi::c_char) -> i32>"
            ),
            "name_fn ABI must include out_error and status return:\n{out}"
        );
        assert!(
            out.contains(
                "pub version_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_version: *mut *mut std::ffi::c_char, out_error: *mut *mut std::ffi::c_char) -> i32>"
            ),
            "version_fn ABI must include out_error and status return:\n{out}"
        );
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
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
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
        assert!(
            out.contains("<my_lib::MyError as ::core::convert::From<String>>::from(msg)"),
            "default plugin error path must use From<String> fallback;\n\
             actual:\n{out}"
        );
        assert!(
            !out.contains("SampleCrateError::Plugin"),
            "default emission must not embed downstream-specific sample_crate literals;\n\
             actual:\n{out}"
        );
    }

    /// Regression (#114): vtable struct field for a Bytes parameter must emit both
    /// `{name}: *const u8` and a companion `{name}_len: usize`.  Binary payloads
    /// can contain embedded NUL bytes (0x00), so the C callee must use an explicit
    /// length rather than a NUL-terminated scan.
    #[test]
    fn vtable_struct_bytes_param_emits_len_companion() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg("Processor");
        let mut method = make_method("process", TypeRef::Unit, false);
        method.params.push(crate::core::ir::ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::Bytes,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        });
        let trait_def = make_trait_def("Processor", vec![method]);
        let spec = make_spec(&trait_def, &bridge_cfg);

        let out = generator.gen_vtable_struct(&spec);
        assert!(
            out.contains("payload: *const u8"),
            "vtable Bytes param must emit `payload: *const u8`;\nactual:\n{out}"
        );
        assert!(
            out.contains("payload_len: usize"),
            "vtable Bytes param must emit companion `payload_len: usize`;\nactual:\n{out}"
        );
    }

    /// When the FFI config provides an explicit `plugin_error_constructor`
    /// expression, the plugin shim emits that verbatim instead of the
    /// `From<String>` fallback. This is the sample_core compatibility path —
    /// sample_core's `SampleCrateError::Plugin` is a struct variant with two
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
            lifetime_type_names: std::collections::HashSet::new(),
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
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
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
