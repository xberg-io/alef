//! Shared trait bridge code generation.
//!
//! Generates wrapper structs that allow foreign language objects (Python, JS, etc.)
//! to implement Rust traits via FFI. Each backend implements [`TraitBridgeGenerator`]
//! to provide language-specific dispatch logic; the shared functions in this module
//! handle the structural boilerplate.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, ParamDef, TypeDef};
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::fmt::Write;

/// Everything needed to generate a trait bridge for one trait.
pub struct TraitBridgeSpec<'a> {
    /// The trait definition from the IR.
    pub trait_def: &'a TypeDef,
    /// Bridge configuration from `alef.toml`.
    pub bridge_config: &'a TraitBridgeConfig,
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: &'a str,
    /// Language-specific prefix for the wrapper type (e.g., `"Python"`, `"Js"`, `"Wasm"`).
    pub wrapper_prefix: &'a str,
    /// Map of type name → fully-qualified Rust path for qualifying `Named` types.
    pub type_paths: HashMap<String, String>,
    /// The crate's error type name (e.g., `"KreuzbergError"`). Defaults to `"Error"`.
    pub error_type: String,
    /// Error constructor pattern. `{msg}` is replaced with the message expression.
    pub error_constructor: String,
}

impl<'a> TraitBridgeSpec<'a> {
    /// Fully qualified error type path (e.g., `"kreuzberg::KreuzbergError"`).
    pub fn error_path(&self) -> String {
        format!("{}::{}", self.core_import, self.error_type)
    }

    /// Generate an error construction expression from a message expression.
    pub fn make_error(&self, msg_expr: &str) -> String {
        self.error_constructor.replace("{msg}", msg_expr)
    }

    /// Wrapper struct name: `{prefix}{TraitName}Bridge` (e.g., `PythonOcrBackendBridge`).
    pub fn wrapper_name(&self) -> String {
        format!("{}{}Bridge", self.wrapper_prefix, self.trait_def.name)
    }

    /// Snake-case version of the trait name (e.g., `"ocr_backend"`).
    pub fn trait_snake(&self) -> String {
        self.trait_def.name.to_snake_case()
    }

    /// Full Rust path to the trait (e.g., `kreuzberg::OcrBackend`).
    pub fn trait_path(&self) -> String {
        self.trait_def.rust_path.replace('-', "_")
    }

    /// Methods that are required (no default impl) — must be provided by the foreign object.
    pub fn required_methods(&self) -> Vec<&'a MethodDef> {
        self.trait_def.methods.iter().filter(|m| !m.has_default_impl).collect()
    }

    /// Methods that have a default impl — optional on the foreign object.
    pub fn optional_methods(&self) -> Vec<&'a MethodDef> {
        self.trait_def.methods.iter().filter(|m| m.has_default_impl).collect()
    }
}

/// Backend-specific trait bridge generation.
///
/// Each binding backend (PyO3, NAPI-RS, wasm-bindgen, etc.) implements this trait
/// to provide the language-specific parts of bridge codegen. The shared functions
/// in this module call these methods to fill in the backend-dependent pieces.
pub trait TraitBridgeGenerator {
    /// The type of the wrapped foreign object (e.g., `"Py<PyAny>"`, `"ThreadsafeFunction"`).
    fn foreign_object_type(&self) -> &str;

    /// Additional `use` imports needed for the bridge code.
    fn bridge_imports(&self) -> Vec<String>;

    /// Generate the body of a synchronous method bridge.
    ///
    /// The returned string is inserted inside the trait impl method. It should
    /// call through to the foreign object and convert the result.
    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String;

    /// Generate the body of an async method bridge.
    ///
    /// The returned string is the body of a `Box::pin(async move { ... })` block.
    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String;

    /// Generate the constructor body that validates and wraps the foreign object.
    ///
    /// Should check that the foreign object provides all required methods and
    /// return `Self { ... }` on success.
    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String;

    /// Generate the complete registration function including attributes, signature, and body.
    ///
    /// Each backend needs different function signatures (PyO3 takes `py: Python`,
    /// NAPI takes `#[napi]` with JS params, FFI takes `extern "C"` with raw pointers),
    /// so the generator owns the full function.
    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String;
}

// ---------------------------------------------------------------------------
// Shared generation functions
// ---------------------------------------------------------------------------

