use serde::{Deserialize, Serialize};

use super::metadata::{CoreWrapper, DefaultValue, ErrorTaxonomy, SerdeContainerConversion, VersionAnnotation};
use super::type_ref::TypeRef;

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
    /// True when the container itself carries `#[serde(default)]` / `#[serde(default = "path")]`.
    ///
    /// A container-level serde default makes **every** field of the struct absent-tolerant on
    /// the wire: a missing key is filled from the container's `Default` (or the named function),
    /// so no field is required in the payload. This is a strictly different fact from
    /// [`TypeDef::has_default`], which only says the type *has* a `Default` impl — a struct can
    /// implement `Default` while every field stays required at the serde level. Backends that
    /// decide per-field wire-optionality (e.g. Go's `omitempty` pointer fields) must consult
    /// this flag and never substitute `has_default` for it. ~keep
    #[serde(default)]
    pub serde_container_default: bool,
    /// Container-level `#[serde(from/into/try_from/transparent)]`, when present. Marks a
    /// struct whose wire shape is NOT the derived, field-by-field object serde would otherwise
    /// produce -- e.g. `from`/`into` route it through a hand-written `From` whose logic (and
    /// thus the real shape) alef cannot see; `transparent` collapses it to the struct's single
    /// field with no wrapping object. Must gate validation instead of letting codegen silently
    /// emit an object DTO that disagrees with the real payload. See
    /// [`crate::core::ir::SerdeContainerConversion`] for why this is one field, not four. ~keep
    #[serde(default)]
    pub serde_container_conversion: SerdeContainerConversion,
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
    /// [`crate::core::ir::RegistrationVariant::wrapper_call`]'s
    /// [`crate::core::ir::WrapperConstructorCall::wrapper_type_name`]. Backends use this flag
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
    /// True when the core struct has one or more non-`pub` fields (e.g. `pub(crate)`).
    /// Those fields are filtered out of the binding surface, but their existence means
    /// the core type cannot be built with struct-literal syntax from a foreign crate
    /// (`error[E0451]` / "cannot construct ... due to private fields"). Backends use this
    /// flag to pick a non-literal construction strategy in binding→core conversions
    /// (public constructor, `Default`-seeded builder, or serde `Deserialize`).
    #[serde(default)]
    pub has_private_fields: bool,
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
    /// Path from `#[serde(with = "...")]` / `#[serde(serialize_with = "...")]` on this field.
    ///
    /// Presence means the field's wire shape comes from a hand-written codec, **not** from
    /// serde's derive. Backends that special-case a type's derived wire shape (e.g. Go's
    /// `DurationMillis`, which encodes `std::time::Duration` as `{"secs":_,"nanos":_}`) must
    /// consult this and fall back to the underlying scalar, because the derive shape is simply
    /// wrong for such a field — the common `duration_ms` convention writes a bare millisecond
    /// integer. Alef cannot know what an arbitrary codec emits, so it must stop asserting a
    /// shape it did not derive.
    #[serde(default)]
    pub serde_with: Option<String>,
    /// True when the field carries `#[serde(skip_serializing_if = "...")]`.
    ///
    /// The field is still a normal, required member of the Rust struct — this is purely a
    /// wire-format fact: serde omits the JSON key entirely (not `null`) whenever the predicate
    /// holds, most commonly `Option::is_none` or `Vec::is_empty` on a field that is not itself
    /// `Option<T>`. Distinct from `optional` (which tracks `Option<T>`-ness of the Rust type):
    /// a `Vec<T>` field with `skip_serializing_if = "Vec::is_empty"` is `optional: false` but
    /// `serde_skip_serializing_if: true`, and a JSON-tree-walking accessor generator must guard
    /// against the key being absent even though the underlying Rust value is never `None`.
    #[serde(default)]
    pub serde_skip_serializing_if: bool,
    /// True when the field carries a bare `#[serde(skip)]` (also matching
    /// `#[cfg_attr(..., serde(skip))]`), which excludes it from BOTH `Serialize` and
    /// `Deserialize` — distinct from [`Self::serde_skip_serializing_if`], which only omits the
    /// wire key while the field remains readable on deserialize.
    ///
    /// A field with this flag set can never receive a caller-supplied value through any code
    /// path that goes through `Deserialize` — most notably an FFI `_from_json` constructor,
    /// which is the *only* construction path for a non-opaque type. Backends that emit a getter
    /// for such a field on such a type must prove some other write path exists (e.g. a
    /// `[[crates.trait_bridges]] bind_via = "options_field"` setter naming this exact field)
    /// before emitting one; otherwise the getter is documented as returning caller-owned data
    /// while being structurally unreachable, i.e. always the type's default value.
    #[serde(default)]
    pub serde_skip: bool,
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
    /// Version annotation (since, deprecated).
    #[serde(default)]
    pub version: VersionAnnotation,
}

