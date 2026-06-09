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
    /// `Arc<Mutex<T>>` — binding wraps with `Arc::new(Mutex::new())`, methods call `.lock()`
    ArcMutex,
    /// `Box<str>` — binding uses String, core needs `.into()` (same shape as Cow
    /// but distinct so backends can keep wrapper-specific behavior addressable).
    Box,
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

/// Deprecation metadata extracted from `#[deprecated(...)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeprecationInfo {
    /// Version when the item was deprecated (from `#[deprecated(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation note (from `#[deprecated(note = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Version annotation on an IR item.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionAnnotation {
    /// Version when this item was introduced (from `#[alef(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation info (from `#[deprecated(...)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationInfo>,
}

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    /// Type names → fully qualified rust_paths for types that were extracted but
    /// then excluded from the public binding surface. Preserved so trait_bridge
    /// codegen can still reference them by qualified path when they appear in
    /// trait method signatures (e.g. `Renderer::render(&HiddenDocument)`).
    #[serde(default)]
    pub excluded_type_paths: std::collections::HashMap<String, String>,
    /// Subset of `excluded_type_paths` keys whose underlying definition is a trait
    /// (`is_trait = true` on the original `TypeDef`). The `is_trait` flag is lost
    /// when the type is stripped, so trait-bridge codegen tracks excluded traits
    /// separately to decide whether a return-type `Named(name)` referencing an
    /// excluded item is a non-bridgeable trait object (skip the method, fall back
    /// to default impl) or a struct/enum still usable via its qualified path.
    #[serde(default)]
    pub excluded_trait_names: std::collections::HashSet<String>,
    /// Descriptions of owner/builder service types with their constructor,
    /// configurator methods, registration points, and run/finalize entrypoints.
    ///
    /// Populated by the service extraction pass when `[[crates.services]]` config
    /// entries are present. Empty for consumers that have not configured any services.
    #[serde(default)]
    pub services: Vec<ServiceDef>,
    /// Async trait contracts that service registration callbacks must satisfy.
    ///
    /// Each entry describes the trait, its dispatch method, and the wire
    /// request/response DTO names the callback receives and returns.
    ///
    /// Populated alongside [`Self::services`]. Empty when no services are configured.
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractDef>,
    /// Public Rust items that Alef saw during extraction but intentionally did not
    /// lower into the binding IR because their shape is not safely representable.
    ///
    /// Validation turns these into hard diagnostics instead of letting public
    /// items disappear silently before generation.
    #[serde(default)]
    pub unsupported_public_items: Vec<UnsupportedPublicItem>,
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}

/// Describes an owner/builder type that acts as a service configurator and runner.
///
/// A service has:
/// - A `constructor` that creates the initial owner instance.
/// - Zero or more `configurators` — chaining methods that set options without callbacks.
/// - Zero or more `registrations` — methods that bind a callback to a route/channel/slot.
/// - One or more `entrypoints` — `run` (long-lived async) or `finalize` (consuming transform).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDef {
    /// Short name of the owner type (e.g. `"App"`).
    pub name: String,
    /// Fully-qualified Rust path to the owner type (e.g. `"my_crate::App"`).
    pub rust_path: String,
    /// The constructor method (e.g. `new`).
    pub constructor: MethodDef,
    /// Chaining methods that mutate configuration but take no callback.
    pub configurators: Vec<MethodDef>,
    /// Registration methods that accept a callback and optional metadata.
    pub registrations: Vec<RegistrationDef>,
    /// Long-lived run entrypoints or transforming finalize methods.
    pub entrypoints: Vec<EntrypointDef>,
    /// Documentation extracted from the owner type.
    pub doc: String,
    /// Optional `#[cfg(...)]` condition string.
    pub cfg: Option<String>,
}

/// A registration method on a service — binds a host-language callback to a slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationDef {
    /// Method name on the owner type (e.g. `"add_route"`).
    pub method: String,
    /// The parameter name that carries the callback (e.g. `"handler"`).
    pub callback_param: String,
    /// Name of the [`HandlerContractDef`] this callback must satisfy
    /// (references [`HandlerContractDef::trait_name`]).
    pub callback_contract: String,
    /// Non-callback parameters (e.g. path pattern, HTTP method).
    pub metadata_params: Vec<ParamDef>,
    /// How `self` is received (`&self`, `&mut self`, or owned).
    pub receiver: Option<ReceiverKind>,
    /// Return type of the registration method.
    pub return_type: TypeRef,
    /// Error type if the registration is fallible.
    pub error_type: Option<String>,
    /// Documentation extracted from the method.
    pub doc: String,
    /// Named shortcuts over this registration with pre-resolved pinned values.
    /// Empty for registrations declared without `[[registrations.variants]]`.
    #[serde(default)]
    pub variants: Vec<RegistrationVariant>,
}