/// Generate the wrapper struct holding the foreign object and cached fields.
///
/// Produces a struct like:
/// ```ignore
/// pub struct PythonOcrBackendBridge {
///     inner: Py<PyAny>,
///     cached_name: String,
/// }
/// ```
pub fn gen_bridge_wrapper_struct(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let wrapper = spec.wrapper_name();
    let foreign_type = generator.foreign_object_type();
    let mut out = String::with_capacity(512);

    writeln!(
        out,
        "/// Wrapper that bridges a foreign {prefix} object to the `{trait_name}` trait.",
        prefix = spec.wrapper_prefix,
        trait_name = spec.trait_def.name,
    )
    .ok();
    writeln!(out, "pub struct {wrapper} {{").ok();
    writeln!(out, "    inner: {foreign_type},").ok();
    writeln!(out, "    cached_name: String,").ok();
    write!(out, "}}").ok();
    out
}

/// Generate `impl SuperTrait for Wrapper` when the bridge config specifies a super-trait.
///
/// Forwards `name()`, `version()`, `initialize()`, and `shutdown()` to the
/// foreign object, using `cached_name` for `name()`.
///
/// The super-trait path is derived from the config's `super_trait` field. If it
/// contains `::`, it's used as-is; otherwise it's qualified as `{core_import}::{super_trait}`.
pub fn gen_bridge_plugin_impl(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    let super_trait_name = spec.bridge_config.super_trait.as_deref()?;

    let wrapper = spec.wrapper_name();
    let core_import = spec.core_import;

    // Derive the fully-qualified super-trait path
    let super_trait_path = if super_trait_name.contains("::") {
        super_trait_name.to_string()
    } else {
        format!("{core_import}::{super_trait_name}")
    };

    // Build synthetic MethodDefs for the Plugin methods and delegate to the generator
    // for the actual call bodies. The Plugin trait interface is well-known: name(),
    // version(), initialize(), shutdown().
    let mut out = String::with_capacity(1024);
    writeln!(out, "impl {super_trait_path} for {wrapper} {{").ok();

    // name() -> &str — uses cached field
    writeln!(out, "    fn name(&self) -> &str {{").ok();
    writeln!(out, "        &self.cached_name").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    let error_path = spec.error_path();

    // version() -> String — delegate to foreign object
    writeln!(out, "    fn version(&self) -> String {{").ok();
    let version_method = MethodDef {
        name: "version".to_string(),
        params: vec![],
        return_type: alef_core::ir::TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(alef_core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };
    let version_body = generator.gen_sync_method_body(&version_method, spec);
    for line in version_body.lines() {
        writeln!(out, "        {}", line.trim_start()).ok();
    }
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // initialize() -> Result<(), ErrorType>
    writeln!(
        out,
        "    fn initialize(&self) -> std::result::Result<(), {error_path}> {{"
    )
    .ok();
    let init_method = MethodDef {
        name: "initialize".to_string(),
        params: vec![],
        return_type: alef_core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some(error_path.clone()),
        doc: String::new(),
        receiver: Some(alef_core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
    };
    let init_body = generator.gen_sync_method_body(&init_method, spec);
    for line in init_body.lines() {
        writeln!(out, "        {}", line.trim_start()).ok();
    }
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // shutdown() -> Result<(), ErrorType>
    writeln!(
        out,
        "    fn shutdown(&self) -> std::result::Result<(), {error_path}> {{"
    )
    .ok();
    let shutdown_method = MethodDef {
        name: "shutdown".to_string(),
        params: vec![],
        return_type: alef_core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some(error_path.clone()),
        doc: String::new(),
        receiver: Some(alef_core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
    };
    let shutdown_body = generator.gen_sync_method_body(&shutdown_method, spec);
    for line in shutdown_body.lines() {
        writeln!(out, "        {}", line.trim_start()).ok();
    }
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    Some(out)
}

/// Generate `impl Trait for Wrapper` dispatching each method through the generator.
///
/// Every method on the trait (including those with `has_default_impl`) gets a
/// generated body that forwards to the foreign object.
pub fn gen_bridge_trait_impl(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let wrapper = spec.wrapper_name();
    let trait_path = spec.trait_path();
    let mut out = String::with_capacity(2048);

    // Add #[async_trait] when the trait has async methods (needed for async_trait macro compatibility)
    let has_async_methods = spec
        .trait_def
        .methods
        .iter()
        .any(|m| m.is_async && m.trait_source.is_none());
    if has_async_methods {
        writeln!(out, "#[async_trait::async_trait]").ok();
    }
    writeln!(out, "impl {trait_path} for {wrapper} {{").ok();

    // Filter out methods inherited from super-traits (they're handled by gen_bridge_plugin_impl)
    let own_methods: Vec<_> = spec
        .trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none())
        .collect();

    for (i, method) in own_methods.iter().enumerate() {
        if i > 0 {
            writeln!(out).ok();
        }

        // Build the method signature
        let async_kw = if method.is_async { "async " } else { "" };
        let receiver = match &method.receiver {
            Some(alef_core::ir::ReceiverKind::Ref) => "&self",
            Some(alef_core::ir::ReceiverKind::RefMut) => "&mut self",
            Some(alef_core::ir::ReceiverKind::Owned) => "self",
            None => "",
        };

        // Build params (excluding self), using format_param_type to respect is_ref/is_mut
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, format_param_type(p, &spec.type_paths)))
            .collect();

        let all_params = if receiver.is_empty() {
            params.join(", ")
        } else if params.is_empty() {
            receiver.to_string()
        } else {
            format!("{}, {}", receiver, params.join(", "))
        };

        // Return type — override the IR's error type with the configured crate error type
        // so the impl matches the actual trait definition (the IR may extract a different
        // error type like anyhow::Error from re-exports or type alias resolution).
        let error_override = method.error_type.as_ref().map(|_| spec.error_path());
        let ret = format_return_type(&method.return_type, error_override.as_deref(), &spec.type_paths);

        writeln!(out, "    {async_kw}fn {}({all_params}) -> {ret} {{", method.name).ok();

        // Generate body: async methods use Box::pin, sync methods call directly
        let body = if method.is_async {
            generator.gen_async_method_body(method, spec)
        } else {
            generator.gen_sync_method_body(method, spec)
        };

        for line in body.lines() {
            writeln!(out, "        {line}").ok();
        }
        writeln!(out, "    }}").ok();
    }

    write!(out, "}}").ok();
    out
}

