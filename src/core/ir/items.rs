use serde::{Deserialize, Serialize};

use super::{CoreWrapper, DefaultValue, TypeRef, VersionAnnotation};

/// A public struct exposed to bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: String,
    pub rust_path: String,
    /// Original rust_path before path mapping rewrites. Used for From impl
    /// targets to avoid orphan rule violations when core_import is a re-export facade.
    #[serde(default)]
    pub original_rust_path: String,
    pub fields: Vec<FieldDef>,
    pub methods: Vec<MethodDef>,
    pub is_opaque: bool,
    pub is_clone: bool,
    /// True if the type derives `Copy` (or is bitwise-copyable).
    /// Used by FFI codegen to avoid emitting `.clone()` (which trips clippy::clone_on_copy).
    #[serde(default)]
    pub is_copy: bool,
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
    /// Super-traits of this trait (e.g., `["Plugin"]` for `WorkerBackend: Plugin`).
    /// Only populated when `is_trait` is true. Used by trait bridge codegen
    /// to determine which super-trait impls to generate.
    #[serde(default)]
    pub super_traits: Vec<String>,
    /// True when source metadata explicitly excludes this type/trait from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// True when this type appears as the wrapper of one or more registration
    /// variants — i.e. its name matches a
    /// [`RegistrationVariant::wrapper_call`]'s
    /// [`WrapperConstructorCall::wrapper_type_name`]. Backends use this flag
    /// to opt the type's static `new` constructor into host-language
    /// constructor emission (e.g. `#[new]` for pyo3, `#[napi(constructor)]`
    /// for napi-rs), so that variant bodies which call
    /// `WrapperType(args...)` resolve to a real instance rather than a
    /// "cannot create instances" runtime error.
    #[serde(default)]
    pub is_variant_wrapper: bool,
    /// True when the core Rust type has one or more lifetime parameters
    /// (e.g. `NodeContext<'a>`). Backends use this flag to emit `From<T<'_>>`
    /// instead of `From<T>`, `serde_json::from_str::<T<'static>>`, and
    /// opaque wrapper newtypes `pub struct Wrapper(pub Source<'static>)`.
    #[serde(default)]
    pub has_lifetime_params: bool,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

/// A field on a public struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Explicit `#[serde(rename = "...")]` on this field, if any. Preserved so binding
    /// structs that mirror the core struct can serialize/deserialize using the same wire
    /// names (e.g. core `tool_type` with `#[serde(rename = "type")]` round-trips as `"type"`).
    #[serde(default)]
    pub serde_rename: Option<String>,
    /// True when the field carries `#[serde(flatten)]`. Backends use this to emit
    /// language-native flatten support: Jackson `@JsonAnyGetter`/`@JsonAnySetter`
    /// in Java, `[JsonExtensionData]` in C# — both keyed `Map<String, Object>` /
    /// `Dictionary<string, JsonElement>` so unknown sibling fields land under the
    /// flattened bag instead of being rejected.
    #[serde(default)]
    pub serde_flatten: bool,
    /// True when source metadata explicitly excludes this field from generated
    /// polyglot binding surfaces.
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// Original Rust type string before sanitization (e.g. `"Vec<(String, String)>"`).
    /// Populated by `sanitize_unknown_types()` when a type is downgraded.
    /// Allows backends to reconstruct proper serialization/deserialization logic
    /// even when the sanitized `ty` field only carries the simplified type.
    #[serde(default)]
    pub original_type: Option<String>,
}

/// A method on a public struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// (e.g. "sample_llm::LlmClient"). None for inherent methods.
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
    /// True when source metadata explicitly excludes this method from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

/// How `self` is received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReceiverKind {
    Ref,
    RefMut,
    Owned,
}