/// How backends should emit a registration variant's host-language surface.
///
/// This controls whether the variant is exposed as a builder/decorator factory
/// (returning a callable that accepts the handler), a direct verb-style method
/// (accepting the handler inline), or both.
///
/// The default is [`RegistrationVariantStyle::Hybrid`], which preserves the
/// pre-existing behaviour of emitting both forms so existing consumers are not
/// broken by this IR extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationVariantStyle {
    /// Emit only the builder/decorator-factory form:
    /// Python: `app.get(path)` returns a decorator; Ruby: `app.get(path) { |req| … }`.
    Builder,
    /// Emit only the verb-decorator form:
    /// Python: `app.get(path, handler)` or `@app.get(path)` (standard decorator);
    /// Ruby: `app.get(path) { |req| … }` (same block form, handler as block).
    ///
    /// In Python this means a single method whose last positional argument is the
    /// handler callable — matching FastAPI / Flask / Sinatra idioms.
    VerbDecorator,
    /// Emit both [`RegistrationVariantStyle::Builder`] and
    /// [`RegistrationVariantStyle::VerbDecorator`] forms.
    ///
    /// This is the default, preserving backward compatibility for consumers that
    /// have not yet declared an explicit `style`.
    #[default]
    Hybrid,
}

/// A named shortcut over a [`RegistrationDef`] with one or more pinned
/// parameter values resolved at extract time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationVariant {
    /// Shortcut name as declared in `alef.toml` (e.g. `"get"`).
    pub name: String,
    /// Resolved overrides — one per pinned base metadata-param.
    pub overrides: Vec<RegistrationVariantOverride>,
    /// When the base registration consumes a wrapper-typed metadata param whose
    /// type carries a static constructor (e.g. `route(builder: RouteBuilder, …)`
    /// where `RouteBuilder::new(method, path)` exists), the extractor pre-builds
    /// the call recipe here. Backends render this directly — composing the
    /// constructor invocation with `Fixed` args substituted in and `Free` args
    /// pulled from the variant's own signature.
    ///
    /// `None` when no metadata param matches the wrapper pattern; in that case
    /// `overrides` targets the base metadata params directly and the variant's
    /// signature is just the non-overridden subset.
    #[serde(default)]
    pub wrapper_call: Option<WrapperConstructorCall>,
    /// The variant's user-facing signature in canonical order: the non-fixed
    /// constructor args (when `wrapper_call` is set) or the non-overridden base
    /// metadata params (otherwise). Each entry carries the host param's name +
    /// type so backends can render them in language-idiomatic shape.
    pub signature_params: Vec<ParamDef>,
    /// Optional documentation for the variant. When absent, backends emit a
    /// generic docstring referencing the base registration.
    #[serde(default)]
    pub doc: Option<String>,
    /// How backends should expose this variant's host-language surface.
    ///
    /// Defaults to [`RegistrationVariantStyle::Hybrid`] so that consumers which
    /// do not declare an explicit `style` in `alef.toml` get the previous
    /// behaviour (both direct-method and decorator-factory forms emitted).
    #[serde(default)]
    pub style: RegistrationVariantStyle,
}

/// A resolved pin: the param being overridden and the expression to substitute
/// at the call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationVariantOverride {
    /// Name of the param this override pins. When `wrapper_call` is set on the
    /// parent variant this matches a wrapper-constructor param; otherwise it
    /// matches a base [`RegistrationDef::metadata_params`] entry.
    pub param_name: String,
    /// Verbatim expression substituted for the param at the call site. For
    /// enum overrides this is the fully-qualified Rust path
    /// (e.g. `"my_crate::Method::GET"`); for other types it is the raw value
    /// expression supplied by the library author.
    pub value_expr: String,
}