/// Generate the `register_xxx()` function that wraps a foreign object and
/// inserts it into the plugin registry.
///
/// Returns `None` when `bridge_config.register_fn` is absent (per-call bridge pattern).
/// The generator owns the full function (attributes, signature, body) because each
/// backend needs different signatures.
pub fn gen_bridge_registration_fn(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    spec.bridge_config.register_fn.as_deref()?;
    Some(generator.gen_registration_fn(spec))
}

/// Result of trait bridge generation: imports (to be added via `builder.add_import`)
/// and the code body (to be added via `builder.add_item`).
pub struct BridgeOutput {
    /// Import paths (e.g., `"std::sync::Arc"`) — callers should add via `builder.add_import()`.
    pub imports: Vec<String>,
    /// The generated code (struct, impls, registration fn).
    pub code: String,
}

/// Generate the complete trait bridge code block: struct, impls, and
/// optionally a registration function.
///
/// Returns [`BridgeOutput`] with imports separated from code so callers can
/// route imports through `builder.add_import()` (which deduplicates).
pub fn gen_bridge_all(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> BridgeOutput {
    let imports = generator.bridge_imports();
    let mut out = String::with_capacity(4096);

    // Wrapper struct
    out.push_str(&gen_bridge_wrapper_struct(spec, generator));
    writeln!(out).ok();
    writeln!(out).ok();

    // Constructor (impl block with new())
    out.push_str(&generator.gen_constructor(spec));
    writeln!(out).ok();
    writeln!(out).ok();

    // Plugin super-trait impl (if applicable)
    if let Some(plugin_impl) = gen_bridge_plugin_impl(spec, generator) {
        out.push_str(&plugin_impl);
        writeln!(out).ok();
        writeln!(out).ok();
    }

    // Trait impl
    out.push_str(&gen_bridge_trait_impl(spec, generator));

    // Registration function — only when register_fn is configured
    if let Some(reg_fn_code) = gen_bridge_registration_fn(spec, generator) {
        writeln!(out).ok();
        writeln!(out).ok();
        out.push_str(&reg_fn_code);
    }

    BridgeOutput { imports, code: out }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a `TypeRef` as a Rust type string for use in trait method signatures.
///
/// `type_paths` qualifies `Named` types with their full Rust path (e.g., `"Config"` →
/// `"kreuzberg::Config"`). If a name isn't in `type_paths`, it's used as-is.
pub fn format_type_ref(ty: &alef_core::ir::TypeRef, type_paths: &HashMap<String, String>) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "u64",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::Usize => "usize",
            PrimitiveType::Isize => "isize",
        }
        .to_string(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Vec(inner) => format!("Vec<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            format_type_ref(k, type_paths),
            format_type_ref(v, type_paths)
        ),
        TypeRef::Named(name) => type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone()),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Format a return type, wrapping in `Result` when an error type is present.
pub fn format_return_type(
    ty: &alef_core::ir::TypeRef,
    error_type: Option<&str>,
    type_paths: &HashMap<String, String>,
) -> String {
    let inner = format_type_ref(ty, type_paths);
    match error_type {
        Some(err) => format!("std::result::Result<{inner}, {err}>"),
        None => inner,
    }
}

/// Format a parameter type, respecting `is_ref`, `is_mut`, and `optional` from the IR.
///
/// Unlike [`format_type_ref`], this function produces reference types when the
/// original Rust parameter was a `&T` or `&mut T`, and wraps in `Option<>` when
/// `param.optional` is true:
/// - `String + is_ref` → `&str`
/// - `String + is_ref + optional` → `Option<&str>`
/// - `Bytes + is_ref` → `&[u8]`
/// - `Path + is_ref` → `&std::path::Path`
/// - `Vec<T> + is_ref` → `&[T]`
/// - `Named(n) + is_ref` → `&{qualified_name}`
pub fn format_param_type(param: &ParamDef, type_paths: &HashMap<String, String>) -> String {
    use alef_core::ir::TypeRef;
    let base = if param.is_ref {
        let mutability = if param.is_mut { "mut " } else { "" };
        match &param.ty {
            TypeRef::String => format!("&{mutability}str"),
            TypeRef::Bytes => format!("&{mutability}[u8]"),
            TypeRef::Path => format!("&{mutability}std::path::Path"),
            TypeRef::Vec(inner) => format!("&{mutability}[{}]", format_type_ref(inner, type_paths)),
            TypeRef::Named(name) => {
                let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                format!("&{mutability}{qualified}")
            }
            TypeRef::Optional(inner) => {
                // Preserve the Option wrapper but apply the ref transformation to the inner type.
                // e.g. Option<String> + is_ref → Option<&str>
                //      Option<Vec<T>> + is_ref → Option<&[T]>
                let inner_type_str = match inner.as_ref() {
                    TypeRef::String => format!("&{mutability}str"),
                    TypeRef::Bytes => format!("&{mutability}[u8]"),
                    TypeRef::Path => format!("&{mutability}std::path::Path"),
                    TypeRef::Vec(v) => format!("&{mutability}[{}]", format_type_ref(v, type_paths)),
                    TypeRef::Named(name) => {
                        let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                        format!("&{mutability}{qualified}")
                    }
                    // Primitives and other Copy types: pass by value inside Option
                    other => format_type_ref(other, type_paths),
                };
                // Already wrapped in Option — return directly to avoid double-wrapping below.
                return format!("Option<{inner_type_str}>");
            }
            // All other types are Copy/small — pass by value even when is_ref is set
            other => format_type_ref(other, type_paths),
        }
    } else {
        format_type_ref(&param.ty, type_paths)
    };

    // Wrap in Option<> when the parameter is optional (e.g. `title: Option<&str>`).
    // The TypeRef::Optional arm above returns early, so this only fires for the
    // `optional: true` IR flag pattern where ty is the unwrapped inner type.
    if param.optional {
        format!("Option<{base}>")
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn make_trait_bridge_config(super_trait: Option<&str>, register_fn: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: super_trait.map(str::to_string),
            registry_getter: None,
            register_fn: register_fn.map(str::to_string),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        }
    }

    fn make_type_def(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: rust_path.to_string(),
            original_rust_path: rust_path.to_string(),
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        }
    }

    fn make_method(
        name: &str,
        params: Vec<ParamDef>,
        return_type: TypeRef,
        is_async: bool,
        has_default_impl: bool,
        trait_source: Option<&str>,
        error_type: Option<&str>,
    ) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async,
            is_static: false,
            error_type: error_type.map(str::to_string),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: trait_source.map(str::to_string),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl,
        }
    }

    fn make_param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref,
            is_mut: false,
            newtype_wrapper: None,
        }
    }

    fn make_spec<'a>(
        trait_def: &'a TypeDef,
        bridge_config: &'a TraitBridgeConfig,
        wrapper_prefix: &'a str,
        type_paths: HashMap<String, String>,
    ) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config,
            core_import: "mylib",
            wrapper_prefix,
            type_paths,
            error_type: "MyError".to_string(),
            error_constructor: "MyError::from({msg})".to_string(),
        }
    }

    // ---------------------------------------------------------------------------
    // Mock backend
    // ---------------------------------------------------------------------------

    struct MockBridgeGenerator;

    impl TraitBridgeGenerator for MockBridgeGenerator {
        fn foreign_object_type(&self) -> &str {
            "Py<PyAny>"
        }

        fn bridge_imports(&self) -> Vec<String> {
            vec!["pyo3::prelude::*".to_string(), "pyo3::types::PyString".to_string()]
        }

        fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
            format!("// sync body for {}", method.name)
        }

        fn gen_async_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
            format!("// async body for {}", method.name)
        }

        fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
            format!(
                "impl {} {{\n    pub fn new(obj: Py<PyAny>) -> Self {{ Self {{ inner: obj, cached_name: String::new() }} }}\n}}",
                spec.wrapper_name()
            )
        }

        fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
            let fn_name = spec.bridge_config.register_fn.as_deref().unwrap_or("register");
            format!("pub fn {fn_name}(obj: Py<PyAny>) {{ /* register */ }}")
        }
    }

    // ---------------------------------------------------------------------------
    // TraitBridgeSpec helpers
    // ---------------------------------------------------------------------------

    #[test]
    fn test_wrapper_name() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        assert_eq!(spec.wrapper_name(), "PyOcrBackendBridge");
    }

    #[test]
    fn test_trait_snake() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        assert_eq!(spec.trait_snake(), "ocr_backend");
    }

    #[test]
    fn test_trait_path_replaces_hyphens() {
        let trait_def = make_type_def("OcrBackend", "my-lib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        assert_eq!(spec.trait_path(), "my_lib::OcrBackend");
    }

    #[test]
    fn test_required_methods_filters_no_default_impl() {
        let methods = vec![
            make_method("process", vec![], TypeRef::String, false, false, None, None),
            make_method("initialize", vec![], TypeRef::Unit, false, true, None, None),
            make_method("detect", vec![], TypeRef::String, false, false, None, None),
        ];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let required = spec.required_methods();
        assert_eq!(required.len(), 2);
        assert!(required.iter().any(|m| m.name == "process"));
        assert!(required.iter().any(|m| m.name == "detect"));
    }

    #[test]
    fn test_optional_methods_filters_has_default_impl() {
        let methods = vec![
            make_method("process", vec![], TypeRef::String, false, false, None, None),
            make_method("initialize", vec![], TypeRef::Unit, false, true, None, None),
            make_method("shutdown", vec![], TypeRef::Unit, false, true, None, None),
        ];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let optional = spec.optional_methods();
        assert_eq!(optional.len(), 2);
        assert!(optional.iter().any(|m| m.name == "initialize"));
        assert!(optional.iter().any(|m| m.name == "shutdown"));
    }

    #[test]
    fn test_error_path() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        assert_eq!(spec.error_path(), "mylib::MyError");
    }

    // ---------------------------------------------------------------------------
    // format_type_ref
    // ---------------------------------------------------------------------------

    #[test]
    fn test_format_type_ref_primitives() {
        let paths = HashMap::new();
        let cases: Vec<(TypeRef, &str)> = vec![
            (TypeRef::Primitive(PrimitiveType::Bool), "bool"),
            (TypeRef::Primitive(PrimitiveType::U8), "u8"),
            (TypeRef::Primitive(PrimitiveType::U16), "u16"),
            (TypeRef::Primitive(PrimitiveType::U32), "u32"),
            (TypeRef::Primitive(PrimitiveType::U64), "u64"),
            (TypeRef::Primitive(PrimitiveType::I8), "i8"),
            (TypeRef::Primitive(PrimitiveType::I16), "i16"),
            (TypeRef::Primitive(PrimitiveType::I32), "i32"),
            (TypeRef::Primitive(PrimitiveType::I64), "i64"),
            (TypeRef::Primitive(PrimitiveType::F32), "f32"),
            (TypeRef::Primitive(PrimitiveType::F64), "f64"),
            (TypeRef::Primitive(PrimitiveType::Usize), "usize"),
            (TypeRef::Primitive(PrimitiveType::Isize), "isize"),
        ];
        for (ty, expected) in cases {
            assert_eq!(format_type_ref(&ty, &paths), expected, "mismatch for {expected}");
        }
    }

    #[test]
    fn test_format_type_ref_string() {
        assert_eq!(format_type_ref(&TypeRef::String, &HashMap::new()), "String");
    }

    #[test]
    fn test_format_type_ref_char() {
        assert_eq!(format_type_ref(&TypeRef::Char, &HashMap::new()), "char");
    }

    #[test]
    fn test_format_type_ref_bytes() {
        assert_eq!(format_type_ref(&TypeRef::Bytes, &HashMap::new()), "Vec<u8>");
    }

    #[test]
    fn test_format_type_ref_path() {
        assert_eq!(format_type_ref(&TypeRef::Path, &HashMap::new()), "std::path::PathBuf");
    }

    #[test]
    fn test_format_type_ref_unit() {
        assert_eq!(format_type_ref(&TypeRef::Unit, &HashMap::new()), "()");
    }

    #[test]
    fn test_format_type_ref_json() {
        assert_eq!(format_type_ref(&TypeRef::Json, &HashMap::new()), "serde_json::Value");
    }

    #[test]
    fn test_format_type_ref_duration() {
        assert_eq!(
            format_type_ref(&TypeRef::Duration, &HashMap::new()),
            "std::time::Duration"
        );
    }

    #[test]
    fn test_format_type_ref_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(format_type_ref(&ty, &HashMap::new()), "Option<String>");
    }

    #[test]
    fn test_format_type_ref_optional_nested() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
            PrimitiveType::U32,
        )))));
        assert_eq!(format_type_ref(&ty, &HashMap::new()), "Option<Option<u32>>");
    }

    #[test]
    fn test_format_type_ref_vec() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8)));
        assert_eq!(format_type_ref(&ty, &HashMap::new()), "Vec<u8>");
    }

    #[test]
    fn test_format_type_ref_vec_nested() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        assert_eq!(format_type_ref(&ty, &HashMap::new()), "Vec<Vec<String>>");
    }

    #[test]
    fn test_format_type_ref_map() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I64)),
        );
        assert_eq!(
            format_type_ref(&ty, &HashMap::new()),
            "std::collections::HashMap<String, i64>"
        );
    }

    #[test]
    fn test_format_type_ref_map_nested_value() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Vec(Box::new(TypeRef::String))),
        );
        assert_eq!(
            format_type_ref(&ty, &HashMap::new()),
            "std::collections::HashMap<String, Vec<String>>"
        );
    }

    #[test]
    fn test_format_type_ref_named_without_type_paths() {
        let ty = TypeRef::Named("Config".to_string());
        assert_eq!(format_type_ref(&ty, &HashMap::new()), "Config");
    }

    #[test]
    fn test_format_type_ref_named_with_type_paths() {
        let ty = TypeRef::Named("Config".to_string());
        let mut paths = HashMap::new();
        paths.insert("Config".to_string(), "mylib::Config".to_string());
        assert_eq!(format_type_ref(&ty, &paths), "mylib::Config");
    }

    #[test]
    fn test_format_type_ref_named_not_in_type_paths_falls_back_to_name() {
        let ty = TypeRef::Named("Unknown".to_string());
        let mut paths = HashMap::new();
        paths.insert("Other".to_string(), "mylib::Other".to_string());
        assert_eq!(format_type_ref(&ty, &paths), "Unknown");
    }

    // ---------------------------------------------------------------------------
    // format_param_type
    // ---------------------------------------------------------------------------

    #[test]
    fn test_format_param_type_string_ref() {
        let param = make_param("input", TypeRef::String, true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "&str");
    }

    #[test]
    fn test_format_param_type_string_owned() {
        let param = make_param("input", TypeRef::String, false);
        assert_eq!(format_param_type(&param, &HashMap::new()), "String");
    }

    #[test]
    fn test_format_param_type_bytes_ref() {
        let param = make_param("data", TypeRef::Bytes, true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "&[u8]");
    }

    #[test]
    fn test_format_param_type_bytes_owned() {
        let param = make_param("data", TypeRef::Bytes, false);
        assert_eq!(format_param_type(&param, &HashMap::new()), "Vec<u8>");
    }

    #[test]
    fn test_format_param_type_path_ref() {
        let param = make_param("path", TypeRef::Path, true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "&std::path::Path");
    }

    #[test]
    fn test_format_param_type_path_owned() {
        let param = make_param("path", TypeRef::Path, false);
        assert_eq!(format_param_type(&param, &HashMap::new()), "std::path::PathBuf");
    }

    #[test]
    fn test_format_param_type_vec_ref() {
        let param = make_param("items", TypeRef::Vec(Box::new(TypeRef::String)), true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "&[String]");
    }

    #[test]
    fn test_format_param_type_vec_owned() {
        let param = make_param("items", TypeRef::Vec(Box::new(TypeRef::String)), false);
        assert_eq!(format_param_type(&param, &HashMap::new()), "Vec<String>");
    }

    #[test]
    fn test_format_param_type_named_ref_with_type_paths() {
        let mut paths = HashMap::new();
        paths.insert("Config".to_string(), "mylib::Config".to_string());
        let param = make_param("cfg", TypeRef::Named("Config".to_string()), true);
        assert_eq!(format_param_type(&param, &paths), "&mylib::Config");
    }

    #[test]
    fn test_format_param_type_named_ref_without_type_paths() {
        let param = make_param("cfg", TypeRef::Named("Config".to_string()), true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "&Config");
    }

    #[test]
    fn test_format_param_type_primitive_ref_passes_by_value() {
        // Copy types like u32 are passed by value even when is_ref is set
        let param = make_param("count", TypeRef::Primitive(PrimitiveType::U32), true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "u32");
    }

    #[test]
    fn test_format_param_type_unit_ref_passes_by_value() {
        let param = make_param("nothing", TypeRef::Unit, true);
        assert_eq!(format_param_type(&param, &HashMap::new()), "()");
    }

    // ---------------------------------------------------------------------------
    // format_return_type
    // ---------------------------------------------------------------------------

    #[test]
    fn test_format_return_type_without_error() {
        let result = format_return_type(&TypeRef::String, None, &HashMap::new());
        assert_eq!(result, "String");
    }

    #[test]
    fn test_format_return_type_with_error() {
        let result = format_return_type(&TypeRef::String, Some("MyError"), &HashMap::new());
        assert_eq!(result, "Result<String, MyError>");
    }

    #[test]
    fn test_format_return_type_unit_with_error() {
        let result = format_return_type(&TypeRef::Unit, Some("Box<dyn std::error::Error>"), &HashMap::new());
        assert_eq!(result, "Result<(), Box<dyn std::error::Error>>");
    }

    #[test]
    fn test_format_return_type_named_with_type_paths_and_error() {
        let mut paths = HashMap::new();
        paths.insert("Output".to_string(), "mylib::Output".to_string());
        let result = format_return_type(&TypeRef::Named("Output".to_string()), Some("mylib::MyError"), &paths);
        assert_eq!(result, "Result<mylib::Output, mylib::MyError>");
    }

    // ---------------------------------------------------------------------------
    // gen_bridge_wrapper_struct
    // ---------------------------------------------------------------------------

    #[test]
    fn test_gen_bridge_wrapper_struct_contains_struct_name() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_wrapper_struct(&spec, &generator);
        assert!(
            result.contains("pub struct PyOcrBackendBridge"),
            "missing struct declaration in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_wrapper_struct_contains_inner_field() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_wrapper_struct(&spec, &generator);
        assert!(result.contains("inner: Py<PyAny>"), "missing inner field in:\n{result}");
    }

    #[test]
    fn test_gen_bridge_wrapper_struct_contains_cached_name() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_wrapper_struct(&spec, &generator);
        assert!(
            result.contains("cached_name: String"),
            "missing cached_name field in:\n{result}"
        );
    }

    // ---------------------------------------------------------------------------
    // gen_bridge_plugin_impl
    // ---------------------------------------------------------------------------

    #[test]
    fn test_gen_bridge_plugin_impl_returns_none_when_no_super_trait() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        assert!(gen_bridge_plugin_impl(&spec, &generator).is_none());
    }

    #[test]
    fn test_gen_bridge_plugin_impl_returns_some_when_super_trait_configured() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        assert!(gen_bridge_plugin_impl(&spec, &generator).is_some());
    }

    #[test]
    fn test_gen_bridge_plugin_impl_uses_qualified_super_trait_path() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(
            result.contains("impl mylib::Plugin for PyOcrBackendBridge"),
            "missing qualified super-trait path in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_plugin_impl_uses_already_qualified_super_trait_path() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("other_crate::Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(
            result.contains("impl other_crate::Plugin for PyOcrBackendBridge"),
            "wrong super-trait path in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_plugin_impl_contains_name_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(
            result.contains("fn name(") && result.contains("cached_name"),
            "missing name() using cached_name in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_plugin_impl_contains_version_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(result.contains("fn version("), "missing version() in:\n{result}");
    }

    #[test]
    fn test_gen_bridge_plugin_impl_contains_initialize_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(result.contains("fn initialize("), "missing initialize() in:\n{result}");
    }

    #[test]
    fn test_gen_bridge_plugin_impl_contains_shutdown_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
        assert!(result.contains("fn shutdown("), "missing shutdown() in:\n{result}");
    }

    // ---------------------------------------------------------------------------
    // gen_bridge_trait_impl
    // ---------------------------------------------------------------------------

    #[test]
    fn test_gen_bridge_trait_impl_includes_impl_header() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(
            result.contains("impl mylib::OcrBackend for PyOcrBackendBridge"),
            "missing impl header in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_trait_impl_includes_method_signatures() {
        let methods = vec![make_method(
            "process",
            vec![],
            TypeRef::String,
            false,
            false,
            None,
            None,
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(result.contains("fn process("), "missing method signature in:\n{result}");
    }

    #[test]
    fn test_gen_bridge_trait_impl_includes_method_body_from_generator() {
        let methods = vec![make_method(
            "process",
            vec![],
            TypeRef::String,
            false,
            false,
            None,
            None,
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(
            result.contains("// sync body for process"),
            "missing sync method body in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_trait_impl_async_method_uses_async_body() {
        let methods = vec![make_method(
            "process_async",
            vec![],
            TypeRef::String,
            true,
            false,
            None,
            None,
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(
            result.contains("// async body for process_async"),
            "missing async method body in:\n{result}"
        );
        assert!(
            result.contains("async fn process_async("),
            "missing async keyword in method signature in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_trait_impl_filters_trait_source_methods() {
        // Methods with trait_source set come from super-traits and should be excluded
        let methods = vec![
            make_method("own_method", vec![], TypeRef::String, false, false, None, None),
            make_method(
                "inherited_method",
                vec![],
                TypeRef::String,
                false,
                false,
                Some("other_crate::OtherTrait"),
                None,
            ),
        ];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(
            result.contains("fn own_method("),
            "own method should be present in:\n{result}"
        );
        assert!(
            !result.contains("fn inherited_method("),
            "inherited method should be filtered out in:\n{result}"
        );
    }

    #[test]
    fn test_gen_bridge_trait_impl_method_with_params() {
        let params = vec![
            make_param("input", TypeRef::String, true),
            make_param("count", TypeRef::Primitive(PrimitiveType::U32), false),
        ];
        let methods = vec![make_method(
            "process",
            params,
            TypeRef::String,
            false,
            false,
            None,
            None,
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(result.contains("input: &str"), "missing &str param in:\n{result}");
        assert!(result.contains("count: u32"), "missing u32 param in:\n{result}");
    }

    #[test]
    fn test_gen_bridge_trait_impl_return_type_with_error() {
        let methods = vec![make_method(
            "process",
            vec![],
            TypeRef::String,
            false,
            false,
            None,
            Some("MyError"),
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_trait_impl(&spec, &generator);
        assert!(
            result.contains("-> Result<String, mylib::MyError>"),
            "missing Result return type in:\n{result}"
        );
    }

    // ---------------------------------------------------------------------------
    // gen_bridge_registration_fn
    // ---------------------------------------------------------------------------

    #[test]
    fn test_gen_bridge_registration_fn_returns_none_without_register_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        assert!(gen_bridge_registration_fn(&spec, &generator).is_none());
    }

    #[test]
    fn test_gen_bridge_registration_fn_returns_some_with_register_fn() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, Some("register_ocr_backend"));
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let result = gen_bridge_registration_fn(&spec, &generator);
        assert!(result.is_some());
        let code = result.unwrap();
        assert!(
            code.contains("register_ocr_backend"),
            "missing register fn name in:\n{code}"
        );
    }

    // ---------------------------------------------------------------------------
    // gen_bridge_all
    // ---------------------------------------------------------------------------

    #[test]
    fn test_gen_bridge_all_includes_imports() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(output.imports.contains(&"pyo3::prelude::*".to_string()));
        assert!(output.imports.contains(&"pyo3::types::PyString".to_string()));
    }

    #[test]
    fn test_gen_bridge_all_includes_wrapper_struct() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("pub struct PyOcrBackendBridge"),
            "missing struct in:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_includes_constructor() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("pub fn new("),
            "missing constructor in:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_includes_trait_impl() {
        let methods = vec![make_method(
            "process",
            vec![],
            TypeRef::String,
            false,
            false,
            None,
            None,
        )];
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("impl mylib::OcrBackend for PyOcrBackendBridge"),
            "missing trait impl in:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_includes_plugin_impl_when_super_trait_set() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(Some("Plugin"), None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("impl mylib::Plugin for PyOcrBackendBridge"),
            "missing plugin impl in:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_no_plugin_impl_when_no_super_trait() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            !output.code.contains("fn name(") || !output.code.contains("cached_name"),
            "unexpected plugin impl present without super_trait"
        );
    }

    #[test]
    fn test_gen_bridge_all_includes_registration_fn_when_configured() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, Some("register_ocr_backend"));
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            output.code.contains("register_ocr_backend"),
            "missing registration fn in:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_no_registration_fn_when_absent() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        assert!(
            !output.code.contains("register_ocr_backend"),
            "unexpected registration fn present:\n{}",
            output.code
        );
    }

    #[test]
    fn test_gen_bridge_all_ordering_struct_before_trait_impl() {
        let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
        let config = make_trait_bridge_config(None, None);
        let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
        let generator = MockBridgeGenerator;
        let output = gen_bridge_all(&spec, &generator);
        let struct_pos = output.code.find("pub struct PyOcrBackendBridge").unwrap();
        let impl_pos = output
            .code
            .find("impl mylib::OcrBackend for PyOcrBackendBridge")
            .unwrap();
        assert!(struct_pos < impl_pos, "struct should appear before trait impl");
    }
}
