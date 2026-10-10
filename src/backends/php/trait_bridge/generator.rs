//! PHP (ext-php-rs) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to PHP objects via ext-php-rs Zval method calls.

use crate::codegen::generators::trait_bridge::{BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// PHP-specific trait bridge generator.
/// Implements code generation for bridging PHP objects to Rust traits.
pub struct PhpBridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
    pub error_type: String,
    /// Callback-param type names that get NATIVE-object marshalling — known serde structs per
    /// the shared [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] rule.
    /// For such a param the bridge constructs the binding's native PHP object (the `#[php_class]`
    /// wrapper, via the same `From<core::T>` conversion used for return values) and hands THAT to
    /// the PHP method as a `Zval`, instead of serializing the param to a JSON string. Enums,
    /// opaque/handle types, and excluded/unknown `Named` params are absent and keep their prior
    /// JSON-string representation.
    pub struct_param_types: std::collections::HashSet<String>,
    /// Callback-RETURN type names that get NATIVE-object marshalling — known serde structs returned
    /// directly by a method, gated to those with a generated `From<Binding> for core` conversion.
    /// For such a return the bridge first extracts the host's native `#[php_class]` object via
    /// `FromZval` and converts via `From<Binding>`, falling back to the `val.string()` + serde path.
    pub struct_return_types: std::collections::HashSet<String>,
    /// Rust-defaulted trait methods the bridge forwards to the host when the PHP
    /// object's class defines them (per the shared `forwardable_defaulted_method_names`
    /// rule). Methods absent here keep the trait's Rust default unconditionally.
    pub forwardable_defaulted: std::collections::HashSet<String>,
}

/// Returns true if a `Zval` for `ty` can be produced directly via `IntoZval` (Rust -> PHP),
/// recursing into `Vec`. Used to decide when a trait-bridge callback argument can be passed as a
/// native PHP array/scalar instead of a Debug-string encoded one (#1304).
pub(super) fn is_php_encodable(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String => true,
        TypeRef::Primitive(_) => true,
        TypeRef::Vec(inner) => is_php_encodable(inner),
        _ => false,
    }
}

/// Returns true if a `Zval` for `ty` can be decoded directly via `FromZval` (PHP -> Rust),
/// recursing into `Vec`. Unlike napi-rs (which lacks `FromNapiValue for f32`), ext-php-rs
/// 0.15.15 implements `FromZval` for every `PrimitiveType` variant including `f32`
/// (`impl FromZval<'_> for f32` in `types/mod.rs`, converting the PHP `double` zval via
/// `zval.double().map(|v| v as f32)`), so no primitive needs to be excluded here — this
/// classifier is intentionally identical to [`is_php_encodable`].
pub(super) fn is_php_decodable(ty: &TypeRef) -> bool {
    is_php_encodable(ty)
}