/// Pre-resolved recipe for building a wrapper metadata-param value via a
/// static constructor with a mix of pinned and free arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperConstructorCall {
    /// Name of the base metadata param the constructor result is bound to
    /// (e.g. `"builder"`).
    pub metadata_param: String,
    /// Fully-qualified Rust path of the wrapper type
    /// (e.g. `"my_crate::RouteBuilder"`).
    pub wrapper_type_path: String,
    /// Bare type name of the wrapper (e.g. `"RouteBuilder"`).
    pub wrapper_type_name: String,
    /// Constructor method name on the wrapper type (e.g. `"new"`).
    pub constructor_method: String,
    /// Constructor args in declared order. Each is either fixed (with a value
    /// expression substituted in) or free (taken from the variant's signature).
    pub args: Vec<WrapperConstructorArg>,
}

/// One argument in a [`WrapperConstructorCall`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WrapperConstructorArg {
    /// Pinned at the call site to `value_expr`.
    Fixed {
        /// Name of the wrapper constructor param (e.g. `"method"`).
        param_name: String,
        /// Verbatim expression substituted at the call site.
        value_expr: String,
    },
    /// Pulled from the variant's own signature at the call site.
    Free {
        /// Definition of the free param — name + type drive both the variant's
        /// signature and the constructor call argument position.
        param: ParamDef,
    },
}

/// An entrypoint on a service — either a long-lived async `run` or a consuming `finalize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrypointDef {
    /// Method name (e.g. `"run"` or `"into_router"`).
    pub method: String,
    /// Whether this is a blocking runner or a transforming finalizer.
    pub kind: EntrypointKind,
    /// True when the method is `async` or returns a `Future`.
    pub is_async: bool,
    /// Non-self parameters accepted by the entrypoint (e.g. socket address).
    pub params: Vec<ParamDef>,
    /// Return type (e.g. `()` for `run`, `Router` for `finalize`).
    pub return_type: TypeRef,
    /// Error type if the entrypoint is fallible.
    pub error_type: Option<String>,
    /// Documentation extracted from the method.
    pub doc: String,
}

/// Discriminates between the two kinds of service entrypoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointKind {
    /// A long-running async method that drives the service until shutdown.
    Run,
    /// A consuming transform that converts the builder into another type (e.g. a router).
    Finalize,
}

/// An async trait that registered service callbacks must satisfy.
///
/// Does **not** duplicate the trait's [`TypeDef`] already in [`ApiSurface::types`];
/// instead it cross-references by name and adds service-specific metadata:
/// the dispatch method, optional overrides, and the wire DTO names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerContractDef {
    /// Trait name as it appears in the surface (e.g. `"Handler"`).
    pub trait_name: String,
    /// Fully-qualified Rust path (e.g. `"my_crate::Handler"`).
    pub rust_path: String,
    /// The primary async dispatch method backends must implement.
    pub dispatch: MethodDef,
    /// Methods with default implementations that backends may optionally override.
    pub optional_methods: Vec<MethodDef>,
    /// Name of the wire request DTO the dispatch method receives (e.g. `"RequestData"`).
    /// When `None`, the dispatch method signature is used verbatim.
    pub wire_request_type: Option<String>,
    /// Name of the wire response DTO the dispatch method returns (e.g. `"ResponseData"`).
    /// When `None`, the dispatch method return type is used verbatim.
    pub wire_response_type: Option<String>,
    /// Verbatim parameter declarations inserted before the wire request parameter in the
    /// generated dispatch signature and ignored in the body. Empty by default.
    pub dispatch_extra_params: Vec<String>,
    /// Name of the wire request parameter in the generated dispatch signature
    /// (defaults to `"request"` when `None`).
    pub wire_param_name: Option<String>,
    /// Verbatim return type for the generated dispatch future's `Output`. When `None`,
    /// the bridge synthesizes `Result<{wire_response}, Box<dyn Error + Send + Sync>>`.
    pub dispatch_return_type: Option<String>,
    /// Path to a library function converting the bridge outcome into
    /// [`Self::dispatch_return_type`]. When `None`, the wire response is returned directly.
    pub response_adapter: Option<String>,
    /// Documentation extracted from the trait.
    pub doc: String,
}

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

/// Reference to a type, with enough info for codegen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
    #[default]
    Unit,
    Json,
    Duration,
}

impl TypeRef {
    /// Returns true if this type reference contains `Named(name)` at any depth.
    pub fn references_named(&self, name: &str) -> bool {
        match self {
            Self::Named(n) => n == name,
            Self::Optional(inner) | Self::Vec(inner) => inner.references_named(name),
            Self::Map(k, v) => k.references_named(name) || v.references_named(name),
            _ => false,
        }
    }
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
