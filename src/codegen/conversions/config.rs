use ahash::AHashSet;

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
    /// When true, untagged-enum tuple variants in the binding use Rust tuple-form
    /// `Variant(T)` instead of struct-form `Variant { _0: T }`. The conversion match
    /// arms must destructure / construct in the same shape, otherwise rustc rejects
    /// the From impls with E0559 / E0769.
    /// Set true ONLY for backends whose enum body emitter switches to tuple form for
    /// `serde_untagged && variant.is_tuple` — currently just Magnus (Ruby) since
    /// commit a715f378. Other data-bearing backends (Rustler, NAPI, PyO3, …) keep
    /// struct-form even for untagged enums and so this flag must stay false.
    pub binding_tuple_form_for_untagged_variants: bool,
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
}
