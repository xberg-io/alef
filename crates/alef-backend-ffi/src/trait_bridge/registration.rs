//! Constructor generation and extern "C" registration/unregistration functions.

use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use alef_core::ir::{MethodDef, TypeRef};

use super::FfiBridgeGenerator;

impl FfiBridgeGenerator {
    /// Generate the `impl {Bridge} { pub unsafe fn new(...) }` constructor block.
    ///
    /// For required methods that return `&[T]` (IR: `Vec(T)` + `returns_ref = true`), an
    /// extra cache-and-leak block is emitted: the vtable fn pointer is called once at
    /// construction, the JSON result is deserialised into a `Vec<String>`, each string is
    /// leaked into a `'static str`, and the resulting slice is stored in the bridge struct.
    /// The trait impl then returns from that field directly, satisfying the `&[&str]` return
    /// type without any per-call vtable overhead or lifetime gymnastics.
    pub(super) fn gen_constructor_impl(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);
        let vtable = self.vtable_name(spec);

        // Collect required methods with `Vec(T) + returns_ref` that need a cache field.
        let slice_cache_methods: Vec<&alef_core::ir::MethodDef> = spec
            .required_methods()
            .into_iter()
            .filter(|m| m.returns_ref && matches!(&m.return_type, alef_core::ir::TypeRef::Vec(_)))
            .collect();

        if slice_cache_methods.is_empty() {
            return crate::template_env::render(
                "constructor_impl.jinja",
                minijinja::context! {
                    bridge_name => &bridge,
                    vtable_name => &vtable,
                },
            );
        }

        // Build the slice-cache initialisation blocks and the field initialisers.
        let mut cache_init_blocks = String::new();
        let mut field_inits = String::new();

        for method in &slice_cache_methods {
            let fname = &method.name;
            let field = format!("{fname}_strs");

            // Emit the block that calls the vtable fn once and leaks the result into
            // `&'static [&'static str]`.  The block is safe to emit in an `unsafe fn`
            // context; individual unsafe sub-expressions are annotated inline.
            cache_init_blocks.push_str(&crate::template_env::render(
                "constructor_slice_cache_init.jinja",
                minijinja::context! {
                    method_name => fname,
                    field_name => &field,
                },
            ));

            field_inits.push_str(&crate::template_env::render(
                "constructor_field_init.jinja",
                minijinja::context! {
                    field => &field,
                },
            ));
        }

        // Generate the constructor with cache init blocks injected before `Self { ... }`.
        crate::template_env::render(
            "constructor_impl_with_cache.jinja",
            minijinja::context! {
                bridge_name => &bridge,
                vtable_name => &vtable,
                cache_init_blocks => &cache_init_blocks,
                field_inits => &field_inits,
            },
        )
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

        // --- register function header ---
        out.push_str(&crate::template_env::render(
            "register_fn_header.jinja",
            minijinja::context! {
                trait_name => &spec.trait_def.name,
                full_register_name => &full_register_name,
                vtable_name => &vtable,
            },
        ));

        // Validate required fn pointers (non-default methods must be non-null)
        for method in spec.required_methods() {
            out.push_str(&crate::template_env::render(
                "register_fn_vtable_check.jinja",
                minijinja::context! {
                    method_name => &method.name,
                },
            ));
        }

        // --- register function body ---
        let register_call = if let Some(extra_args) = &spec.bridge_config.register_extra_args {
            format!("registry.register(arc, {extra_args})")
        } else {
            "registry.register(arc)".to_string()
        };

        out.push_str(&crate::template_env::render(
            "register_fn_body.jinja",
            minijinja::context! {
                bridge_name => &bridge,
                trait_path => &trait_path,
                registry_getter => registry_getter,
                register_call => &register_call,
            },
        ));

        out.push('\n');

