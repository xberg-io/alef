use ahash::{AHashMap, AHashSet};

/// Backend-specific configuration for From/field conversion generation.
/// Enables shared code to handle all backend differences via parameters.
#[derive(Default, Clone)]
pub struct ConversionConfig<'a> {
    /// Prefix for binding type names ("Js" for NAPI/WASM, "" for others).
    pub type_name_prefix: &'a str,
    /// U64/Usize/Isize need `as i64` casts (NAPI, PHP — JS/PHP lack native u64).
    pub cast_large_ints_to_i64: bool,
    /// Enum names mapped to String in the binding layer (PHP only).
    /// Named fields referencing these use `format!("{:?}")` in core→binding.
    pub enum_string_names: Option<&'a AHashSet<String>>,
    /// A known fieldless (unit) variant name for entries in `enum_string_names` that have
    /// one, keyed by enum type name (PHP only).
    ///
    /// A PHP-facing `String` field can hold any value a script assigns before the
    /// binding→core conversion re-parses it into the real enum. On a bad value, the
    /// non-optional binding→core conversion needs *some* valid `Self` to return -- `From`
    /// cannot bail out early -- so it falls back to this variant (safe to reference by
    /// bare name precisely because it is fieldless) after reporting the real failure to
    /// PHP as a catchable exception. Absent for an enum with no fieldless variant. ~keep
    pub enum_string_fallback_variant: Option<&'a AHashMap<String, String>>,
    /// Map types use JsValue in the binding layer (WASM only).
    /// When true, Map fields use `serde_wasm_bindgen` for conversion instead of
    /// iterator-based collect patterns (JsValue is not iterable).
    pub map_uses_jsvalue: bool,
    /// When true, f32 is mapped to f64 (NAPI only — JS has no f32).
    pub cast_f32_to_f64: bool,
    /// When true, non-optional fields on defaultable types are wrapped in `Option<T>`
    /// in the binding struct and need `.unwrap_or_default()` in binding→core From.
    /// Used by NAPI to make JS-facing structs fully optional.
    pub optionalize_defaults: bool,
    /// When true, Json (serde_json::Value) fields are mapped to String in the binding layer.
    /// Core→binding uses `.to_string()`, binding→core uses `Default::default()` (lossy).
    /// Used by PHP where serde_json::Value can't cross the extension boundary.
    pub json_to_string: bool,
    /// When true, Json fields stay as `serde_json::Value` in the binding layer (no wrapping).
    /// Core↔binding conversions are identity since both sides hold the same type.
    /// Used by NAPI (with `serde-json` feature) so JS callers can pass arbitrary objects
    /// directly without first stringifying them.
    pub json_as_value: bool,
    /// When true, add synthetic metadata field conversion for ConversionResult.
    /// Only NAPI backend sets this (it adds metadata field to the struct).
    pub include_cfg_metadata: bool,
    /// When true, non-optional Duration fields on `has_default` types are stored as
    /// `Option<u64>` in the binding struct.  The From conversion uses the builder
    /// pattern so that `None` falls back to the core type's `Default` implementation
    /// (giving the real default, e.g. `Duration::from_secs(30)`) instead of `Duration::ZERO`.
    /// Used by PyO3 to prevent validation failures when `request_timeout` is unset.
    pub option_duration_on_defaults: bool,
    /// When true, binding enums include data variant fields (Magnus).
    /// When false (default), binding enums are unit-only and data is lost in conversion.
    pub binding_enums_have_data: bool,
    /// Type names excluded from the binding layer. Fields referencing these types
    /// are skipped in the binding struct and defaulted in From conversions.
    /// Used by WASM to handle types excluded due to native dependency requirements.
    pub exclude_types: &'a [String],
    /// When true, `Vec<Named>` fields are stored as JSON strings in the binding layer.
    /// Core→binding uses `serde_json::to_string`, binding→core uses `serde_json::from_str`.
    /// Used by Magnus (Ruby) where `Vec<Named>` cannot cross the FFI boundary directly and
    /// is collapsed to String by `field_type_for_serde`'s catch-all arm.
    pub vec_named_to_string: bool,
    /// When true, all Map(K, V) fields are stored as a plain `String` in the binding layer.
    /// Core→binding uses `format!("{:?}", val.field)`, binding→core uses `Default::default()` (lossy).
    /// Used by Rustler (Elixir NIFs) where `HashMap` cannot cross the NIF boundary directly.
    pub map_as_string: bool,
    /// When true, `Map(K, V)` fields are stored as JSON strings in the binding layer.
    /// Core→binding uses `serde_json::to_string`, binding→core uses `serde_json::from_str`
    /// (a lossless round trip, unlike `map_as_string`'s lossy `Debug`/`Default` handling).
    /// Used by Magnus (Ruby) enum data-variant fields: `Map` fields on enum variants cannot
    /// cross the FFI boundary directly and are collapsed to `String` by the catch-all arm of
    /// `backends::magnus::gen_bindings::classes::gen_enum::field_type_for_serde_inner`. Regular
    /// struct `Map` fields are unaffected — Magnus keeps those as native `HashMap`, so this flag
    /// must only be set on the `ConversionConfig` used for enum conversions, not struct ones.
    pub map_flatten_to_string: bool,
    /// Set of opaque type names in the binding layer.
    /// When a field has `CoreWrapper::Arc` and its type is an opaque Named type,
    /// the binding wrapper holds `inner: Arc<CoreT>` and the conversion must extract
    /// `.inner` directly instead of calling `.into()` + wrapping in `Arc::new`.
    pub opaque_types: Option<&'a AHashSet<String>>,
    /// Type names that should use `Default::default()` in the binding→core From impl.
    /// Used by PHP to skip bridge type fields (e.g., VisitorHandle) that can't be
    /// auto-converted via Into and are always handled by the bridge machinery instead.
    pub from_binding_skip_types: &'a [String],
    /// When `core_crate_override` is set for a language, the IR's `rust_path` values
    /// still contain the original source crate prefix (e.g. `mylib_core::Method`).
    /// This field remaps those paths: `(original_crate_name, override_crate_name)`.
    /// When set, any `rust_path` whose leading crate segment equals `original_crate_name`
    /// is rewritten to use `override_crate_name` instead.
    /// Example: `Some(("mylib_core", "mylib_http"))` rewrites
    /// `mylib_core::Method` → `mylib_http::Method`.
    pub source_crate_remaps: &'a [(&'a str, &'a str)],
    /// Per-field binding name overrides.  Key is `"TypeName.field_name"` (using the original
    /// IR field name); value is the binding struct's actual Rust field name (e.g. `"class_"`).
    /// Used when a field name is a reserved keyword in the target language and must be escaped
    /// in the binding struct (e.g. `class` → `class_`).
    ///
    /// When present, `val.<binding_name>` is used for binding-side access and the original
    /// `field_name` is used for core-side access (struct literal and assignment targets).
    pub binding_field_renames: Option<&'a std::collections::HashMap<String, String>>,
    /// When true, U8/U16/U32 (and their signed counterparts I8/I16) need `as i32` casts.
    /// extendr maps all small integers to R's native integer type (i32), so binding→core
    /// conversions must cast back to the original unsigned/narrow types.
    pub cast_uints_to_i32: bool,
    /// When true, U64/Usize/Isize are mapped to f64 (R's native double type) rather than i64.
    /// extendr uses f64 for large integers because R has no native 64-bit integer type.
    /// Binding→core: `as usize`/`as u64` casts; core→binding: `as f64` casts.
    pub cast_large_ints_to_f64: bool,
    /// Names of untagged data enums (`#[serde(untagged)]` with at least one data variant —
    /// e.g. `Single(String) | Multiple(Vec<String>)`). Fields referencing these types are
    /// stored as `serde_json::Value` in the binding struct (the wire JSON shape varies per
    /// variant, so we accept any value at the boundary). Used by the PHP backend; ext-php-rs
    /// has no `FromZval`/`IntoZval` for typed Rust enums with mixed-shape variants, and the
    /// only safe wire format is JSON-via-Value. Conversions:
    ///
    ///   - core→binding: `serde_json::to_value(val.<name>).unwrap_or_default()`
    ///   - binding→core: `serde_json::from_value(val.<name>).unwrap_or_default()`
    pub untagged_data_enum_names: Option<&'a AHashSet<String>>,
    /// Names of content-union types opted into a display-text binding representation (via the
    /// crate-level `untagged_union_text_types` config). Fields referencing these types are stored
    /// as `String` (the display text) in the binding struct, mirroring the core type's `Display`
    /// impl. Used by the WASM backend so `message.content` returns the assistant text directly
    /// instead of an opaque discriminant. Conversions:
    ///
    ///   - core→binding: `val.<name>.to_string()` (or `.as_ref().map(|v| v.to_string())`)
    ///   - binding→core: `serde_json::from_value(serde_json::Value::String(val.<name>))`
    ///     (an untagged content union deserialises a JSON string into its text variant)
    pub text_field_enum_names: Option<&'a AHashSet<String>>,
    /// Names of tagged-data enums (`#[serde(tag = "...")]` with at least one data variant).
    /// Fields referencing these types (or `Vec` of these types) are stored as `JsValue` in the
    /// wasm binding struct so that plain JS objects `{ role: "user", content: "..." }` can be
    /// passed without being wrapped in an explicit binding-class instance.
    ///
    /// Used by the WASM backend only; `map_uses_jsvalue` must also be `true`.
    ///
    /// Conversions:
    ///   - core→binding: `serde_wasm_bindgen::to_value(&val.<name>).unwrap_or(JsValue::NULL)`
    ///   - binding→core: `serde_wasm_bindgen::from_value(val.<name>.clone()).unwrap_or_default()`
    pub tagged_data_enum_names: Option<&'a AHashSet<String>>,
    /// Names of cfg-gated fields that must NOT be skipped in conversions because the binding
    /// emits them (via `RustBindingConfig::never_skip_cfg_field_names`).
    /// Empty by default; backends populate from trait-bridge `bind_via = "options_field"` config.
    pub never_skip_cfg_field_names: &'a [String],
    /// Names of trait-bridge OptionsField fields whose binding wrapper holds the core value
    /// as `inner: Arc<core::T>` (the standard codegen layout for every OptionsField bridge).
    /// When a field matches both `is_opaque_no_wrapper_field` and this list, the binding→core
    /// From impl emits `(*v.inner).clone()` instead of `Default::default()`, so the visitor
    /// (or other bridge handle) is forwarded rather than silently dropped.
    pub trait_bridge_arc_wrapper_field_names: &'a [String],
    /// When true, cfg-gated fields (not listed in `never_skip_cfg_field_names`) are
    /// stripped from the binding struct entirely (no field at all in the struct body).
    /// Conversions must then skip those fields and rely on `..Default::default()` in
    /// the template to fill the core struct slot.
    ///
    /// Set to `true` for backends whose binding crate does not carry feature gates into
    /// its own Cargo.toml — e.g. extendr (R), where the binding struct is uniform across
    /// all feature combinations.  PyO3/NAPI/PHP/etc keep cfg-gated fields in the binding
    /// struct (decorated with `#[cfg(...)]`) and want them included in conversions.
    pub strip_cfg_fields_from_binding_struct: bool,
    /// When true, representation-eligible tuple variants in the binding use Rust tuple-form
    /// `Variant(T)` instead of struct-form `Variant { _0: T }`. The conversion match
    /// arms must destructure / construct in the same shape, otherwise rustc rejects
    /// the From impls with E0559 / E0769.
    /// Set true only for backends whose enum body emitter follows the shared serde
    /// representation predicate — currently Magnus (Ruby). Other data-bearing backends
    /// (Rustler, NAPI, PyO3, …) keep struct form, so this flag must stay false.
    pub binding_tuple_form_for_variants: bool,
    /// This binding's own configured feature set, already expanded (and unioned with the core
    /// crate's own declared defaults where active) via
    /// `codegen::cfg::enabled_features_for_language(config, lang)`.
    ///
    /// Used only to decide whether a FOREIGN-owned cfg-gated enum variant (one whose defining
    /// crate is not this binding's core crate, so this crate cannot declare the dependency's own
    /// feature name as its own Cargo feature) is provably unreachable for this binding -- see
    /// `enum_variant_declaration` and `enum_conversion_needs_catch_all`'s callers in this module.
    /// `None` means the backend has not wired this up, which keeps the existing conservative
    /// behavior (assume a foreign cfg-gated variant might still exist) unchanged -- only a
    /// backend that explicitly passes `Some` gets the more precise treatment. ~keep
    pub configured_features: Option<&'a [String]>,
    /// Whether this backend's binding-wrapper enum DECLARATION (the `#[pyclass] enum Foo`,
    /// `#[napi(string_enum)] enum Js...`, the extendr/rustler/dart mirror enum, etc -- emitted by
    /// a `gen_enum`-style function outside `codegen::conversions`, not by this module) itself
    /// drops a FOREIGN cfg-gated variant when `configured_features` proves it unreachable, the
    /// same verdict `enum_variant_declaration` computes.
    ///
    /// This flag only changes [`gen_enum_from_binding_to_core_cfg`]'s catch-all decision. That
    /// conversion's match is over the BINDING type this backend itself declares, so whether the
    /// match stays exhaustive without a catch-all depends on what that declaration actually
    /// emitted -- not on whether `configured_features` can merely prove the dependency's own
    /// variant unreachable. [`gen_enum_from_core_to_binding_cfg`] is unaffected by this flag: its
    /// match is over the real CORE type, a shape alef does not declare and cannot influence, so
    /// `configured_features`' proof about that dependency is already the complete, correct answer
    /// there regardless of what any binding declaration does.
    ///
    /// NAPI (`backends::napi::gen_bindings::enums::gen_enum`), PyO3's fieldless-enum declaration,
    /// and Magnus's own declaration all call `enum_variant_declaration` and therefore genuinely
    /// drop the variant -- as do Rustler's and Dart's mirror declarations, fixed the same way but
    /// outside this module's `ConversionConfig` flow entirely. PyO3's data-enum bodies (a struct
    /// wrapper with no per-variant declaration to drop) and wasm's foreign-variant branch of
    /// `enum_variant_declaration_without_cfg_attribute` still keep a foreign cfg-gated variant
    /// unconditionally, ignoring `configured_features` entirely, so for them the binding_to_core
    /// catch-all is required whenever such a variant exists no matter what `configured_features`
    /// proves. Defaults to `false` (the safe direction: emit the catch-all) -- set it `true` only
    /// when the paired declaration this `ConversionConfig` feeds has actually been taught to
    /// drop, as pyo3's and magnus's own construction sites now do. ~keep
    pub declaration_drops_unreachable_foreign_variants: bool,
    /// This binding's own declared Cargo feature names, used to narrow a cfg-gated field's
    /// `#[cfg(...)]` gate (see `restrict_field_gate`) before it is copied onto a conversion's
    /// field reference -- see `codegen::cfg::restrict_cfg_gate_to_declared` for why a verbatim
    /// copy can name a feature this binding crate's own `[features]` table never declares.
    ///
    /// `None` for every backend that has not wired this up, which preserves the exact prior
    /// behavior of copying `field.cfg` verbatim -- only a backend that explicitly passes `Some`
    /// (currently PHP) gets the narrowed/dropped treatment. Deliberately a *separate* field from
    /// `configured_features` above: that one drives foreign-variant reachability inside
    /// `codegen::conversions::enums`, and reusing it here would also switch on that unrelated
    /// behavior for any backend wired for this fix, which is a bigger change than the cfg-gate
    /// fix calls for. ~keep
    pub declared_features: Option<&'a std::collections::HashSet<&'a str>>,
}