impl FieldDef {
    /// The `#[cfg(...)]` condition for a reference to this field (an accessor method, its
    /// registration, or a constructor's per-field conversion) that also sits behind `owner_cfg`
    /// (the owning type's own gate): the AND of the two.
    ///
    /// Mirrors [`MethodDef::cfg_within`] exactly -- a field's own `cfg` is the same kind of
    /// "this reference needs a feature the owner's gate does not already imply" fact a method's
    /// is, so the two share one combination rule rather than each emission site re-deriving it.
    #[must_use]
    pub fn cfg_within(&self, owner_cfg: Option<&str>) -> Option<String> {
        match (owner_cfg, self.cfg.as_deref()) {
            (Some(owner), Some(own)) if owner.trim() == own.trim() => Some(owner.to_string()),
            (Some(owner), Some(own)) => Some(crate::codegen::cfg::combine_gates(owner, own)),
            (Some(owner), None) => Some(owner.to_string()),
            (None, own) => own.map(str::to_string),
        }
    }
}

/// Stable prefix `pipeline::extract`'s adapter-marking pass writes into a method's
/// [`MethodDef::binding_exclusion_reason`] when an `[[crates.adapters]]` entry's
/// `(owner_type, core_path)` names it.
///
/// `binding_exclusion_reason` is documented as "used in diagnostics" -- free text, not a
/// contract -- but this one prefix is the sole exception: it is also read back by
/// `crate::e2e::codegen::call_ir` to recognize the one call-resolution case
/// [`crate::e2e::codegen::call_ir::CallIr::signature`]'s method fallback cannot see through on
/// its own. An adapter can re-expose a method's calling convention under a different shape --
/// often a same-named sibling free function, kept purely to give e2e config a Rust-side
/// identifier to point `[e2e.calls.*]` at -- and that sibling can be independently excluded from
/// `ApiSurface.functions` (its own `#[alef::skip]`, or a crate-wide `exclude.functions` entry
/// matching its bare name) without anything re-checking whether the *method's* exclusion reason
/// still means what the fallback assumes. Reading this marker back is asking the extractor's own
/// answer rather than re-deriving it, per this repo's rule against two components computing the
/// same fact separately. ~keep
pub(crate) const ADAPTER_HANDLED_REASON_PREFIX: &str = "handled by [[crates.adapters]]";

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
    /// `#[cfg(...)]` condition string on this method, AND-combined with its `impl` block's
    /// own `#[cfg(...)]` (`all(<impl>, <method>)`).
    ///
    /// One `ApiSurface` is extracted once and handed to every backend, each with its own
    /// `features_for_language`, so the gate cannot be resolved at extraction time — it must
    /// travel in the IR. Backends that generate Rust source re-emit it verbatim
    /// ([`MethodDef::rust_cfg_attribute`]); backends that emit a single host-language surface
    /// drop the method when their feature set does not satisfy it
    /// ([`crate::core::ir::ApiSurface::with_cfg_filtered_deep`]).
    #[serde(default)]
    pub cfg: Option<String>,
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

