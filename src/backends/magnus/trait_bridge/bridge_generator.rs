use super::visitor_bridge::gen_visitor_bridge;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec, gen_bridge_all};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<String> {
    if !crate::codegen::generators::trait_bridge::bridge_targets_language(
        bridge_cfg,
        &crate::backends::magnus::trait_bridge::TARGET_SPELLINGS,
    ) {
        return Ok(String::new());
    }

    let trait_path = trait_type.rust_path.replace('-', "_");

    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Rb", bridge_cfg);
        let mut out = String::with_capacity(8192);
        gen_visitor_bridge(
            &mut out,
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
            api,
        )?;
        Ok(out)
    } else {
        // value (the `#[magnus::wrap]` struct, built via the same `From<core::T>` conversion used
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let struct_return_types: std::collections::HashSet<String> =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_returns(trait_type, api)
                .into_iter()
                .filter(|name| binding_to_core.contains(name.as_str()))
                .collect();
        let forwardable_defaulted =
            crate::codegen::generators::trait_bridge::forwardable_defaulted_method_names(trait_type, api);
        let plugin_version_is_fallible = bridge_cfg
            .super_trait
            .as_deref()
            .and_then(|name| {
                let short_name = name.rsplit("::").next().unwrap_or(name);
                api.types.iter().find(|typ| typ.is_trait && typ.name == short_name)
            })
            .and_then(|typ| typ.methods.iter().find(|method| method.name == "version"))
            .is_some_and(|method| method.error_type.is_some());
        let generator = MagnusBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
            struct_param_types,
            struct_return_types,
            forwardable_defaulted,
            plugin_version_is_fallible,
        };
        let lifetime_type_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|typ| typ.has_lifetime_params)
            .map(|typ| typ.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Rb",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let output = gen_bridge_all(&spec, &generator);
        let mut prefixed = String::with_capacity(output.imports.len() * 64 + output.code.len());
        let imports_to_emit: Vec<_> = output
            .imports
            .iter()
            .filter(|imp| *imp != "magnus::prelude::*")
            .collect();
        for imp in &imports_to_emit {
            prefixed.push_str("#[allow(unused_imports)]\n");
            prefixed.push_str("use ");
            prefixed.push_str(imp);
            prefixed.push_str(" as _;\n");
        }
        prefixed.push_str(&generator.runtime_dispatcher_support(&spec));
        prefixed.push_str("\n\n");
        prefixed.push_str(&output.code);
        Ok(prefixed)
    }
}

/// Magnus-specific trait bridge generator.
/// Implements code generation for bridging Ruby objects to Rust traits.
struct MagnusBridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    type_paths: HashMap<String, String>,
    /// Canonical error type for the host crate (e.g. `"SampleCrateError"`).
    /// Used to construct Result return types matching the trait's signature.
    error_type: String,
    /// Error constructor template (e.g. `"SampleCrateError::Plugin {{ message: {msg}, plugin_name: String::new() }}"`).
    error_constructor: String,
    /// Callback-param type names that get NATIVE-object marshalling — known serde structs per the
    /// shared [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] rule. For
    /// such a param the bridge constructs the binding's native Ruby value (the `#[magnus::wrap]`
    /// struct, via the same `From<core::T>` conversion used for function return values / struct
    /// fields) and hands THAT to the host method, instead of serializing the param to a JSON
    /// string. Enums, opaque/handle types, and excluded/unknown `Named` params are absent and keep
    /// their prior JSON-string representation.
    struct_param_types: std::collections::HashSet<String>,
    /// Callback-RETURN type names that get NATIVE-object marshalling — known serde structs returned
    /// directly by a method (per the shared `native_marshalled_struct_returns` rule). For such a
    /// return the bridge routes the value through the binding struct's `TryConvert` (which accepts
    /// the native wrapped object as well as a Hash/JSON via `to_json`) and converts via
    /// `From<Binding> for core`, instead of `serde_json::from_str` into core directly.
    struct_return_types: std::collections::HashSet<String>,
    /// Rust-defaulted trait methods the bridge forwards to the host when the Ruby
    /// object responds to them. Presence is cached as `has_<method>` bool fields at
    /// construction (under the GVL) because async bridge bodies run on worker
    /// threads where the Ruby object cannot be probed. Methods absent here keep
    /// the trait's Rust default unconditionally.
    forwardable_defaulted: std::collections::HashSet<String>,
    /// Whether the extracted plugin super-trait returns `Result` from `version`.
    plugin_version_is_fallible: bool,
}