/// A free function exposed to bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub rust_path: String,
    #[serde(default)]
    pub original_rust_path: String,
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
    /// True if the return type was sanitized (Named replaced with String).  When true,
    /// the binding-side return type is wider than the actual core return — codegen must
    /// JSON-serialize the core value rather than treating it as the binding type.
    #[serde(default)]
    pub return_sanitized: bool,
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
    /// True when source metadata explicitly excludes this function from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
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
    /// Original Rust type before sanitization, stored when param.sanitized=true.
    /// Allows codegen to reconstruct proper deserialization logic.
    /// E.g. `"Vec<(PathBuf, Option<FileExtractionConfig>)>"` when sanitized to `Vec<String>`.
    #[serde(default)]
    pub original_type: Option<String>,
    /// True when the original Rust map container was `AHashMap` (from the `ahash` crate)
    /// rather than `std::collections::HashMap`. FFI codegen uses this to emit the correct
    /// deserialization target type and runtime conversion.
    #[serde(default)]
    pub map_is_ahash: bool,
    /// True when the map's key type in the original Rust source was `Cow<'_, str>` (resolved
    /// to `TypeRef::String` in the IR). FFI codegen uses this to emit a `Cow::Owned(k)`
    /// conversion when constructing the `AHashMap` expected by the core function.
    #[serde(default)]
    pub map_key_is_cow: bool,
    /// True when the original Rust slice/vec element type was a reference (`&T`),
    /// e.g. `&[&str]` or `Vec<&str>`. FFI codegen uses this to insert a `Vec<&T>` intermediate
    /// when calling the core function (since `&Vec<T>` coerces to `&[T]`,
    /// not `&[&T]`).
    #[serde(default)]
    pub vec_inner_is_ref: bool,
    /// True when the original Rust map container was `BTreeMap` rather than `HashMap`.
    /// Codegen uses this to convert the binding-layer `HashMap` into `BTreeMap` at the
    /// call site, since the two types are not directly interchangeable.
    #[serde(default)]
    pub map_is_btree: bool,
    /// Core wrapper on this parameter (Cow, Arc, etc.). Affects call-site codegen.
    /// When `CoreWrapper::Cow`, the binding layer passes `String`/`Option<String>` but the
    /// core function expects `Cow<'_, str>`/`Option<Cow<'_, str>>`. Codegen must insert
    /// `.into()` / `.map(std::borrow::Cow::Owned)` at the call site.
    #[serde(default)]
    pub core_wrapper: CoreWrapper,
}

impl Default for ParamDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            ty: TypeRef::Unit,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: CoreWrapper::None,
        }
    }
}

/// A public enum.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: String,
    pub rust_path: String,
    #[serde(default)]
    pub original_rust_path: String,
    pub variants: Vec<EnumVariant>,
    pub doc: String,
    #[serde(default)]
    pub cfg: Option<String>,
    /// True if the enum derives `Copy`. Only unit-variant enums can derive Copy.
    /// Used by FFI codegen to avoid emitting `.clone()` (which trips clippy::clone_on_copy).
    #[serde(default)]
    pub is_copy: bool,
    /// True if the enum derives both `serde::Serialize` and `serde::Deserialize`.
    /// Used by host-language emission (e.g. Swift `Codable`) to gate JSON-bridge conformance.
    #[serde(default)]
    pub has_serde: bool,
    /// Serde tag property name for internally tagged enums (from `#[serde(tag = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_tag: Option<String>,
    /// True when the enum has `#[serde(untagged)]`.
    /// Absence of `serde_tag` does NOT imply untagged — it means externally-tagged (the serde
    /// default). Only set this when the attribute is explicitly present on the Rust type.
    #[serde(default)]
    pub serde_untagged: bool,
    /// Serde rename strategy for enum variants (from `#[serde(rename_all = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_rename_all: Option<String>,
    /// True when source metadata explicitly excludes this enum from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// Variants that were stripped from `variants` because they are variant-level
    /// `binding_excluded` (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    /// Retained here so backends that generate exhaustive Rust match expressions
    /// (e.g. Dart FRB `From<CoreType>` impls) can emit `unreachable!()` arms for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_variants: Vec<EnumVariant>,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

/// An enum variant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// True if this is a tuple variant (unnamed fields like `Variant(T1, T2)`).
    /// False for struct variants with named fields or unit variants.
    #[serde(default)]
    pub is_tuple: bool,
    /// True when source metadata explicitly excludes this variant from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_exclusion_reason: Option<String>,
    /// True when this variant had data fields in the source, but all were stripped by
    /// `strip_binding_excluded` (i.e. all fields carry `#[cfg_attr(alef, alef(skip))]`).
    /// Used by codegen to emit wildcard patterns (`{ .. }` or `(..)`) rather than a bare
    /// unit pattern, which would be a compiler error against the real core type.
    #[serde(default)]
    pub originally_had_data_fields: bool,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

/// An error type (enum used in Result<T, E>).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDef {
    pub name: String,
    pub rust_path: String,
    #[serde(default)]
    pub original_rust_path: String,
    pub variants: Vec<ErrorVariant>,
    pub doc: String,
    /// Whitelisted introspection methods on the error enum (e.g. `status_code`,
    /// `is_transient`, `error_type`). Only methods explicitly opted in via the
    /// extractor whitelist are populated here — Rust-only ergonomic helpers are
    /// intentionally excluded so backends do not accidentally expose them.
    #[serde(default)]
    pub methods: Vec<MethodDef>,
    /// True when source metadata explicitly excludes this error type from generated
    /// polyglot binding surfaces (via `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]`).
    #[serde(default)]
    pub binding_excluded: bool,
    /// Human-readable reason for `binding_excluded`, used in diagnostics.
    #[serde(default)]
    pub binding_exclusion_reason: Option<String>,
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

/// An error variant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// True if this is a tuple variant (unnamed fields like `Variant(T1, T2)`).
    /// Needed by codegen to emit the correct wildcard pattern (`(..)`) when all
    /// fields are `binding_excluded` and stripped before codegen runs.
    #[serde(default)]
    pub is_tuple: bool,
    pub doc: String,
}
