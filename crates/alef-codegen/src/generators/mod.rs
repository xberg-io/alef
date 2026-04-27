use ahash::AHashMap;

pub mod binding_helpers;
pub mod enums;
pub mod functions;
pub mod methods;
pub mod structs;
pub mod trait_bridge;
pub mod type_paths;

/// Map of adapter-generated method/function bodies.
/// Key: "TypeName.method_name" for methods, "function_name" for free functions.
pub type AdapterBodies = AHashMap<String, String>;

/// Async support pattern for the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncPattern {
    /// No async support
    None,
    /// PyO3: pyo3_async_runtimes::tokio::future_into_py
    Pyo3FutureIntoPy,
    /// NAPI-RS: native async fn → auto-Promise
    NapiNativeAsync,
    /// wasm-bindgen: native async fn → auto-Promise
    WasmNativeAsync,
    /// Block on Tokio runtime (Ruby, PHP)
    TokioBlockOn,
}

/// Configuration for Rust binding code generation.
pub struct RustBindingConfig<'a> {
    /// Attrs applied to generated structs, e.g. `["pyclass(frozen)"]`.
    pub struct_attrs: &'a [&'a str],
    /// Attrs applied to each field, e.g. `["pyo3(get)"]`.
    pub field_attrs: &'a [&'a str],
    /// Derives applied to generated structs, e.g. `["Clone"]`.
    pub struct_derives: &'a [&'a str],
    /// Attr wrapping the impl block, e.g. `Some("pymethods")`.
    pub method_block_attr: Option<&'a str>,
    /// Attr placed on the constructor, e.g. `"#[new]"`.
    pub constructor_attr: &'a str,
    /// Attr placed on static methods, e.g. `Some("staticmethod")`.
    pub static_attr: Option<&'a str>,
    /// Attr placed on free functions, e.g. `"#[pyfunction]"`.
    pub function_attr: &'a str,
    /// Attrs applied to generated enums, e.g. `["pyclass(eq, eq_int)"]`.
    pub enum_attrs: &'a [&'a str],
    /// Derives applied to generated enums, e.g. `["Clone", "PartialEq"]`.
    pub enum_derives: &'a [&'a str],
    /// Whether the backend requires `#[pyo3(signature = (...))]`-style annotations.
    pub needs_signature: bool,
    /// Prefix for the signature annotation, e.g. `"#[pyo3(signature = ("`.
    pub signature_prefix: &'a str,
    /// Suffix for the signature annotation, e.g. `"))]"`.
    pub signature_suffix: &'a str,
    /// Core crate import path, e.g. `"liter_llm"`. Used to generate calls into core.
    pub core_import: &'a str,
    /// Async pattern supported by this backend.
    pub async_pattern: AsyncPattern,
    /// Whether serde/serde_json are available in the output crate's dependencies.
    /// When true, the generator can use serde-based param conversion and add `serde::Serialize` derives.
    /// When false, non-convertible Named params fall back to `gen_unimplemented_body`.
    pub has_serde: bool,
    /// Prefix for binding type names (e.g. "Js" for NAPI/WASM, "" for PyO3/PHP).
    /// Used in impl block targets: `impl {prefix}{TypeName}`.
    pub type_name_prefix: &'a str,
    /// When true, non-optional Duration fields on `has_default` types are emitted as
    /// `Option<u64>` in the binding struct so that unset fields fall back to the core
    /// type's `Default` implementation rather than `Duration::ZERO`.
    /// Used by PyO3 to prevent validation failures when `request_timeout` is unset.
    pub option_duration_on_defaults: bool,
    /// Opaque type names. Structs with non-optional fields of these types
    /// skip `Default`/`Serialize`/`Deserialize` derives since opaque wrappers don't impl them.
    pub opaque_type_names: &'a [String],
}

/// Method names that conflict with standard trait methods.
/// When a generated method has one of these names, we add
/// `#[allow(clippy::should_implement_trait)]` to suppress the lint.
pub(super) const TRAIT_METHOD_NAMES: &[&str] = &[
    "default", "from", "from_str", "into", "eq", "ne", "lt", "le", "gt", "ge", "add", "sub", "mul", "div", "rem",
    "neg", "not", "index", "deref",
];

// Re-exports for backwards compatibility — callers use `crate::generators::*`.
pub use binding_helpers::{
    gen_async_body, gen_call_args, gen_call_args_with_let_bindings, gen_lossy_binding_to_core_fields,
    gen_named_let_bindings_no_promote, gen_named_let_bindings_pub, gen_serde_let_bindings, gen_unimplemented_body,
    has_named_params, is_simple_non_opaque_param, wrap_return, wrap_return_with_mutex,
};
pub use enums::{enum_has_data_variants, gen_enum, gen_pyo3_data_enum};
pub use functions::{collect_explicit_core_imports, collect_trait_imports, gen_function, has_unresolved_trait_methods};
pub use methods::{
    gen_constructor, gen_constructor_with_renames, gen_impl_block, gen_impl_block_with_renames, gen_method,
    gen_opaque_impl_block, gen_static_method, is_trait_method_name,
};
pub use structs::{
    can_generate_default_impl, gen_opaque_struct, gen_opaque_struct_prefixed, gen_struct, gen_struct_default_impl,
    gen_struct_with_per_field_attrs, gen_struct_with_rename, type_needs_mutex,
};
