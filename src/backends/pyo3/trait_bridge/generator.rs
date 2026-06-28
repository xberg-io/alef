use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec, host_function_path};
use crate::core::ir::{MethodDef, TypeRef};
use std::collections::HashMap;

fn exported_pyfunction_symbol(fn_name: &str) -> String {
    fn_name.to_string()
}

/// PyO3-specific trait bridge generator.
/// Implements code generation for bridging Python objects to Rust traits.
pub struct Pyo3BridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
    pub error_type: String,
    /// Callback-param type names that get NATIVE-object marshalling — known serde structs per
    /// the shared [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] rule.
    /// For such a param the bridge constructs the binding's native Python object (the `#[pyclass]`
    /// wrapper, via the same `From<core::T>` conversion used for return values) and hands THAT to
    /// the host method, instead of serializing the param to a JSON string. Enums, opaque/handle
    /// types, and excluded/unknown `Named` params are absent and keep their prior representation.
    pub struct_param_types: std::collections::HashSet<String>,
    /// Callback-RETURN type names that get NATIVE-object marshalling — known serde structs returned
    /// directly by a method (per the shared `native_marshalled_struct_returns` rule). For such a
    /// return the bridge first tries to extract the host's native Python object and convert it via
    /// `From<Binding>` for the core type, falling back to the JSON/mapping path otherwise.
    pub struct_return_types: std::collections::HashSet<String>,
}