impl MagnusBridgeGenerator {
    fn runtime_dispatcher_name(&self, spec: &TraitBridgeSpec) -> String {
        format!("{}RuntimeDispatcher", spec.wrapper_name())
    }

    fn runtime_job_name(&self, spec: &TraitBridgeSpec) -> String {
        format!("{}RuntimeJob", spec.wrapper_name())
    }

    fn runtime_dispatcher_support(&self, spec: &TraitBridgeSpec) -> String {
        crate::backends::magnus::template_env::render(
            "trait_bridge_runtime_dispatcher.rs.jinja",
            minijinja::context! {
                dispatcher_name => self.runtime_dispatcher_name(spec),
                job_name => self.runtime_job_name(spec),
            },
        )
    }

    /// Build the fully-qualified error path (`{core_import}::{error_type}` unless already qualified).
    fn error_path(&self) -> String {
        if self.error_type.contains("::") || self.error_type.contains('<') {
            self.error_type.clone()
        } else {
            format!("{}::{}", self.core_import, self.error_type)
        }
    }

    /// Build an error construction expression from a message expression.
    fn make_error(&self, msg_expr: &str) -> String {
        self.error_constructor.replace("{msg}", msg_expr)
    }
}

impl TraitBridgeGenerator for MagnusBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "magnus::value::Opaque<magnus::Value>"
    }

    fn gen_method_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        self.forwardable_defaulted
            .contains(&method.name)
            .then(|| format!("self.has_{}", method.name))
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!("self.has_{}", method.name))
    }

    fn plugin_version_is_fallible(&self) -> bool {
        self.plugin_version_is_fallible
    }

    fn extra_bridge_fields(&self, spec: &TraitBridgeSpec) -> Vec<(String, String)> {
        let mut fields: Vec<(String, String)> = spec
            .trait_def
            .methods
            .iter()
            .filter(|m| self.forwardable_defaulted.contains(&m.name))
            .map(|m| (format!("has_{}", m.name), "bool".to_string()))
            .collect();
        if spec.bridge_config.super_trait.is_some() {
            fields.push(("has_initialize".to_string(), "bool".to_string()));
            fields.push(("has_shutdown".to_string(), "bool".to_string()));
        }
        fields.push(("runtime_dispatcher".to_string(), self.runtime_dispatcher_name(spec)));
        fields
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "magnus::value::InnerValue".to_string(),
            "magnus::TryConvert".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);
        // `self.runtime_dispatcher.dispatch` requires `Send + 'static`, so every param still
        // needs an owned binding moved into the shared callback even on the sync fast path.
        // Unlike the async body, the sync callback is not itself moved into a further
        // `spawn_blocking` closure, so the owned binding can reuse the parameter's own name
        // instead of a `_owned` suffix. ~keep
        let conversion_bindings = self.owned_param_bindings(method, "");

        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| self.ruby_arg_expr_custom(&p.ty, &p.name))
            .collect();

        let call = if args.is_empty() {
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", ())")
        } else {
            let args_tuple = if args.len() == 1 {
                format!("({},)", args[0])
            } else {
                format!("({})", args.join(", "))
            };
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", {args_tuple})")
        };

        let err_expr = if has_error {
            self.make_error(&format!("format!(\"Ruby method '{name}' failed: {{}}\", e)"))
        } else {
            String::new()
        };

        let mut callback_body = crate::backends::magnus::template_env::render(
            "sync_method_body.rs.jinja",
            minijinja::context! {
                wrapper => spec.wrapper_name(),
                method_name => name,
                call => call,
                has_error => has_error,
                is_unit => is_unit,
                err_expr => err_expr,
            },
        );

        if !is_unit {
            callback_body.push_str(&self.return_conversion(method, spec, has_error, ""));
        }

        let result_type = self.method_result_type(method);
        let indented_callback = callback_body
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let dispatch_failure = if has_error {
            format!(
                "Err({})",
                self.make_error(&format!(
                    "format!(\"Ruby runtime dispatcher failed for method '{name}': {{}}\", error)"
                ))
            )
        } else {
            format!(
                "{{ tracing::warn!(wrapper = \"{}\", method = \"{name}\", %error, \"Ruby runtime dispatcher failed; returning default\"); Default::default() }}",
                spec.wrapper_name()
            )
        };

        format!(
            "{conversion_bindings}let callback = move |ruby: &magnus::Ruby, value: magnus::Value| -> {result_type} {{\n\
             {indented_callback}\n\
             }};\n\
             match magnus::Ruby::get() {{\n\
                 Ok(ruby) => callback(&ruby, self.inner.get_inner_with(&ruby)),\n\
                 Err(_) => match self.runtime_dispatcher.dispatch(callback) {{\n\
                     Ok(result) => result,\n\
                     Err(error) => {dispatch_failure},\n\
                 }},\n\
             }}"
        )
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);

        // Unlike the sync body, this callback is moved again into a further `spawn_blocking`
        // closure below, so its owned bindings keep the `_owned` suffix distinguishing them
        // from the by-ref parameters of the enclosing trait method. ~keep
        let conversion_bindings = self.owned_param_bindings(method, "_owned");

        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| self.ruby_arg_expr_custom(&p.ty, &format!("{}_owned", p.name)))
            .collect();

        let call = if args.is_empty() {
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", ())")
        } else {
            let args_tuple = if args.len() == 1 {
                format!("({},)", args[0])
            } else {
                format!("({})", args.join(", "))
            };
            format!("value.funcall::<_, _, magnus::Value>(\"{name}\", {args_tuple})")
        };

        let err_expr_call = if has_error {
            self.make_error(&format!("format!(\"Ruby method '{name}' failed: {{}}\", e)"))
        } else {
            String::new()
        };
        let mut callback_body = crate::backends::magnus::template_env::render(
            "sync_method_body.rs.jinja",
            minijinja::context! {
                wrapper => spec.wrapper_name(),
                method_name => name,
                call => call,
                has_error => has_error,
                is_unit => is_unit,
                err_expr => err_expr_call,
            },
        );
        if !is_unit {
            callback_body.push_str(&self.return_conversion(method, spec, has_error, ""));
        }
        let indented_callback = callback_body
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result_ty = self.method_result_type(method);
        let dispatch_failure = if has_error {
            format!(
                "Err({})",
                self.make_error(&format!(
                    "format!(\"Ruby runtime dispatcher failed for method '{name}': {{}}\", error)"
                ))
            )
        } else {
            "{ tracing::warn!(%error, \"Ruby runtime dispatcher failed; returning default\"); Default::default() }"
                .to_string()
        };
        let join_failure = if has_error {
            format!(
                "Err({})",
                self.make_error(&format!(
                    "format!(\"Ruby runtime dispatcher task failed for method '{name}': {{}}\", error)"
                ))
            )
        } else {
            "{ tracing::warn!(%error, \"Ruby runtime dispatcher task failed; returning default\"); Default::default() }"
                .to_string()
        };

        format!(
            "{conversion_bindings}let dispatcher = self.runtime_dispatcher.clone();\n\
             let callback = move |ruby: &magnus::Ruby, value: magnus::Value| -> {result_ty} {{\n\
             {indented_callback}\n\
             }};\n\
             match tokio::task::spawn_blocking(move || dispatcher.dispatch(callback)).await {{\n\
                 Ok(Ok(result)) => result,\n\
                 Ok(Err(error)) => {dispatch_failure},\n\
                 Err(error) => {join_failure},\n\
             }}"
        )
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods: Vec<_> = spec.required_methods().iter().map(|m| m.name.as_str()).collect();
        let mut optional_methods: Vec<_> = spec
            .trait_def
            .methods
            .iter()
            .filter(|m| self.forwardable_defaulted.contains(&m.name))
            .map(|m| m.name.as_str())
            .collect();
        if spec.bridge_config.super_trait.is_some() {
            optional_methods.push("initialize");
            optional_methods.push("shutdown");
        }

        crate::backends::magnus::template_env::render(
            "trait_bridge_constructor.rs.jinja",
            minijinja::context! {
                wrapper => wrapper,
                required_methods => required_methods,
                optional_methods => optional_methods,
                dispatcher_name => self.runtime_dispatcher_name(spec),
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        let func = format!(
            "pub fn {unregister_fn}(name: String) -> Result<(), magnus::Error> {{\n\
             {host_path}(&name).map_err(|e| {{\n\
             let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};\n\
             magnus::Error::new(ruby.exception_runtime_error(), format!(\"{{}}\", e))\n\
             }})\n\
             }}\n"
        );
        func
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let mut out = String::with_capacity(512);
        let func = format!(
            "pub fn {clear_fn}() -> Result<(), magnus::Error> {{\n\
             {host_path}().map_err(|e| {{\n\
             let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};\n\
             magnus::Error::new(ruby.exception_runtime_error(), format!(\"{{}}\", e))\n\
             }})\n\
             }}\n"
        );
        out.push_str(&func);
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
        let required_methods: Vec<_> = spec
            .required_methods()
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect();
        let required_methods = required_methods.join(", ");

        let register_extra_args = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        crate::backends::magnus::template_env::render(
            "trait_bridge_registration_fn.rs.jinja",
            minijinja::context! {
                register_fn => register_fn,
                registry_getter => registry_getter,
                wrapper => wrapper,
                trait_path => trait_path,
                required_methods => required_methods,
                register_extra_args => register_extra_args,
            },
        )
    }
}

