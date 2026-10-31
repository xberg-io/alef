use serde::{Deserialize, Serialize};

use super::items::{FieldDef, MethodDef};
use super::metadata::{SerdeContainerConversion, SerdeDeserializationPolicy, VersionAnnotation};

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
    /// Source deserialization rules that a language DTO must preserve.
    #[serde(default, skip_serializing_if = "SerdeDeserializationPolicy::is_empty")]
    pub serde_deserialization: SerdeDeserializationPolicy,
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
