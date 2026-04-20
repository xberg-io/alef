use serde::{Deserialize, Serialize};

/// Indicates the core Rust type wraps the resolved type in a smart pointer or cow.
/// Used by codegen to generate correct From/Into conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CoreWrapper {
    #[default]
    None,
    /// `Cow<'static, str>` — binding uses String, core needs `.into()`
    Cow,
    /// `Arc<T>` — binding unwraps, core wraps with `Arc::new()`
    Arc,
    /// `bytes::Bytes` — binding uses `Vec<u8>`, core needs `Bytes::from()`
    Bytes,
}

/// Typed default value for a field, enabling backends to emit language-native defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DefaultValue {
    BoolLiteral(bool),
    StringLiteral(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    EnumVariant(String),
    /// Empty collection or Default::default()
    Empty,
    /// None / null
    None,
}

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
}

/// A public struct exposed to bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: String,
    pub rust_path: String,
    pub fields: Vec<FieldDef>,
    pub methods: Vec<MethodDef>,
    pub is_opaque: bool,
    pub is_clone: bool,
    pub doc: String,
    #[serde(default)]
    pub cfg: Option<String>,
    /// True if this type was extracted from a trait definition.
    /// Trait types need `dyn` keyword when used as opaque inner types.
    #[serde(default)]
    pub is_trait: bool,
    /// True if the type implements Default (via derive or manual impl).
    /// Used by backends like NAPI to make all fields optional with defaults.
    #[serde(default)]
    pub has_default: bool,
    /// True if some fields were stripped due to `#[cfg]` conditions.
    /// When true, struct literal initializers need `..Default::default()` to fill
    /// the missing fields that may exist when the core crate is compiled with features.
    #[serde(default)]
    pub has_stripped_cfg_fields: bool,
    /// True if this type appears as a function return type.
    /// Used to select output DTO style (e.g., TypedDict for Python return types).
    #[serde(default)]
    pub is_return_type: bool,
    /// Serde `rename_all` strategy for this type (e.g., `"camelCase"`, `"snake_case"`).
    /// Used by Go/Java/C# backends to emit correct JSON tags matching Rust serde config.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// True if the type derives `serde::Serialize` and `serde::Deserialize`.
    /// Used by FFI backend to gate `from_json`/`to_json` generation — types
    /// without serde derives cannot be (de)serialized.
    #[serde(default)]
    pub has_serde: bool,
    /// Super-traits of this trait (e.g., `["Plugin"]` for `OcrBackend: Plugin`).
    /// Only populated when `is_trait` is true. Used by trait bridge codegen
    /// to determine which super-trait impls to generate.
    #[serde(default)]
    pub super_traits: Vec<String>,
}

/// A field on a public struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeRef,
    pub optional: bool,
    pub default: Option<String>,
    pub doc: String,
    /// True if this field's type was sanitized (e.g., Duration→u64, trait object→String).
    /// Fields marked sanitized cannot participate in auto-generated From/Into conversions.
    #[serde(default)]
    pub sanitized: bool,
    /// True if the core field type is `Box<T>` (or `Option<Box<T>>`).
    /// Used by FFI backends to insert proper deref when cloning field values.
    #[serde(default)]
    pub is_boxed: bool,
    /// Fully qualified Rust path for the field's type (e.g. `my_crate::types::OutputFormat`).
    /// Used by backends to disambiguate types with the same short name.
    #[serde(default)]
    pub type_rust_path: Option<String>,
    /// `#[cfg(...)]` condition string on this field, if any.
    /// Used by backends to conditionally include fields in struct literals.
    #[serde(default)]
    pub cfg: Option<String>,
    /// Typed default value for language-native default emission.
    #[serde(default)]
    pub typed_default: Option<DefaultValue>,
    /// Core wrapper on this field (Cow, Arc, Bytes). Affects From/Into codegen.
    #[serde(default)]
    pub core_wrapper: CoreWrapper,
    /// Core wrapper on Vec inner elements (e.g., `Vec<Arc<T>>`).
    #[serde(default)]
    pub vec_inner_core_wrapper: CoreWrapper,
    /// Full Rust path of the newtype wrapper that was resolved away for this field,
    /// e.g. `"my_crate::NodeIndex"` when `NodeIndex(u32)` was resolved to `u32`.
    /// When set, binding→core codegen must wrap values into the newtype
    /// (e.g. `my_crate::NodeIndex(val.field)`) and core→binding codegen must unwrap (`.0`).
    #[serde(default)]
    pub newtype_wrapper: Option<String>,
}