impl MethodDef {
    /// True when this is a static method that returns a borrowed reference to its own
    /// opaque owner type (e.g. `Registry::global() -> &'static Registry`).
    ///
    /// The C FFI backend cannot box a borrow into an owned `*mut T` handle, so it never
    /// exports a `{prefix}_{type}_{method}` symbol for such accessors. Language backends
    /// (Java, C#, Zig) must consult this predicate so they do NOT bind a symbol the cdylib
    /// does not provide — otherwise eager symbol resolution (Java) crashes at class-init
    /// and lazy resolution (C#, Zig) fails on first call.
    ///
    /// Keep this in lockstep with `is_static_constructor` in the FFI backend: that emitter
    /// skips static `returns_ref` methods, so every backend must agree on the absent symbol.
    pub fn returns_ref_to_owner(&self, owner_type_name: &str) -> bool {
        self.is_static
            && self.returns_ref
            && matches!(&self.return_type, TypeRef::Named(name) if name == owner_type_name)
    }

    /// True when this method's `#[cfg(...)]` gate is satisfied by `enabled_features`.
    ///
    /// The sibling of [`Self::returns_ref_to_owner`]: a per-method predicate every backend
    /// consults, so the gate is evaluated in one place rather than re-derived at each
    /// emission site. Backends that generate Rust source should prefer
    /// [`Self::rust_cfg_attribute`] — `rustc` resolves the gate for them, and unlike this
    /// predicate it also honours non-feature leaves such as `target_arch`.
    #[must_use]
    pub fn cfg_satisfied(&self, enabled_features: &std::collections::HashSet<&str>) -> bool {
        super::surface::cfg_feature_satisfied(self.cfg.as_deref(), enabled_features)
    }

    /// The `#[cfg(...)]` condition for an emitted item that also sits behind `owner_cfg`
    /// (its owning type's or `impl` block's gate): the AND of the two.
    ///
    /// Identical gates collapse to one operand rather than `all(X, X)` — the common case, since
    /// a gated type is normally implemented in an `impl` block carrying the same gate, and a
    /// method inherits its block's gate at extraction time.
    #[must_use]
    pub fn cfg_within(&self, owner_cfg: Option<&str>) -> Option<String> {
        match (owner_cfg, self.cfg.as_deref()) {
            (Some(owner), Some(own)) if owner.trim() == own.trim() => Some(owner.to_string()),
            (Some(owner), Some(own)) => Some(crate::codegen::cfg::combine_gates(owner, own)),
            (Some(owner), None) => Some(owner.to_string()),
            (None, own) => own.map(str::to_string),
        }
    }