impl PhpBridgeGenerator {
    /// Binding `#[php_class]` name to extract for a native-object return, when the return is a bare
    /// `Named` struct on the (conversion-gated) native-marshalled return allowlist. The bridge tries
    /// `FromZval` (which yields `Some` only for the host's native object) and converts via
    /// `From<Binding>` for core. `None` keeps the `val.string()` + serde path.
    fn native_struct_return<'a>(&self, ty: &'a TypeRef) -> Option<&'a str> {
        match ty {
            TypeRef::Named(n) if self.struct_return_types.contains(n) => Some(n.as_str()),
            _ => None,
        }
    }

    /// Build the `Zval` argument expression for one callback parameter.
    ///
    /// Known serde structs (per the shared allowlist) are handed to PHP as the binding's native
    /// `#[php_class]` object — constructed from the core value through the same `From<core::T>`
    /// conversion the binding uses for function return values (`{Class}::from((*name).clone())`),
    /// then boxed into a `ZendClassObject` and converted to a Zval via `IntoZval` — the bare
    /// `#[php_class]` struct is not itself `IntoZval`, only `ZBox<ZendClassObject<T>>` is. All
    /// other params keep their prior representation (JSON string for other `Named` types, etc.).
    fn arg_zval_expr(&self, p: &crate::core::ir::ParamDef) -> String {
        match &p.ty {
            TypeRef::String => format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name),
            TypeRef::Path => format!(
                "ext_php_rs::types::Zval::try_from({}.to_string_lossy().to_string()).unwrap_or_default()",
                p.name
            ),
            TypeRef::Bytes => format!(
                "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                p.name
            ),
            // bare `#[php_class]` struct is not `IntoZval`; box it in a `ZendClassObject` (which
            TypeRef::Named(n) if self.struct_param_types.contains(n.as_str()) => {
                let core_value = if p.is_ref {
                    format!("(*{}).clone()", p.name)
                } else {
                    p.name.clone()
                };
                format!(
                    "ext_php_rs::convert::IntoZval::into_zval(ext_php_rs::types::ZendClassObject::new({n}::from({core_value})), false).unwrap_or_default()"
                )
            }
            TypeRef::Named(_) => format!(
                "ext_php_rs::types::Zval::try_from(serde_json::to_string(&{}).unwrap_or_default()).unwrap_or_default()",
                p.name
            ),
            TypeRef::Primitive(_) => {
                format!("ext_php_rs::types::Zval::try_from({}).unwrap_or_default()", p.name)
            }
            // `Vec<T>` of a php-native `T` (String/primitive, recursively) is `IntoZval` in
            // ext-php-rs — pass it as a native PHP array instead of a Debug string (#1304).
            TypeRef::Vec(inner) if is_php_encodable(inner) => {
                let owned = if p.is_ref {
                    format!("{}.to_vec()", p.name)
                } else {
                    format!("{}.clone()", p.name)
                };
                format!("ext_php_rs::convert::IntoZval::into_zval({owned}, false).unwrap_or_default()")
            }
            _ => format!(
                "ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default()",
                p.name
            ),
        }
    }

    /// Render the `Vec<&dyn IntoZvalDyn>` args expression passed to `try_call_method`,
    /// or `vec![]` when the method takes no params.
    fn args_expr(&self, method: &MethodDef) -> String {
        if method.params.is_empty() {
            return "vec![]".to_string();
        }
        let args_parts: Vec<String> = method.params.iter().map(|p| self.arg_zval_expr(p)).collect();
        format!(
            "[{}].iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect()",
            args_parts.join(", ")
        )
    }
}

