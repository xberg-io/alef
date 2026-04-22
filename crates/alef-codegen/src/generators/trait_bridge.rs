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
}

impl<'a> TraitBridgeSpec<'a> {
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

    // version() -> &str — delegate to foreign object
    writeln!(out, "    fn version(&self) -> &str {{").ok();
    // Build a synthetic method for version
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
        returns_ref: true,
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

    // initialize() -> Result<()> — delegate to foreign object
    writeln!(
        out,
        "    fn initialize(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {{"
    )
    .ok();
    let init_method = MethodDef {
        name: "initialize".to_string(),
        params: vec![],
        return_type: alef_core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
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

    // shutdown() -> Result<()> — delegate to foreign object
    writeln!(
        out,
        "    fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {{"
    )
    .ok();
    let shutdown_method = MethodDef {
        name: "shutdown".to_string(),
        params: vec![],
        return_type: alef_core::ir::TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
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

        // Return type
        let ret = format_return_type(&method.return_type, method.error_type.as_deref(), &spec.type_paths);

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

/// Generate the complete trait bridge code block: imports, struct, impls, and
/// optionally a registration function.
///
/// The registration function is only emitted when `bridge_config.register_fn` is set.
/// Bridges without a `register_fn` use the per-call visitor pattern instead.
pub fn gen_bridge_all(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let mut out = String::with_capacity(4096);

    // Imports
    let imports = generator.bridge_imports();
    for imp in &imports {
        writeln!(out, "use {imp};").ok();
    }
    if !imports.is_empty() {
        writeln!(out).ok();
    }

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

    out
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
        Some(err) => format!("Result<{inner}, {err}>"),
        None => inner,
    }
}

/// Format a parameter type, respecting `is_ref` and `is_mut` from the IR.
///
/// Unlike [`format_type_ref`], this function produces reference types when the
/// original Rust parameter was a `&T` or `&mut T`:
/// - `String + is_ref` → `&str`
/// - `Bytes + is_ref` → `&[u8]`
/// - `Path + is_ref` → `&std::path::Path`
/// - `Vec<T> + is_ref` → `&[T]`
/// - `Named(n) + is_ref` → `&{qualified_name}`
pub fn format_param_type(param: &ParamDef, type_paths: &HashMap<String, String>) -> String {
    use alef_core::ir::TypeRef;
    if param.is_ref {
        match &param.ty {
            TypeRef::String => "&str".to_string(),
            TypeRef::Bytes => "&[u8]".to_string(),
            TypeRef::Path => "&std::path::Path".to_string(),
            TypeRef::Vec(inner) => format!("&[{}]", format_type_ref(inner, type_paths)),
            TypeRef::Named(name) => {
                let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                format!("&{qualified}")
            }
            // All other types are Copy/small — pass by value even when is_ref is set
            other => format_type_ref(other, type_paths),
        }
    } else {
        format_type_ref(&param.ty, type_paths)
    }
}