    /// The `#[cfg(...)]` attribute line to emit above this method in generated Rust source,
    /// including the trailing newline; empty when the method is ungated.
    ///
    /// Backends that emit Rust (FFI, pyo3, napi, magnus, rustler, Dart's bridge crate) must
    /// re-emit the gate rather than filter on it: their generated crate is compiled with the
    /// core crate's features, so `rustc` decides. Backends that emit a single host-language
    /// surface cannot express the gate and must filter instead.
    #[must_use]
    pub fn rust_cfg_attribute(&self) -> String {
        self.cfg
            .as_deref()
            .map(|cfg| format!("#[cfg({cfg})]\n"))
            .unwrap_or_default()
    }
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
    /// Associated functions (static factory methods) declared in `impl EnumName { ... }`.
    /// Only associated functions (no `self` receiver, i.e. `is_static = true`) are collected here.
    /// Instance methods on enums are not expressible across most FFI boundaries and are excluded.
    #[serde(default)]
    pub methods: Vec<MethodDef>,
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
    /// True when the core enum implements `Default` (via `#[derive(Default)]` with a
    /// `#[default]` variant, OR a manual `impl Default for Enum`). Used by data-enum
    /// wrapper codegen to decide whether to emit a delegating `impl Default` that calls
    /// `<core::Enum as Default>::default()`. Without this, enums with a manual `impl
    /// Default` (no `#[default]` variant) wrongly get no wrapper Default, breaking
    /// structs that contain them.
    #[serde(default)]
    pub has_default: bool,
    /// Serde tag property name for internally tagged enums (from `#[serde(tag = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_tag: Option<String>,
    /// Serde content property name for adjacently tagged enums (from `#[serde(content = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_content: Option<String>,
    /// True when the enum has `#[serde(untagged)]`.
    /// Absence of `serde_tag` does NOT imply untagged — it means externally-tagged (the serde
    /// default). Only set this when the attribute is explicitly present on the Rust type.
    #[serde(default)]
    pub serde_untagged: bool,
    /// Serde rename strategy for enum variants (from `#[serde(rename_all = "...")]`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serde_rename_all: Option<String>,
    /// Serde rename strategy for the FIELDS of every struct-shaped variant (from
    /// `#[serde(rename_all_fields = "...")]`). This is a distinct serde namespace from
    /// `serde_rename_all`: that one cases VARIANT names, this one cases the field names inside a
    /// struct-shaped variant's payload. Setting one must never affect the other -- an enum can
    /// set either, both, or neither. A field's own `#[serde(rename = "...")]` still wins over
    /// this container rule; see `crate::codegen::naming::wire_field_name`. ~keep
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename_all_fields: Option<String>,
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
    /// `#[cfg(...)]` condition string on this variant, if any.
    ///
    /// When set, backends that generate Rust source (e.g. Dart FRB mirror enums and
    /// their `From` impls) must emit a matching `#[cfg(...)]` attribute so the binding
    /// compiles correctly under every active feature subset — including targets whose
    /// `[target_dep_overrides]` disable the feature that gates this variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<String>,
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
    /// Explicit stable C ABI error code from `#[alef(error_code = N)]`. ~keep
    #[serde(default)]
    pub error_code: Option<u32>,
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

impl ErrorVariant {
    /// Returns taxonomy metadata only when the source explicitly allocated a stable code. ~keep
    pub fn taxonomy(&self, error_type: &str) -> Option<ErrorTaxonomy> {
        self.error_code
            .map(|code| ErrorTaxonomy::for_variant(code, error_type, &self.name))
    }
}

// Compile-time witnesses guarding against a field being added to a core IR struct and
// silently ignored by every backend.
//
// THE HAZARD: backends consume IR structs (like `FieldDef`) by dot-access —
// `field.name`, `field.ty`, `field.optional` — not by destructuring. Adding a field to
// a struct like `FieldDef` therefore does not break any backend; the new field is
// simply never read. Two fields shipped exactly this way in the past and were only
// caught by a human reading generated output.
//
// Contrast that with *construction*: `FieldDef` gaining its `version` field broke
// ~300 exhaustive struct literals with E0063, forcing every construction site to be
// revisited. That is the property this module recreates for the read side.
//
// THE MECHANISM: each function below takes one IR struct by value and destructures it
// with a TOTAL pattern — every field named, no `..` rest. Total destructuring is
// compiler-checked: rustc rejects a pattern that omits a field with E0027, and rejects
// a pattern that names a field the struct no longer has with E0026. So adding (or
// removing) a field on a covered struct is a compile error at this exact spot — no
// other change anywhere in the codebase is needed for the guard to fire.
//
// The functions are never called, and don't need to be: rustc type-checks a function
// body whether or not anything invokes it, so the guard fires the moment `cargo test`
// or `cargo clippy --all-targets` type-checks this module. No dummy data, no `Default`
// requirement, no parallel copy of the IR to maintain.
//
// WHAT TO DO WHEN ONE OF THESE FAILS TO COMPILE: the compiler error names the field
// that is missing from (or no longer exists on) the pattern.
//   1. Grep `src/backends/` for how the struct's *other* fields are consumed (e.g.
//      `.optional`, `.core_wrapper`) to find the codegen sites that build the binding
//      surface for this IR item.
//   2. For each backend, decide whether the new field should change what it emits.
//      Some fields are legitimately diagnostics-only (see `binding_exclusion_reason`
//      below) — that is a considered decision, not an oversight, and must be recorded
//      as one.
//   3. Add the field to the pattern with a one-line trailing comment stating that
//      decision, following the existing entries as a template. Never add `..` back —
//      the whole point is that the *next* field addition fails here too.
//
// SCOPE — why these nine structs and nothing else:
// This covers every struct in this file (`items.rs`) that a codegen backend walks to
// build a binding surface: `TypeDef`, `FieldDef`, `MethodDef`, `FunctionDef`,
// `ParamDef`, `EnumDef`, `EnumVariant`, `ErrorDef`, `ErrorVariant`.
//
// Deliberately NOT covered:
// - `ReceiverKind` (this file) and `CoreWrapper` / `DefaultValue` (`metadata.rs`) are
//   plain enums, not structs with dot-accessed fields. Consuming an enum means
//   matching it, and an exhaustive `match` with no wildcard arm already gets this exact
//   protection from the compiler for free — a new variant is E0004 at every real match
//   site. That guarantee only breaks down if a match grows a wildcard `_` arm, which is
//   a lint concern (`clippy::wildcard_enum_match_arm`), not a silent-IR-drop concern,
//   and is out of scope here.
// - `ApiSurface` and the `service.rs` types (`ServiceDef`, `RegistrationDef`, and
//   friends) model a narrower, opt-in slice of the IR (HTTP-style service
//   registration) that only a subset of backends touch at all, unlike
//   `TypeDef` / `FieldDef` / `MethodDef`, which every backend must walk to emit
//   anything. The same witness pattern applies there unchanged; it is left as
//   follow-up work to keep this change scoped to the struct family the motivating
//   incidents actually involved.
#[cfg(test)]
mod tests {
    use super::{EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef, TypeDef};

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn type_def_field_coverage_witness(value: TypeDef) {
        let TypeDef {
            name: _,                       // identifier; every backend reads it directly
            rust_path: _,                  // import / qualified-path emission ~keep
            original_rust_path: _,         // From-impl target when core_import re-exports
            fields: _,                     // drives the whole struct-field codegen loop
            methods: _,                    // drives the whole method codegen loop
            is_opaque: _,                  // opaque-handle vs. value-type binding strategy ~keep
            is_clone: _,                   // gates `.clone()` emission ~keep
            is_copy: _,                    // gates Copy-vs-clone (avoids clippy::clone_on_copy)
            doc: _,                        // doc-comment emission
            cfg: _,                        // conditional `#[cfg]` propagation
            is_trait: _,                   // `dyn` keyword for opaque inner types
            has_default: _,                // NAPI-style all-fields-optional-with-defaults
            has_stripped_cfg_fields: _,    // `..Default::default()` in struct literals ~keep
            is_return_type: _,             // output DTO style (e.g. Python TypedDict)
            serde_rename_all: _,           // Go/Java/C# JSON tag casing
            has_serde: _,                  // gates FFI from_json/to_json generation
            serde_container_default: _,    // per-field wire-optionality (Go omitempty pointers) ~keep
            serde_container_conversion: _, // flags an unmirrorable wire shape for validation ~keep
            super_traits: _,               // trait bridge super-trait impl selection ~keep
            binding_excluded: _,           // excludes the type from generated surfaces
            binding_exclusion_reason: _,   // diagnostics only; deliberately not codegen input
            is_variant_wrapper: _,         // opts static `new` into host constructor emission
            has_lifetime_params: _,        // From<T<'_>> vs From<T> / opaque wrapper choice
            has_private_fields: _,         // non-literal construction strategy in conversions
            version: _,                    // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn field_def_field_coverage_witness(value: FieldDef) {
        let FieldDef {
            name: _,                      // identifier; every backend reads it directly
            ty: _,                        // determines the emitted binding type
            optional: _,                  // nullability across every backend
            default: _,                   // literal default expression
            doc: _,                       // doc-comment emission
            sanitized: _,                 // gates auto-generated From/Into conversions
            is_boxed: _,                  // FFI deref-before-clone on Box<T> fields
            type_rust_path: _,            // disambiguates same-named types across modules
            cfg: _,                       // conditional inclusion in struct literals
            typed_default: _,             // language-native default emission
            core_wrapper: _,              // Cow/Arc/Bytes/Box From/Into codegen
            vec_inner_core_wrapper: _,    // Vec<Wrapper<T>> element codegen
            newtype_wrapper: _,           // wrap/unwrap newtype at the binding boundary
            serde_rename: _,              // wire-name parity with core serde
            serde_flatten: _,             // language-native flatten support (e.g. Jackson)
            serde_with: _,                // suppresses derive-shape wire assumptions (e.g. Duration)
            serde_skip_serializing_if: _, // wire-optional JSON key despite a required Rust field
            serde_skip: _,                // gates getter emission for JSON-only-constructed types
            binding_excluded: _,          // drops the field from the generated surface
            binding_exclusion_reason: _,  // diagnostics only; deliberately not codegen input
            original_type: _,             // reconstructs pre-sanitize (de)serialization
            version: _,                   // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn method_def_field_coverage_witness(value: MethodDef) {
        let MethodDef {
            name: _,                     // identifier; every backend reads it directly
            params: _,                   // drives parameter-list codegen
            return_type: _,              // drives return-type codegen
            is_async: _,                 // selects sync vs. async call-site codegen
            is_static: _,                // selects associated-function vs. instance-method form
            error_type: _,               // fallible-call error-mapping codegen
            doc: _,                      // doc-comment emission
            receiver: _,                 // `&self` / `&mut self` / owned call-site codegen
            cfg: _,                      // `rust_cfg_attribute` re-emits it; `cfg_satisfied` drops it ~keep
            sanitized: _,                // methods with sanitized signatures can't auto-delegate
            trait_source: _,             // trait bridge impl selection ~keep
            returns_ref: _,              // inserts `.clone()` before type conversion ~keep
            returns_cow: _,              // inserts `.into_owned()` before type conversion ~keep
            return_newtype_wrapper: _,   // unwraps the returned newtype value
            has_default_impl: _,         // optional-impl opt-in in trait bridge codegen
            binding_excluded: _,         // excludes the method from generated surfaces
            binding_exclusion_reason: _, // diagnostics only; deliberately not codegen input
            version: _,                  // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn function_def_field_coverage_witness(value: FunctionDef) {
        let FunctionDef {
            name: _,                     // identifier; every backend reads it directly
            rust_path: _,                // qualified call-site path
            original_rust_path: _,       // From-impl target when core_import re-exports
            params: _,                   // drives parameter-list codegen
            return_type: _,              // drives return-type codegen
            is_async: _,                 // selects sync vs. async call-site codegen
            error_type: _,               // fallible-call error-mapping codegen
            doc: _,                      // doc-comment emission
            cfg: _,                      // conditional `#[cfg]` propagation
            sanitized: _,                // functions with sanitized signatures can't auto-delegate
            return_sanitized: _,         // widens the binding return type; forces JSON round-trip
            returns_ref: _,              // inserts `.clone()` before type conversion ~keep
            returns_cow: _,              // inserts `.into_owned()` before type conversion ~keep
            return_newtype_wrapper: _,   // unwraps the returned newtype value
            binding_excluded: _,         // excludes the function from generated surfaces
            binding_exclusion_reason: _, // diagnostics only; deliberately not codegen input
            version: _,                  // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn param_def_field_coverage_witness(value: ParamDef) {
        let ParamDef {
            name: _,             // identifier; every backend reads it directly
            ty: _,               // determines the emitted binding type
            optional: _,         // nullability across every backend
            default: _,          // literal default expression
            sanitized: _,        // allows reconstructing proper deserialization logic
            typed_default: _,    // language-native default emission
            is_ref: _,           // generates owned intermediates for `&T` params
            is_mut: _,           // generates `&mut` refs when calling core functions
            newtype_wrapper: _,  // wraps the raw value back into the newtype at the call site
            original_type: _,    // reconstructs pre-sanitize deserialization
            map_is_ahash: _,     // selects AHashMap vs. HashMap deserialization target
            map_key_is_cow: _,   // inserts `Cow::Owned(k)` when building the AHashMap
            vec_inner_is_ref: _, // inserts a `Vec<&T>` intermediate at the call site
            map_is_btree: _,     // converts binding HashMap into core BTreeMap at the call site
            core_wrapper: _,     // Cow/etc. conversion at the call site
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn enum_def_field_coverage_witness(value: EnumDef) {
        let EnumDef {
            name: _,                     // identifier; every backend reads it directly
            rust_path: _,                // qualified path for From/Into codegen
            original_rust_path: _,       // From-impl target when core_import re-exports
            variants: _,                 // drives the whole variant codegen loop
            methods: _,                  // extraction bookkeeping only; no backend forwards this into codegen ~keep
            doc: _,                      // doc-comment emission
            cfg: _,                      // conditional `#[cfg]` propagation
            is_copy: _,                  // gates Copy-vs-clone (avoids clippy::clone_on_copy)
            has_serde: _,                // gates Codable/serde-bridge conformance emission
            has_default: _,              // gates delegating `impl Default` emission
            serde_tag: _,                // internally-tagged enum property name
            serde_content: _,            // adjacently-tagged enum payload property name
            serde_untagged: _,           // untagged-enum handling
            serde_rename_all: _,         // variant renaming strategy
            rename_all_fields: _,        // struct-variant field renaming strategy; independent of serde_rename_all
            binding_excluded: _,         // excludes the enum from generated surfaces
            binding_exclusion_reason: _, // diagnostics only; deliberately not codegen input
            excluded_variants: _,        // retained so exhaustive Rust matches emit unreachable!()
            version: _,                  // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn enum_variant_field_coverage_witness(value: EnumVariant) {
        let EnumVariant {
            name: _,                       // identifier; every backend reads it directly
            fields: _,                     // drives struct/tuple-variant field codegen
            doc: _,                        // doc-comment emission
            is_default: _,                 // selects the `#[derive(Default)]` variant
            serde_rename: _,               // wire-name parity with core serde
            is_tuple: _,                   // selects struct- vs. tuple- vs. unit-pattern shape
            binding_excluded: _,           // excludes the variant from generated surfaces
            binding_exclusion_reason: _,   // diagnostics only; deliberately not codegen input
            originally_had_data_fields: _, // selects wildcard vs. bare-unit pattern emission
            cfg: _,                        // matching `#[cfg]` attribute emission requirement
            version: _,                    // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn error_def_field_coverage_witness(value: ErrorDef) {
        let ErrorDef {
            name: _,                     // identifier; every backend reads it directly
            rust_path: _,                // qualified path for From/Into codegen
            original_rust_path: _,       // From-impl target when core_import re-exports
            variants: _,                 // drives the whole error-variant codegen loop
            doc: _,                      // doc-comment emission
            methods: _,                  // whitelisted introspection method codegen
            binding_excluded: _,         // excludes the error type from generated surfaces
            binding_exclusion_reason: _, // diagnostics only; deliberately not codegen input
            version: _,                  // since/deprecated annotation emission
        } = value;
    }

    // See the module-level comment above for the contract this pattern enforces.
    #[allow(dead_code)]
    fn error_variant_field_coverage_witness(value: ErrorVariant) {
        let ErrorVariant {
            name: _,             // identifier; every backend reads it directly
            error_code: _,       // explicit stable C ABI error taxonomy allocation
            message_template: _, // `#[error("...")]` message template emission
            fields: _,           // drives struct/tuple field codegen
            has_source: _,       // detects `#[source]`/`#[from]` for chained-error codegen
            has_from: _,         // gates auto `From` conversion emission
            is_unit: _,          // selects the unit-variant pattern shape
            is_tuple: _,         // selects the tuple-variant wildcard pattern `(..)`
            doc: _,              // doc-comment emission
        } = value;
    }

    fn gated(cfg: Option<&str>) -> MethodDef {
        MethodDef {
            name: "stream".to_string(),
            cfg: cfg.map(str::to_string),
            ..MethodDef::default()
        }
    }

    #[test]
    fn rust_cfg_attribute_is_empty_when_ungated() {
        assert_eq!(gated(None).rust_cfg_attribute(), "");
    }

    #[test]
    fn rust_cfg_attribute_wraps_the_condition_with_a_trailing_newline() {
        assert_eq!(
            gated(Some("feature = \"streaming\"")).rust_cfg_attribute(),
            "#[cfg(feature = \"streaming\")]\n"
        );
    }

    #[test]
    fn cfg_within_and_combines_owner_and_method_gates() {
        assert_eq!(
            gated(Some("feature = \"streaming\"")).cfg_within(Some("feature = \"client\"")),
            Some("all(feature = \"client\", feature = \"streaming\")".to_string())
        );
    }

    #[test]
    fn cfg_within_collapses_an_owner_gate_the_method_already_inherited() {
        // A method in `#[cfg(feature = "x")] impl T` inherits that gate verbatim; if `T` carries
        // the same gate, emitting `all(X, X)` would be redundant churn in every generated file.
        assert_eq!(
            gated(Some("feature = \"client\"")).cfg_within(Some("feature = \"client\"")),
            Some("feature = \"client\"".to_string())
        );
    }

    #[test]
    fn cfg_within_falls_back_to_whichever_gate_is_present() {
        assert_eq!(
            gated(None).cfg_within(Some("feature = \"client\"")),
            Some("feature = \"client\"".to_string())
        );
        assert_eq!(
            gated(Some("feature = \"streaming\"")).cfg_within(None),
            Some("feature = \"streaming\"".to_string())
        );
        assert_eq!(gated(None).cfg_within(None), None);
    }

    #[test]
    fn cfg_satisfied_matches_the_shared_feature_evaluator() {
        let enabled: std::collections::HashSet<&str> = ["streaming"].into_iter().collect();
        assert!(gated(Some("feature = \"streaming\"")).cfg_satisfied(&enabled));
        assert!(!gated(Some("feature = \"tokenizer\"")).cfg_satisfied(&enabled));
        assert!(gated(None).cfg_satisfied(&enabled));
    }

    fn gated_field(cfg: Option<&str>) -> FieldDef {
        FieldDef {
            name: "sparse_embedding".to_string(),
            cfg: cfg.map(str::to_string),
            ..FieldDef::default()
        }
    }

    #[test]
    fn field_cfg_within_combines_owner_and_field_gates() {
        assert_eq!(
            gated_field(Some("feature = \"embeddings\"")).cfg_within(Some("feature = \"quality\"")),
            Some("all(feature = \"quality\", feature = \"embeddings\")".to_string())
        );
    }

    #[test]
    fn field_cfg_within_collapses_an_owner_gate_the_field_already_inherited() {
        assert_eq!(
            gated_field(Some("feature = \"quality\"")).cfg_within(Some("feature = \"quality\"")),
            Some("feature = \"quality\"".to_string())
        );
    }

    #[test]
    fn field_cfg_within_falls_back_to_whichever_gate_is_present() {
        assert_eq!(
            gated_field(None).cfg_within(Some("feature = \"quality\"")),
            Some("feature = \"quality\"".to_string())
        );
        assert_eq!(
            gated_field(Some("feature = \"embeddings\"")).cfg_within(None),
            Some("feature = \"embeddings\"".to_string())
        );
        assert_eq!(gated_field(None).cfg_within(None), None);
    }
}
