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
    // Skip if explicitly excluded for Ruby
    if bridge_cfg.exclude_languages.contains(&"ruby".to_string()) {
        return Ok(String::new());
    }

    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (for example, `&HiddenDoc`)
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

    if is_visitor_bridge {
        // Visitor pattern: use the old visitor bridge code
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
        // Plugin pattern: use the shared TraitBridgeGenerator infrastructure.
        // Use the host crate's canonical error type (e.g. SampleCrateError) so the
        // generated `impl Plugin for ...` matches the trait's actual signature.
        // Classify which callback params get native-object marshalling using the SHARED rule
        // (`native_marshalled_struct_params`) so the allowlist is identical to what other
        // backends consult. For such params the bridge hands the host the binding's native Ruby
        // value (the `#[magnus::wrap]` struct, built via the same `From<core::T>` conversion used
        // for return values / fields) instead of a JSON string.
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        // Return-side counterpart: a host may return the binding's native Ruby object. The binding
        // struct's `TryConvert` accepts the native wrapped object (and a Hash/JSON via `to_json`),
        // and `From<Binding> for core` converts it. Gate on the binding→core conversion actually
        // being generated for the type (`convertible_types`) — unlike pyo3's always-emitted
        // `From<Binding>`, magnus generates it conditionally; for a type that does not qualify, keep
        // the proven `serde_json::from_str`-into-core path so the bridge always compiles.
        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let struct_return_types: std::collections::HashSet<String> =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_returns(trait_type, api)
                .into_iter()
                .filter(|name| binding_to_core.contains(name.as_str()))
                .collect();
        let generator = MagnusBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
            struct_param_types,
            struct_return_types,
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
        // Emit trait-bridge specific imports as `use ... as _;` at the top of the
        // bridge block so multiple bridges can share trait imports without name
        // collisions on the same module-level identifier.
        let mut prefixed = String::with_capacity(output.imports.len() * 64 + output.code.len());
        let imports_to_emit: Vec<_> = output
            .imports
            .iter()
            .filter(|imp| *imp != "magnus::prelude::*")
            .collect();
        // Emit allow attribute before each import group to suppress unused_imports warnings
        for imp in &imports_to_emit {
            prefixed.push_str("#[allow(unused_imports)]\n");
            prefixed.push_str("use ");
            prefixed.push_str(imp);
            prefixed.push_str(" as _;\n");
        }
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
}

impl MagnusBridgeGenerator {
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

    fn bridge_imports(&self) -> Vec<String> {
        // Keep this list small. `Arc` is already imported globally at file scope by
        // the magnus gen_bindings pipeline. Trait-only imports are emitted as `use ... as _`
        // by `gen_trait_bridge` so multiple bridges can co-exist without name collisions.
        vec![
            "magnus::value::InnerValue".to_string(),
            "magnus::TryConvert".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);

        // Build funcall args
        let args: Vec<String> = method.params.iter().map(|p| self.ruby_arg_expr(p)).collect();

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

        let mut body = crate::backends::magnus::template_env::render(
            "sync_method_body.rs.jinja",
            minijinja::context! {
                call => call,
                has_error => has_error,
                is_unit => is_unit,
                err_expr => err_expr,
            },
        );

        if !is_unit {
            body.push_str(&self.return_conversion(method, has_error, ""));
        }

        body
    }