/// A method on a public struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodDef {
    pub name: String,
    pub params: Vec<ParamDef>,
    pub return_type: TypeRef,
    pub is_async: bool,
    pub is_static: bool,
    pub error_type: Option<String>,
    pub doc: String,
    pub receiver: Option<ReceiverKind>,
    /// True if any param or return type was sanitized during unknown type resolution.
    /// Methods with sanitized signatures cannot be auto-delegated.
    #[serde(default)]
    pub sanitized: bool,
    /// Fully qualified trait path if this method comes from a trait impl
    /// (e.g. "liter_llm::LlmClient"). None for inherent methods.
    #[serde(default)]
    pub trait_source: Option<String>,
    /// True if the core function returns a reference (`&T`, `Option<&T>`, etc.).
    /// Used by code generators to insert `.clone()` before type conversion.
    #[serde(default)]
    pub returns_ref: bool,
    /// True if the core function returns `Cow<'_, T>` where T is a named type (not str/bytes).
    /// Used by code generators to emit `.into_owned()` before type conversion.
    #[serde(default)]
    pub returns_cow: bool,
    /// Full Rust path of the newtype wrapper that was resolved away for the return type,
    /// e.g. `"my_crate::NodeIndex"` when the return type `NodeIndex(u32)` was resolved to `u32`.
    /// When set, codegen must unwrap the returned newtype value (e.g. `result.0`) before returning.
    #[serde(default)]
    pub return_newtype_wrapper: Option<String>,
    /// True if this method has a default implementation in the trait definition.
    /// Methods with defaults can be optionally implemented by the foreign object
    /// in trait bridge codegen.
    #[serde(default)]
    pub has_default_impl: bool,
}

/// How `self` is received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReceiverKind {
    Ref,
    RefMut,
    Owned,
}

/// A free function exposed to bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub rust_path: String,
    pub params: Vec<ParamDef>,
    pub return_type: TypeRef,
    pub is_async: bool,
    pub error_type: Option<String>,
    pub doc: String,
    #[serde(default)]
    pub cfg: Option<String>,
    /// True if any param or return type was sanitized during unknown type resolution.
    #[serde(default)]
    pub sanitized: bool,
    /// True if the core function returns a reference (`&T`, `Option<&T>`, etc.).
    /// Used by code generators to insert `.clone()` before type conversion.
    #[serde(default)]
    pub returns_ref: bool,
    /// True if the core function returns `Cow<'_, T>` where T is a named type (not str/bytes).
    /// Used by code generators to emit `.into_owned()` before type conversion.
    #[serde(default)]
    pub returns_cow: bool,
    /// Full Rust path of the newtype wrapper that was resolved away for the return type.
    /// When set, codegen must unwrap the returned newtype value (e.g. `result.0`).
    #[serde(default)]
    pub return_newtype_wrapper: Option<String>,
}

/// A function/method parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub ty: TypeRef,
    pub optional: bool,
    pub default: Option<String>,
    /// True if this param's type was sanitized during unknown type resolution.
    #[serde(default)]
    pub sanitized: bool,
    /// Typed default value for language-native default emission.
    #[serde(default)]
    pub typed_default: Option<DefaultValue>,
    /// True if the original Rust parameter was a reference (`&T`).
    /// Used by codegen to generate owned intermediates and pass refs.
    #[serde(default)]
    pub is_ref: bool,
    /// True if the original Rust parameter was a mutable reference (`&mut T`).
    /// Used by codegen to generate `&mut` refs when calling core functions.
    #[serde(default)]
    pub is_mut: bool,
    /// Full Rust path of the newtype wrapper that was resolved away for this param,
    /// e.g. `"my_crate::NodeIndex"` when `NodeIndex(u32)` was resolved to `u32`.
    /// When set, codegen must wrap the raw value back into the newtype when calling core:
    /// `my_crate::NodeIndex(param)` instead of just `param`.
    #[serde(default)]
    pub newtype_wrapper: Option<String>,
}

/// A public enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: String,
    pub rust_path: String,
    pub variants: Vec<EnumVariant>,
    pub doc: String,
    #[serde(default)]
    pub cfg: Option<String>,
    /// Serde tag property name for internally tagged enums (from `#[serde(tag = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_tag: Option<String>,
    /// Serde rename strategy for enum variants (from `#[serde(rename_all = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_rename_all: Option<String>,
}

/// An enum variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub doc: String,
    /// True if this variant has `#[default]` attribute (used by `#[derive(Default)]`).
    #[serde(default)]
    pub is_default: bool,
    /// Explicit serde rename for this variant (from `#[serde(rename = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_rename: Option<String>,
}

/// An error type (enum used in Result<T, E>).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDef {
    pub name: String,
    pub rust_path: String,
    pub variants: Vec<ErrorVariant>,
    pub doc: String,
}

/// An error variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorVariant {
    pub name: String,
    /// The `#[error("...")]` message template string, e.g. `"I/O error: {0}"`.
    pub message_template: Option<String>,
    /// Fields on this variant (struct or tuple fields).
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    /// True if any field has `#[source]` or `#[from]`.
    #[serde(default)]
    pub has_source: bool,
    /// True if any field has `#[from]` (auto From conversion).
    #[serde(default)]
    pub has_from: bool,
    /// True if this is a unit variant (no fields).
    #[serde(default)]
    pub is_unit: bool,
    pub doc: String,
}

/// Reference to a type, with enough info for codegen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TypeRef {
    Primitive(PrimitiveType),
    String,
    /// Rust `char` — single Unicode character. Binding layer represents as single-char string.
    Char,
    Bytes,
    Optional(Box<TypeRef>),
    Vec(Box<TypeRef>),
    Map(Box<TypeRef>, Box<TypeRef>),
    Named(String),
    Path,
    Unit,
    Json,
    Duration,
}

/// Rust primitive types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PrimitiveType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Usize,
    Isize,
}