impl<'a> ConversionConfig<'a> {
    /// Look up the binding struct field name for a given type and IR field name.
    ///
    /// Returns the escaped name (e.g. `"class_"`) when the field was renamed due to a
    /// reserved keyword conflict, or the original `field_name` when no rename applies.
    pub fn binding_field_name<'b>(&self, type_name: &str, field_name: &'b str) -> &'b str
    where
        'a: 'b,
    {
        let _ = type_name;
        field_name
    }

    /// Returns `true` when `field_name` is a trait-bridge OptionsField whose binding wrapper
    /// stores the core value as `inner: Arc<core::T>`. Used by `gen_from_binding_to_core_cfg`
    /// to emit `(*v.inner).clone()` instead of `Default::default()` for opaque-no-wrapper fields.
    pub fn trait_bridge_field_is_arc_wrapper(&self, field_name: &str) -> bool {
        self.trait_bridge_arc_wrapper_field_names
            .iter()
            .any(|n| n == field_name)
    }

    /// Like `binding_field_name` but returns an owned `String`, suitable for use in
    /// format strings and string interpolation.
    pub fn binding_field_name_owned(&self, type_name: &str, field_name: &str) -> String {
        if let Some(map) = self.binding_field_renames {
            let key = format!("{type_name}.{field_name}");
            if let Some(renamed) = map.get(&key) {
                return renamed.clone();
            }
        }
        field_name.to_string()
    }

    /// Narrow a cfg-gated field's `#[cfg(...)]` gate to only the feature names this binding
    /// crate declares (`declared_features`), before it is copied onto a conversion's field
    /// reference. See `codegen::cfg::restrict_cfg_gate_to_declared` for the term-by-term
    /// semantics.
    ///
    /// A gate reaching this call site was already proven satisfiable by this same feature set
    /// upstream -- the caller that decided to keep this field at all
    /// (`never_skip_cfg_field_names`) used `cfg_feature_satisfied` against the identical set a
    /// consistent caller passes here. So `restrict_cfg_gate_to_declared` returning
    /// `Unreachable` at this call site means the caller populated
    /// `never_skip_cfg_field_names` and `declared_features` from two different feature sets --
    /// a logic error, not a real reachable state for a consistent caller. Neither generated
    /// `From` impl has a template-safe way to drop the field reference this guards in every
    /// code path (`core_to_binding_impl` has no `..Default::default()` fallback at all), so the
    /// conservative choice on that unreachable branch is to fall back to the gate unrestricted
    /// rather than emit a struct literal missing a field. ~keep
    pub(crate) fn restrict_field_gate<'g>(&self, gate: &'g str) -> std::borrow::Cow<'g, str> {
        match self.declared_features {
            Some(declared) => match crate::codegen::cfg::restrict_cfg_gate_to_declared(gate, declared) {
                crate::codegen::cfg::DeclaredCfgGate::Gate(narrowed) => std::borrow::Cow::Owned(narrowed),
                crate::codegen::cfg::DeclaredCfgGate::Unreachable => std::borrow::Cow::Borrowed(gate),
            },
            None => std::borrow::Cow::Borrowed(gate),
        }
    }
}