    fn gen_async_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);

        // async_trait wraps the body in `Pin<Box<dyn Future + Send>>`, so anything
        // captured into the future must be Send. magnus::Value is !Send, so we
        // capture only the Send wrappers (Opaque<Value>, owned param copies),
        // then dereference inside spawn_blocking which holds GVL on the worker thread.

        // Clone params into Send-safe owned copies for the blocking task.
        let conversions: Vec<String> = method
            .params
            .iter()
            .map(|p| match (&p.ty, p.is_ref) {
                (TypeRef::String, true) => format!("let {}_owned = {}.to_string();\n", p.name, p.name),
                (TypeRef::Bytes, true) => format!("let {}_owned = {}.to_vec();\n", p.name, p.name),
                (TypeRef::Path, true) => format!("let {}_owned = {}.to_path_buf();\n", p.name, p.name),
                _ => format!("let {}_owned = {}.clone();\n", p.name, p.name),
            })
            .collect();
        let conversion_bindings = conversions.join("");

        let return_type_rust = if is_unit {
            "()".to_string()
        } else {
            self.return_rust_type(&method.return_type)
        };
        let err_path = self.error_path();
        let result_ty = if has_error {
            format!("std::result::Result<{return_type_rust}, {err_path}>")
        } else {
            return_type_rust.clone()
        };

        let conversions = format!(
            "let inner = self.inner;\n\
let cached_name = self.cached_name.clone();\n\
// cached_name is referenced both inside the spawn_blocking closure and after\n\
// the await for the JoinError fallback, so clone once for each consumer.\n\
let cached_name_for_blocking = cached_name.clone();\n\
{conversion_bindings}\n",
            conversion_bindings = conversion_bindings,
        );

        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_name = if matches!(&p.ty, TypeRef::String) && p.is_ref {
                    format!("{}_owned.as_str()", p.name)
                } else {
                    format!("{}_owned", p.name)
                };
                self.ruby_arg_expr_custom(&p.ty, &param_name)
            })
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

        let err_expr_call = self.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name_for_blocking, e)"
        ));
        let err_expr_join = if has_error {
            self.make_error("format!(\"spawn_blocking failed for '{}': {}\", cached_name, e)")
        } else {
            String::new()
        };

        crate::backends::magnus::template_env::render(
            "trait_bridge_async_method_body.rs.jinja",
            minijinja::context! {
                conversions => conversions,
                call => call,
                has_error => has_error,
                is_unit => is_unit,
                result_ty => result_ty,
                err_expr_call => err_expr_call,
                err_expr_join => err_expr_join,
                return_conversion => self.return_conversion(method, has_error, "            "),
            },
        )
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods: Vec<_> = spec.required_methods().iter().map(|m| m.name.as_str()).collect();

        crate::backends::magnus::template_env::render(
            "trait_bridge_constructor.rs.jinja",
            minijinja::context! {
                wrapper => wrapper,
                required_methods => required_methods,
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
    fn return_conversion(&self, method: &MethodDef, has_error: bool, indent: &str) -> String {
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

    /// Build a Ruby arg expression for funcall given a Rust parameter.
    fn ruby_arg_expr(&self, p: &crate::core::ir::ParamDef) -> String {
        self.ruby_arg_expr_custom(&p.ty, &p.name)
    }

    /// Build a Ruby arg expression for funcall given a type and variable name.
    /// Wraps `var` in deref/borrow as needed so the expression always type-checks
    /// regardless of whether `var` is owned (`String`, `Vec<u8>`, ...) or borrowed.
    fn ruby_arg_expr_custom(&self, ty: &TypeRef, var: &str) -> String {
        match ty {
            // str_new takes Into<&str>; AsRef<str> covers both String and &str.
            TypeRef::String => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(AsRef::<str>::as_ref(&{var})).as_value() }}"
            ),
            // String::from_utf8_lossy needs &[u8]; AsRef<[u8]> covers both Vec<u8> and &[u8].
            TypeRef::Bytes => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(String::from_utf8_lossy(AsRef::<[u8]>::as_ref(&{var})).as_ref()).as_value() }}"
            ),
            // Known serde struct: hand the host the binding's native Ruby value, built from the
            // core value through the same Rust→Ruby conversion used for return values / struct
            // fields (`{Binding}::from(core_value)`). The `#[magnus::wrap]` struct implements
            // `IntoValue`, so `into_value_with` produces the same `magnus::Value` the funcall tuple
            // expects — no JSON round-trip. `{var}.clone()` yields an owned `core::T` whether `var`
            // is a `&core::T` (sync path) or an owned `core::T` (async `_owned` path).
            TypeRef::Named(n) if self.is_native_struct_param(n) => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; use magnus::IntoValue; {n}::from({var}.clone()).into_value_with(&ruby) }}"
            ),
            // serde_json::to_string takes &T; the macro `&{var}` is fine for both owned and ref.
            // Other Named params (enums, opaque/handle, excluded/unknown) keep the prior
            // JSON-string representation.
            TypeRef::Named(_) | TypeRef::Json => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value()) }}"
            ),
            TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_) => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; serde_json::to_string(&{var}).ok().map(|s| ruby.str_new(s.as_str()).as_value()).unwrap_or_else(|| ruby.qnil().as_value()) }}"
            ),
            // Both PathBuf (owned) and &Path (borrowed) coerce via AsRef<Path>; pin
            // the AsRef target type explicitly so type inference doesn't fail.
            TypeRef::Path => format!(
                "{{ let ruby = unsafe {{ magnus::Ruby::get_unchecked() }}; ruby.str_new(<_ as AsRef<std::path::Path>>::as_ref(&{var}).to_string_lossy().as_ref()).as_value() }}"
            ),
            _ => var.to_string(),
        }
    }
}