        // --- unregister function ---
        out.push_str(&crate::template_env::render(
            "unregister_fn.jinja",
            minijinja::context! {
                full_unregister_name => &full_unregister_name,
                registry_getter => registry_getter,
            },
        ));

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
        // `inside_closure = false`: errors must use the trait's actual error type,
        // not `Box<dyn Error>`, because the body is emitted directly in the trait impl.
        self.gen_vtable_call_body(method, spec, false)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // Short-circuit: slice-cache methods (returns_ref + Vec) return from a pre-populated
        // `&'static [&'static str]` field.  No async work is needed.
        if method.returns_ref && matches!(&method.return_type, TypeRef::Vec(_)) {
            return format!("self.{}_strs\n", method.name);
        }

        // For async methods we block-on inside a spawn_blocking call, mirroring
        // the Go/Java/C# strategy of running synchronous C callbacks on a thread pool.
        // The sync body references `self.vtable.*` and `self.user_data`; inside the
        // closure we have local `vtable` / `user_data` bindings instead.
        let sync_body = self
            // `inside_closure = true`: the body runs inside a `_SendFn` closure
            // whose return type is `Box<dyn Error + Send + Sync>`, so `Box::from` is correct.
            .gen_vtable_call_body(method, spec, true)
            .replace("self.vtable.", "vtable.")
            .replace("self.user_data", "user_data");
        let has_error = method.error_type.is_some();
        let core_import = &self.core_import;
        let method_name = &method.name;
        let _cached_name_clone = if has_error {
            "let _cached_name = self.cached_name.clone();\n"
        } else {
            ""
        };

        let _vtable_name = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        // *const c_void is !Send, but the caller guarantees thread-safety via the vtable
        // API contract. Wrap the entire closure in a Send newtype to bypass the check.
        out.push_str(
            "struct _SendFn<F>(F);
",
        );
        out.push_str("// SAFETY: caller guarantees vtable fn pointers and user_data are valid across threads.\n");
        out.push_str(
            "unsafe impl<F> Send for _SendFn<F> {}
",
        );
        out.push_str(
            "impl<F: FnOnce() -> R, R> _SendFn<F> {
",
        );
        out.push_str(
            "    fn call(self) -> R { (self.0)() }
",
        );
        out.push_str(
            "}
",
        );
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "ffi_async_cached_name_init.jinja",
            minijinja::context! {
                has_cached_name => has_error,
            },
        ));
        out.push_str(
            "let user_data = self.user_data;
",
        );
        for p in &method.params {
            let clone_expr = match &p.ty {
                TypeRef::Path => format!("{}.to_path_buf()", p.name),
                TypeRef::Bytes => format!("{}.to_vec()", p.name),
                // &str is not Clone-into-owned by itself — `.clone()` returns &str (same lifetime).
                // Use `.to_string()` so the closure captures an owned String that is 'static.
                TypeRef::String | TypeRef::Char if p.is_ref => format!("{}.to_string()", p.name),
                _ => format!("{}.clone()", p.name),
            };
            out.push_str(&crate::template_env::render(
                "ffi_async_capture_param.jinja",
                minijinja::context! {
                    param_name => &p.name,
                    expr => &clone_expr,
                },
            ));
        }
        out.push('\n');

        out.push_str(
            "let _task = _SendFn(move || {
",
        );
        out.push_str(
            "    // Inline the sync body:
",
        );
        for line in sync_body.lines() {
            out.push_str(&crate::template_env::render(
                "ffi_async_body_indent.jinja",
                minijinja::context! {
                    line => line,
                },
            ));
        }
        out.push_str(
            "});
",
        );
        out.push_str(
            "tokio::task::spawn_blocking(move || _task.call())
",
        );
        out.push_str(
            ".await
",
        );
        if has_error {
            let inner_error_constructor = spec.make_error("e.to_string()");
            out.push_str(&crate::template_env::render(
                "ffi_async_map_err_method.jinja",
                minijinja::context! {
                    core_import => &core_import,
                    method_name => &method_name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "ffi_async_box_error_map.jinja",
                minijinja::context! {
                    inner_error_constructor => &inner_error_constructor,
                },
            ));
        } else {
            out.push_str(
                ".unwrap_or_else(|_| Default::default())
",
            );
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