impl TraitBridgeGenerator for Pyo3BridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "Py<PyAny>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["pyo3::prelude::*".to_string(), "std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();

        let py_args = self.sync_py_args(method);
        // Invoke the host method through the caller's contextvars Context so any ContextVar
        // set by the caller is visible inside the callback. `ctx.run(bound_method, *args)`
        // runs the callable with `ctx` as the active context; calling the method directly
        // would run it under the worker/empty context instead. The trailing comma keeps the
        // zero-arg case a 1-tuple `(bound_method,)` rather than a parenthesized expression.
        let run_args = if py_args.is_empty() {
            "bound_method,".to_string()
        } else {
            format!("bound_method, {py_args}")
        };
        let call = if py_args.is_empty() {
            format!("self.inner.bind(py).call_method0(\"{name}\")")
        } else {
            format!("self.inner.bind(py).call_method1(\"{name}\", ({py_args}))")
        };
        let error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", self.cached_name, e)"
        ));

        if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::pyo3::template_env::render(
                "trait_bridge/sync_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    run_args => run_args,
                    has_error => has_error,
                    error_expr => error_expr,
                },
            )
        } else {
            let ext = self.extract_ty(&method.return_type);
            let is_named = matches!(method.return_type, TypeRef::Named(_));
            // Name the expected return type and hint the shape so a host returning a mismatched
            // value can fix it. This is a PyErr (the sync chain is `PyResult`-typed before the
            // final `map_err`); the serde error (`{}`) already names the offending field/path.
            let return_type_name = self.return_type_display_name(&method.return_type);
            let deserialize_error_expr = format!(
                "pyo3::exceptions::PyRuntimeError::new_err(format!(\"method '{name}' returned a value that does not match the expected return type `{return_type_name}`: {{}}. The returned value must be a mapping matching the fields of `{return_type_name}`.\", e))"
            );
            crate::backends::pyo3::template_env::render(
                "trait_bridge/sync_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    run_args => run_args,
                    is_named => is_named,
                    extract_ty => ext,
                    native_return_binding => self.native_struct_return(&method.return_type),
                    has_error => has_error,
                    error_expr => error_expr,
                    deserialize_error_expr => deserialize_error_expr,
                },
            )
        }
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        // Build param cloning code using template
        let params: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                minijinja::context! {
                    name => &p.name,
                    ty => match &p.ty {
                        TypeRef::Bytes => "Bytes",
                        TypeRef::Path => "Path",
                        TypeRef::Named(_) => {

                            match &p.ty {
                                TypeRef::Named(n) => n.as_str(),
                                _ => "",
                            }
                        },
                        _ => "",
                    }.to_string(),
                    ty_is_named => matches!(&p.ty, TypeRef::Named(_)),
                    // Native serde struct: the preamble clones the core value into `{name}_owned`
                    // so the call site can build the binding's native Python object from it,
                    // rather than serializing the param to a JSON string.
                    is_native_struct => matches!(&p.ty, TypeRef::Named(n) if self.is_native_struct_param(n)),
                    is_ref => p.is_ref,
                }
            })
            .collect();

        let param_cloning = crate::backends::pyo3::template_env::render(
            "trait_bridge/async_param_cloning.jinja",
            minijinja::context! {
                params => params,
            },
        );

        let py_args = self.async_py_args(method);
        // Run the host method through the caller's contextvars Context (captured on the calling
        // thread before `spawn_blocking`) so the callback sees the caller's ContextVars rather
        // than the worker thread's fresh, empty context. The trailing comma keeps the zero-arg
        // case a 1-tuple `(bound_method,)` rather than a parenthesized expression.
        let run_args = if py_args.is_empty() {
            "bound_method,".to_string()
        } else {
            format!("bound_method, {py_args}")
        };
        let call = if py_args.is_empty() {
            format!("obj.call_method0(\"{name}\")")
        } else {
            format!("obj.call_method1(\"{name}\", ({py_args}))")
        };
        let error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e)"
        ));
        let json_error_expr =
            spec.make_error("format!(\"Plugin '{}': JSON serialization failed: {}\", cached_name, e)");
        // Name the expected return type and hint the shape so a host returning a mismatched
        // value can fix it. The serde error (`{}`) already names the offending field/path
        // (e.g. "missing field `title`" / "invalid type ... at line L column C").
        let return_type_name = self.return_type_display_name(&method.return_type);
        let deserialize_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' returned a value that does not match the expected return type `{return_type_name}`: {{}}. The returned value must be a mapping matching the fields of `{return_type_name}`.\", cached_name, e)"
        ));
        let spawn_error_expr = spec.make_error("format!(\"spawn_blocking failed: {}\", e)");

        if self.is_named(&method.return_type) {
            let return_type =
                crate::codegen::generators::trait_bridge::format_type_ref(&method.return_type, &spec.type_paths);
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_named_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    run_args => run_args,
                    param_cloning => param_cloning,
                    return_type => return_type,
                    native_return_binding => self.native_struct_return(&method.return_type),
                    error_expr => error_expr,
                    json_error_expr => json_error_expr,
                    deserialize_error_expr => deserialize_error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        } else if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    run_args => run_args,
                    param_cloning => param_cloning,
                    error_expr => error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        } else {
            let ext = self.extract_ty(&method.return_type);
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    run_args => run_args,
                    extract_ty => ext,
                    param_cloning => param_cloning,
                    error_expr => error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        }
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods = spec.required_methods();
        crate::backends::pyo3::template_env::render(
            "trait_bridge/constructor.jinja",
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
        // Derive the FQN of the host crate's `unregister_*` function from the
        // bridge's `registry_getter` path: `sample_core::plugins::registry::get_*`
        // → `sample_core::plugins::*::unregister_*`. When `registry_getter` is not
        // set we fall back to `{core}::plugins::{unregister_fn}` and trust the
        // caller's wiring.
        let host_path = host_function_path(spec, unregister_fn);
        let host_symbol = exported_pyfunction_symbol(unregister_fn);
        crate::backends::pyo3::template_env::render(
            "trait_bridge/unregistration_fn.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
                host_symbol => host_symbol,
                host_path => host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, clear_fn);
        let host_symbol = exported_pyfunction_symbol(clear_fn);
        crate::backends::pyo3::template_env::render(
            "trait_bridge/clear_fn.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
                host_symbol => host_symbol,
                host_path => host_path,
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
        let required_methods_str = req_methods
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect::<Vec<_>>()
            .join(", ");

        let register_extra_args = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        crate::backends::pyo3::template_env::render(
            "trait_bridge/registration_fn.jinja",
            minijinja::context! {
                register_fn => register_fn,
                wrapper => wrapper,
                trait_path => trait_path,
                registry_getter => registry_getter,
                register_extra_args => register_extra_args,
                has_required_methods => !req_methods.is_empty(),
                required_methods_str => required_methods_str,
            },
        )
    }
}