impl TraitBridgeGenerator for PhpBridgeGenerator {
    fn disabled_error(&self) -> Option<&'static str> {
        Some(super::DISABLED_MESSAGE)
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!(
            "{{\n    let __class = self.inner.object().and_then(|object| object.get_class_name().ok()).unwrap_or_default();\n    ext_php_rs::zend::Function::try_from_method(&__class, \"{}\").is_some()\n}}",
            method.name
        ))
    }

    fn gen_method_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        self.forwardable_defaulted.contains(&method.name).then(|| {
            format!(
                "{{\n    let __class = self.inner.object().and_then(|object| object.get_class_name().ok()).unwrap_or_default();\n    ext_php_rs::zend::Function::try_from_method(&__class, \"{}\").is_some()\n}}",
                method.name
            )
        })
    }

    fn foreign_object_type(&self) -> &str {
        "ext_php_rs::types::Zval"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        let args_expr = self.args_expr(method);

        let is_result_type = method.error_type.is_some();
        let is_unit_return = matches!(method.return_type, TypeRef::Unit);
        let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_));

        let return_type = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => crate::codegen::generators::trait_bridge::format_type_ref(other, &self.type_paths),
        };

        let deserialize_error_expr = spec.make_error("format!(\"Deserialize error: {}\", e)");
        let call_error_expr = spec.make_error("e.to_string()");
        // Native decode only applies to the generic (non-primitive, non-native-struct) branch: ~keep
        // `is_primitive_return` already decodes primitives directly via `val.long()`/`val.bool()`, ~keep
        // and `native_return_binding` already covers known serde structs. ~keep
        let native_decodable = !is_primitive_return && is_php_decodable(&method.return_type);
        let native_decode_error_expr = spec.make_error(&format!(
            "\"Failed to decode native PHP return value for method '{name}'\".to_string()"
        ));

        crate::backends::php::template_env::render(
            "sync_method_body.jinja",
            crate::alef_context! {
                wrapper => spec.wrapper_name(),
                method_name => name,
                args_expr => args_expr,
                is_result_type => is_result_type,
                is_unit_return => is_unit_return,
                is_primitive_return => is_primitive_return,
                return_type => return_type,
                native_return_binding => self.native_struct_return(&method.return_type),
                native_decodable => native_decodable,
                native_decode_error_expr => native_decode_error_expr,
                deserialize_error_expr => deserialize_error_expr,
                call_error_expr => call_error_expr,
            },
        )
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        let string_params: Vec<String> = method
            .params
            .iter()
            .filter(|p| matches!(&p.ty, TypeRef::String))
            .map(|p| p.name.clone())
            .collect();

        let args_expr = self.args_expr(method);

        let is_result_type = method.error_type.is_some();
        let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_));
        // Native decode only applies to the generic (non-primitive, non-native-struct) branch —
        // see the matching comment in `gen_sync_method_body`.
        let native_decodable = !is_primitive_return && is_php_decodable(&method.return_type);
        let return_type = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => crate::codegen::generators::trait_bridge::format_type_ref(other, &self.type_paths),
        };
        let deserialize_error_expr = spec.make_error("format!(\"Deserialize error: {}\", e)");
        let call_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e)"
        ));
        let native_decode_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' failed to decode native PHP return value for method '{name}'\", cached_name)"
        ));

        crate::backends::php::template_env::render(
            "async_method_body.jinja",
            crate::alef_context! {
                method_name => name,
                args_expr => args_expr,
                string_params => string_params,
                is_result_type => is_result_type,
                return_type => return_type,
                native_return_binding => self.native_struct_return(&method.return_type),
                native_decodable => native_decodable,
                native_decode_error_expr => native_decode_error_expr,
                deserialize_error_expr => deserialize_error_expr,
                call_error_expr => call_error_expr,
            },
        )
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();

        crate::backends::php::template_env::render(
            "bridge_constructor.jinja",
            crate::alef_context! {
                wrapper => &wrapper,
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);

        crate::backends::php::template_env::render(
            "bridge_unregister_fn.jinja",
            crate::alef_context! {
                unregister_fn => unregister_fn,
                host_path => &host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, clear_fn);

        crate::backends::php::template_env::render(
            "bridge_clear_fn.jinja",
            crate::alef_context! {
                clear_fn => clear_fn,
                host_path => &host_path,
            },
        )
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
                crate::alef_context! {
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

        crate::backends::php::template_env::render(
            "bridge_registration_fn.jinja",
            crate::alef_context! {
                register_fn => register_fn,
                required_methods => required_methods,
                wrapper => &wrapper,
                trait_path => &trait_path,
                registry_getter => registry_getter,
                extra_args => &extra_args,
            },
        )
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    _api: &ApiSurface,
) -> BridgeOutput {
    if bridge_cfg.php_callbacks_unsupported() {
        return gen_unsupported_lifecycle(trait_type, bridge_cfg, core_import, error_type, error_constructor);
    }
    BridgeOutput {
        imports: Vec::new(),
        code: super::disabled_code(),
    }
}

fn gen_unsupported_lifecycle(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
) -> BridgeOutput {
    let spec = TraitBridgeSpec {
        trait_def: trait_type,
        bridge_config: bridge_cfg,
        core_import,
        wrapper_prefix: "Php",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: error_type.to_string(),
        error_constructor: error_constructor.to_string(),
    };
    let mut blocks = Vec::new();

    if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
        let message = format!(
            "PHP callback bridge `{}` is unsupported: Zend values are request-thread-bound and cannot safely implement Send or Sync",
            bridge_cfg.trait_name
        );
        blocks.push(format!(
            "#[php_function]\npub fn {register_fn}(_backend: &mut ext_php_rs::types::ZendObject) -> ext_php_rs::prelude::PhpResult<()> {{\n    Err(ext_php_rs::exception::PhpException::from_message({message:?}.to_string()))\n}}"
        ));
    }
    if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(&spec, unregister_fn);
        blocks.push(crate::backends::php::template_env::render(
            "bridge_unregister_fn.jinja",
            crate::alef_context! {
                unregister_fn => unregister_fn,
                host_path => &host_path,
            },
        ));
    }
    if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(&spec, clear_fn);
        blocks.push(crate::backends::php::template_env::render(
            "bridge_clear_fn.jinja",
            crate::alef_context! {
                clear_fn => clear_fn,
                host_path => &host_path,
            },
        ));
    }

    BridgeOutput {
        imports: Vec::new(),
        code: blocks.join("\n\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_generator_fails_closed_through_generic_entrypoint() {
        let generator = PhpBridgeGenerator {
            core_import: "sample_core".to_string(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_string(),
            struct_param_types: std::collections::HashSet::new(),
            struct_return_types: std::collections::HashSet::new(),
            forwardable_defaulted: std::collections::HashSet::new(),
        };
        let trait_def = crate::core::ir::TypeDef {
            name: "Handler".to_string(),
            rust_path: "sample_core::Handler".to_string(),
            is_trait: true,
            is_opaque: true,
            ..Default::default()
        };
        let bridge = crate::core::config::TraitBridgeConfig {
            trait_name: "Handler".to_string(),
            register_fn: Some("register_handler".to_string()),
            registry_getter: Some("sample_core::get_handlers".to_string()),
            ..Default::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge,
            core_import: "sample_core",
            wrapper_prefix: "Php",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
            error_type: "SampleError".to_string(),
            error_constructor: "SampleError::from({msg})".to_string(),
        };
        let output = crate::codegen::generators::trait_bridge::gen_bridge_all(&spec, &generator);
        assert!(output.imports.is_empty());
        assert!(output.code.contains("compile_error!"));
        for forbidden in ["Zval", "unsafe impl Send", "unsafe impl Sync", "Arc<dyn", "Arc::new"] {
            assert!(
                !output.code.contains(forbidden),
                "unsafe token `{forbidden}` in: {}",
                output.code
            );
        }

        let components = [
            crate::codegen::generators::trait_bridge::gen_bridge_wrapper_struct(&spec, &generator),
            crate::codegen::generators::trait_bridge::gen_bridge_trait_impl(&spec, &generator),
            crate::codegen::generators::trait_bridge::gen_bridge_registration_fn(&spec, &generator)
                .expect("disabled registration must emit an explicit error"),
        ];
        for component in components {
            assert!(component.contains("compile_error!"));
            for forbidden in ["Zval", "unsafe impl Send", "unsafe impl Sync", "Arc<dyn", "Arc::new"] {
                assert!(
                    !component.contains(forbidden),
                    "unsafe token `{forbidden}` in: {component}"
                );
            }
        }
    }

    #[test]
    fn presence_check_emitted_only_for_forwardable_defaulted_methods() {
        let mut generator = PhpBridgeGenerator {
            core_import: "sample_core".to_string(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_string(),
            struct_param_types: std::collections::HashSet::new(),
            struct_return_types: std::collections::HashSet::new(),
            forwardable_defaulted: std::collections::HashSet::new(),
        };
        let trait_def = crate::core::ir::TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "sample_core::OcrBackend".to_string(),
            is_trait: true,
            is_opaque: true,
            ..Default::default()
        };
        let bridge = crate::core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            ..crate::core::config::TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge,
            core_import: "sample_core",
            wrapper_prefix: "Php",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
            error_type: "SampleError".to_string(),
            error_constructor: "SampleError::Message { message: {msg} }".to_string(),
        };
        let method = crate::core::ir::MethodDef {
            name: "supports_table_detection".to_string(),
            has_default_impl: true,
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        assert!(generator.gen_method_presence_check(&method, &spec).is_none());
        generator
            .forwardable_defaulted
            .insert("supports_table_detection".to_string());
        let check = generator.gen_method_presence_check(&method, &spec).unwrap();
        assert!(
            check.contains("Function::try_from_method"),
            "php presence check must look the method up on the class: {check}"
        );
        assert!(
            !check.contains("try_call_method"),
            "presence must not invoke the method: {check}"
        );
    }
}