impl MagnusBridgeGenerator {
    fn method_result_type(&self, method: &MethodDef) -> String {
        let value = self.return_rust_type(&method.return_type);
        match method.error_type.as_ref() {
            Some(_) => format!("std::result::Result<{value}, {}>", self.error_path()),
            None => value,
        }
    }

    fn owned_param_bindings(&self, method: &MethodDef, suffix: &str) -> String {
        method
            .params
            .iter()
            .map(|param| {
                let conversion = if !param.is_ref {
                    param.name.clone()
                } else {
                    match &param.ty {
                        TypeRef::String => format!("{}.to_string()", param.name),
                        TypeRef::Bytes => format!("{}.to_vec()", param.name),
                        TypeRef::Path => format!("{}.to_path_buf()", param.name),
                        _ => format!("{}.clone()", param.name),
                    }
                };
                format!("let {}{suffix} = {conversion};\n", param.name)
            })
            .collect()
    }

    /// The fully-qualified Rust return type as it appears in the trait method
    /// signature — uses `core_import::Foo` for Named types.
    fn return_rust_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => {
                use crate::core::ir::PrimitiveType::*;
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
                .to_string()
            }
            TypeRef::String => "String".to_string(),
            TypeRef::Bytes => "Vec<u8>".to_string(),
            TypeRef::Vec(inner) => format!("Vec<{}>", self.return_rust_type(inner)),
            TypeRef::Optional(inner) => format!("Option<{}>", self.return_rust_type(inner)),
            TypeRef::Named(name) => self
                .type_paths
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(|| format!("{}::{}", self.core_import, name)),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Map(k, v) => format!(
                "std::collections::HashMap<{}, {}>",
                self.return_rust_type(k),
                self.return_rust_type(v)
            ),
            TypeRef::Json => "serde_json::Value".to_string(),
            TypeRef::Duration => "std::time::Duration".to_string(),
            TypeRef::Char => "char".to_string(),
            TypeRef::Path => "std::path::PathBuf".to_string(),
        }
    }

    /// Whether converting `ty` from a Ruby `magnus::Value` requires a JSON round-trip.
    /// True for any Named type or composite that contains a Named type — magnus's
    /// `TryConvert` is only implemented for primitives, String, Vec<T: TryConvert>,
    /// HashMap with TryConvert keys/values, and a few container types.
    fn needs_json_marshalling(&self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(_) | TypeRef::Json => true,
            TypeRef::Vec(inner) | TypeRef::Optional(inner) => self.needs_json_marshalling(inner),
            TypeRef::Map(k, v) => self.needs_json_marshalling(k) || self.needs_json_marshalling(v),
            _ => false,
        }
    }

    /// Emit code that converts the Ruby `val` (in scope) into the Rust return type
    /// and either returns it (if has_error: false) or wraps it in `Ok(...)` (if has_error: true).
    /// For sync bodies — no leading whitespace.
    fn return_conversion(&self, method: &MethodDef, spec: &TraitBridgeSpec, has_error: bool, indent: &str) -> String {
        let rust_ty = self.return_rust_type(&method.return_type);
        let err_non_json = if has_error {
            self.make_error(&format!(
                "format!(\"Ruby method '{}' returned non-JSON value: {{}}\", e)",
                method.name
            ))
        } else {
            String::new()
        };
        let err_deserialize = if has_error {
            self.make_error(&format!(
                "format!(\"Failed to deserialize Ruby '{}' return value: {{}}\", e)",
                method.name
            ))
        } else {
            String::new()
        };
        let err_convert = if has_error {
            self.make_error(&format!(
                "format!(\"Failed to convert Ruby '{}' return value: {{}}\", e)",
                method.name
            ))
        } else {
            String::new()
        };

        crate::backends::magnus::template_env::render(
            "trait_bridge_return_conversion.rs.jinja",
            minijinja::context! {
                wrapper => spec.wrapper_name(),
                method_name => &method.name,
                has_error => has_error,
                needs_json => self.needs_json_marshalling(&method.return_type),
                native_return_binding => self.native_struct_return(&method.return_type),
                indent => indent,
                rust_ty => rust_ty,
                err_non_json => err_non_json,
                err_deserialize => err_deserialize,
                err_convert => err_convert,
            },
        )
    }

    /// Binding struct name to route a native-object return through, when the return is a bare
    /// `Named` struct on the native-marshalled return allowlist. The binding struct's `TryConvert`
    /// accepts the host's native wrapped object (and a Hash/JSON via `to_json`); `From<Binding> for
    /// core` then yields the core value. `None` keeps the `serde_json::from_str`-into-core path.
    fn native_struct_return<'a>(&self, ty: &'a TypeRef) -> Option<&'a str> {
        match ty {
            TypeRef::Named(n) if self.struct_return_types.contains(n) => Some(n.as_str()),
            _ => None,
        }
    }

    /// True when a `Named(name)` param should be handed to the host as the binding's native Ruby
    /// value rather than a JSON string — i.e. it is a known serde struct per the shared allowlist.
    /// The native value is the `#[magnus::wrap]` binding struct, constructed from the core value
    /// via the same `From<core::T>` conversion the binding uses for function return values.
    fn is_native_struct_param(&self, name: &str) -> bool {
        self.struct_param_types.contains(name)
    }

    /// Build a Ruby arg expression for funcall given a type and variable name.
    /// Wraps `var` in deref/borrow as needed so the expression always type-checks
    /// regardless of whether `var` is owned (`String`, `Vec<u8>`, ...) or borrowed.
    fn ruby_arg_expr_custom(&self, ty: &TypeRef, var: &str) -> String {
        match ty {
            TypeRef::String => format!("ruby.str_new(AsRef::<str>::as_ref(&{var})).as_value()"),
            TypeRef::Bytes => {
                format!("ruby.str_new(String::from_utf8_lossy(AsRef::<[u8]>::as_ref(&{var})).as_ref()).as_value()")
            }
            // fields (`{Binding}::from(core_value)`). The `#[magnus::wrap]` struct implements
            TypeRef::Named(n) if self.is_native_struct_param(n) => {
                format!("{{ use magnus::IntoValue; {n}::from({var}.clone()).into_value_with(&ruby) }}")
            }
            TypeRef::Named(_) | TypeRef::Json => format!(
                "serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value())"
            ),
            TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_) => format!(
                "serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value())"
            ),
            TypeRef::Path => format!(
                "ruby.str_new(<_ as AsRef<std::path::Path>>::as_ref(&{var}).to_string_lossy().as_ref()).as_value()"
            ),
            _ => var.to_string(),
        }
    }
}

