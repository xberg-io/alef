//! Shared trait bridge code generation.
//!
//! Generates wrapper structs that allow foreign language objects (Python, JS, etc.)
//! to implement Rust traits via FFI. Each backend implements [`TraitBridgeGenerator`]
//! to provide language-specific dispatch logic; the shared functions in this module
//! handle the structural boilerplate.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef};
use heck::ToSnakeCase;
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

    /// Generate the body of the registration function.
    ///
    /// This is the code inside the `register_xxx()` function that wraps the
    /// foreign object, builds the bridge, and inserts it into the registry.
    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String;

    /// The attribute placed on the registration function (e.g., `"#[pyfunction]"`, `"#[napi]"`).
    fn registration_fn_attr(&self) -> &str;
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

/// Generate `impl Plugin for Wrapper` when the trait has `Plugin` as a super-trait.
///
/// Forwards `name()`, `version()`, `initialize()`, and `shutdown()` to the
/// foreign object, using `cached_name` for `name()`.
pub fn gen_bridge_plugin_impl(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    if !spec.trait_def.super_traits.iter().any(|s| s == "Plugin") {
        return None;
    }

    let wrapper = spec.wrapper_name();
    let core_import = spec.core_import;

    // Build synthetic MethodDefs for the Plugin methods and delegate to the generator
    // for the actual call bodies. Use a simplified approach: generate the impl block
    // with hardcoded structure since Plugin's interface is well-known.
    let mut out = String::with_capacity(1024);
    writeln!(out, "impl {core_import}::Plugin for {wrapper} {{").ok();

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
        writeln!(out, "        {line}").ok();
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
        writeln!(out, "        {line}").ok();
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
        writeln!(out, "        {line}").ok();
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

    for (i, method) in spec.trait_def.methods.iter().enumerate() {
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

        // Build params (excluding self)
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, format_type_ref(&p.ty)))
            .collect();

        let all_params = if receiver.is_empty() {
            params.join(", ")
        } else if params.is_empty() {
            receiver.to_string()
        } else {
            format!("{}, {}", receiver, params.join(", "))
        };

        // Return type
        let ret = format_return_type(&method.return_type, method.error_type.as_deref());

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
pub fn gen_bridge_registration_fn(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> String {
    let reg_fn = &spec.bridge_config.register_fn;
    let attr = generator.registration_fn_attr();
    let mut out = String::with_capacity(1024);

    writeln!(out, "{attr}").ok();
    writeln!(out, "pub fn {reg_fn}() -> Result<(), String> {{").ok();

    let body = generator.gen_registration_fn(spec);
    for line in body.lines() {
        writeln!(out, "    {line}").ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate the complete trait bridge code block: imports, struct, impls, and
/// registration function.
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

    // Plugin super-trait impl (if applicable)
    if let Some(plugin_impl) = gen_bridge_plugin_impl(spec, generator) {
        out.push_str(&plugin_impl);
        writeln!(out).ok();
        writeln!(out).ok();
    }

    // Trait impl
    out.push_str(&gen_bridge_trait_impl(spec, generator));
    writeln!(out).ok();
    writeln!(out).ok();

    // Registration function
    out.push_str(&gen_bridge_registration_fn(spec, generator));

    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a `TypeRef` as a Rust type string for use in trait method signatures.
fn format_type_ref(ty: &alef_core::ir::TypeRef) -> String {
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
        TypeRef::Optional(inner) => format!("Option<{}>", format_type_ref(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", format_type_ref(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            format_type_ref(k),
            format_type_ref(v)
        ),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Format a return type, wrapping in `Result` when an error type is present.
fn format_return_type(ty: &alef_core::ir::TypeRef, error_type: Option<&str>) -> String {
    let inner = format_type_ref(ty);
    match error_type {
        Some(err) => format!("Result<{inner}, {err}>"),
        None => inner,
    }
}