impl Pyo3BridgeGenerator {
    /// Human-facing name of a return type for callback deserialization error messages.
    /// For a `Named` type this is the bare type name (e.g. `Doc`), not the fully-qualified
    /// Rust path, so the message reads the way a host implementer thinks about their return
    /// value. Other shapes fall back to their Rust rendering.
    fn return_type_display_name(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Named(name) => name.clone(),
            other => self.extract_ty(other),
        }
    }

    /// Extract the Python type that corresponds to a Rust TypeRef.
    fn extract_ty(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => self.prim(p).to_string(),
            TypeRef::String | TypeRef::Path | TypeRef::Char => "String".into(),
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
    fn prim(&self, p: &crate::core::ir::PrimitiveType) -> &'static str {
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
    }

    /// True when a `Named(name)` param should be handed to the host as the binding's native
    /// Python object rather than a JSON string — i.e. it is a known serde struct per the shared
    /// allowlist. The native object is the `#[pyclass]` wrapper, constructed from the core value
    /// via the same `From<core::T>` conversion the binding uses for function return values.
    fn is_native_struct_param(&self, name: &str) -> bool {
        self.struct_param_types.contains(name)
    }

    /// Build Python call argument expressions for a sync method.
    fn sync_py_args(&self, method: &MethodDef) -> String {
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, {})", p.name),
                (TypeRef::Path, true) => format!("{}.to_str().unwrap_or_default()", p.name),
                // Known serde struct: hand the host the binding's native Python object, built from
                // the core value through the same Rust→Python conversion used for return values
                // (`{Binding}::from(core_value)`). PyO3 auto-converts the `#[pyclass]` to a Python
                // object at the call boundary. No JSON round-trip.
                (TypeRef::Named(n), true) if self.is_native_struct_param(n) => {
                    format!("{}::from((*{}).clone())", n, p.name)
                }
                // Owned native serde struct (`is_ref == false`): build the native Python object
                // from the owned core value directly (no deref). Mirrors the borrowed arm above.
                (TypeRef::Named(n), false) if self.is_native_struct_param(n) => {
                    format!("{}::from({}.clone())", n, p.name)
                }
                // Other Named params (enums, opaque/handle, excluded/unknown) keep the prior
                // JSON-string representation.
                (TypeRef::Named(_), true) => {
                    format!("serde_json::to_string({}).unwrap_or_default()", p.name)
                }
                _ => p.name.clone(),
            })
            .collect();
        if args.len() == 1 {
            format!("{},", args[0])
        } else {
            args.join(", ")
        }
    }

    /// Build Python call argument expressions for an async method.
    fn async_py_args(&self, method: &MethodDef) -> String {
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, &{})", p.name),
                (TypeRef::Path, true) => format!("{}_str.as_str()", p.name),
                // Known serde struct (borrowed or owned): the param-cloning preamble owns the
                // cloned core value in `{name}_owned`; build the native Python object from it here.
                // Owned params (`is_ref == false`, e.g. the by-value `ExtractInput` envelope) need
                // the same marshalling — passing the raw `xberg::T` has no `IntoPyObject` (E0277).
                (TypeRef::Named(n), _) if self.is_native_struct_param(n) => {
                    format!("{}::from({}_owned.clone())", n, p.name)
                }
                (TypeRef::Named(_), true) => format!("{}_json.as_str()", p.name),
                _ => p.name.clone(),
            })
            .collect();
        if args.len() == 1 {
            format!("{},", args[0])
        } else {
            args.join(", ")
        }
    }

    /// Check if a TypeRef is a Named type.
    fn is_named(&self, ty: &TypeRef) -> bool {
        matches!(ty, TypeRef::Named(_))
    }

    /// Binding pyclass type name to extract for a native-object return, when the return is a bare
    /// `Named` struct on the native-marshalled return allowlist. The bridge tries
    /// `py_result.extract::<Binding>()` and converts via `From<Binding>` for the core type, falling
    /// back to the JSON/mapping path. `None` keeps the mapping path unchanged.
    fn native_struct_return<'a>(&self, ty: &'a TypeRef) -> Option<&'a str> {
        match ty {
            TypeRef::Named(n) if self.struct_return_types.contains(n) => Some(n.as_str()),
            _ => None,
        }
    }
}