#[cfg(test)]
mod forwarding_tests {
    use super::*;

    fn make_generator() -> MagnusBridgeGenerator {
        MagnusBridgeGenerator {
            core_import: "sample_core".to_string(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_string(),
            error_constructor: "SampleError::Message { message: {msg} }".to_string(),
            struct_param_types: std::collections::HashSet::new(),
            struct_return_types: std::collections::HashSet::new(),
            forwardable_defaulted: std::collections::HashSet::new(),
            plugin_version_is_fallible: false,
        }
    }

    fn make_trait() -> crate::core::ir::TypeDef {
        crate::core::ir::TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "sample_core::OcrBackend".to_string(),
            is_trait: true,
            is_opaque: true,
            methods: vec![
                crate::core::ir::MethodDef {
                    name: "supports_language".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::Ref),
                    cfg: None,
                    return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    ..Default::default()
                },
                crate::core::ir::MethodDef {
                    name: "supports_table_detection".to_string(),
                    has_default_impl: true,
                    receiver: Some(crate::core::ir::ReceiverKind::Ref),
                    cfg: None,
                    return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn presence_uses_construction_time_flag_and_wrapper_gains_field() {
        let mut generator = make_generator();
        generator
            .forwardable_defaulted
            .insert("supports_table_detection".to_string());
        let trait_def = make_trait();
        let bridge = crate::core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            ..crate::core::config::TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge,
            core_import: "sample_core",
            wrapper_prefix: "Rb",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
            error_type: "SampleError".to_string(),
            error_constructor: "SampleError::Message { message: {msg} }".to_string(),
        };

        let check = generator
            .gen_method_presence_check(&trait_def.methods[1], &spec)
            .unwrap();
        assert_eq!(check, "self.has_supports_table_detection");

        let fields = generator.extra_bridge_fields(&spec);
        assert_eq!(
            fields,
            vec![
                ("has_supports_table_detection".to_string(), "bool".to_string()),
                (
                    "runtime_dispatcher".to_string(),
                    "RbOcrBackendBridgeRuntimeDispatcher".to_string()
                )
            ]
        );

        let ctor = generator.gen_constructor(&spec);
        assert!(
            ctor.contains("has_supports_table_detection: rb_obj.respond_to(\"supports_table_detection\", false)"),
            "constructor must capture the flag under the GVL:\n{ctor}"
        );

        let output = crate::codegen::generators::trait_bridge::gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("has_supports_table_detection: bool,"),
            "wrapper struct must declare the flag field:\n{}",
            output.code
        );
        assert!(
            output
                .code
                .contains("RbOcrBackendBridgeDefaultSupportsTableDetection(self).supports_table_detection()"),
            "fallback must run the Rust default via the delegate:\n{}",
            output.code
        );
    }
}
