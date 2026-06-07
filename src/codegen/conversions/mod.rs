mod binding_to_core;
mod core_to_binding;
mod enums;
pub(crate) mod helpers;

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
    /// When true, non-optional fields on defaultable types are wrapped in Option<T>
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
    /// When true, Vec<Named> fields are stored as JSON strings in the binding layer.
    /// Core→binding uses `serde_json::to_string`, binding→core uses `serde_json::from_str`.
    /// Used by Magnus (Ruby) where Vec<Named> cannot cross the FFI boundary directly and
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
    /// emits them (via [`super::generators::RustBindingConfig::never_skip_cfg_field_names`]).
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
        // &'b str: we return either the original (which has lifetime 'b from the parameter)
        // or a &str from the HashMap (which would have lifetime 'a). Since 'a: 'b we can
        // return either. But Rust's lifetime inference won't let us return `&'a str` from a
        // `&'b str` parameter without unsafe. Use a helper that returns an owned String instead.
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

// Re-export all public items so callers continue to use `conversions::foo`.
pub use binding_to_core::{
    apply_core_wrapper_to_core, field_conversion_to_core, field_conversion_to_core_cfg, gen_from_binding_to_core,
    gen_from_binding_to_core_cfg, gen_from_lifetime_type_constructor,
};
pub use core_to_binding::{
    field_conversion_from_core, field_conversion_from_core_cfg, gen_from_core_to_binding, gen_from_core_to_binding_cfg,
};
pub use enums::{
    gen_enum_from_binding_to_core, gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding,
    gen_enum_from_core_to_binding_cfg,
};
pub use helpers::{
    apply_crate_remaps, binding_to_core_match_arm, build_type_path_map, can_generate_conversion,
    can_generate_enum_conversion, can_generate_enum_conversion_from_core, convertible_types, core_enum_path,
    core_enum_path_remapped, core_to_binding_convertible_types, core_to_binding_match_arm, core_type_path,
    core_type_path_remapped, field_references_excluded_type, has_sanitized_fields, input_type_names, is_tuple_variant,
    resolve_named_path,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::*;

    fn simple_type() -> TypeDef {
        TypeDef {
            name: "Config".to_string(),
            rust_path: "my_crate::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "name".into(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "timeout".into(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "backend".into(),
                    ty: TypeRef::Named("Backend".into()),
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn simple_enum() -> EnumDef {
        EnumDef {
            name: "Backend".to_string(),
            rust_path: "my_crate::Backend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Cpu".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Gpu".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }
    }

    #[test]
    fn test_from_binding_to_core() {
        let typ = simple_type();
        let result = gen_from_binding_to_core(&typ, "my_crate");
        assert!(result.contains("impl From<Config> for my_crate::Config"));
        assert!(result.contains("name: val.name"));
        assert!(result.contains("timeout: val.timeout"));
        assert!(result.contains("backend: val.backend.map(Into::into)"));
    }

    #[test]
    fn test_from_core_to_binding() {
        let typ = simple_type();
        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(result.contains("impl From<my_crate::Config> for Config"));
    }

    #[test]
    fn test_enum_from_binding_to_core() {
        let enum_def = simple_enum();
        let result = gen_enum_from_binding_to_core(&enum_def, "my_crate");
        assert!(result.contains("impl From<Backend> for my_crate::Backend"));
        assert!(result.contains("Backend::Cpu => Self::Cpu"));
        assert!(result.contains("Backend::Gpu => Self::Gpu"));
    }

    #[test]
    fn test_enum_from_core_to_binding() {
        let enum_def = simple_enum();
        let result = gen_enum_from_core_to_binding(&enum_def, "my_crate");
        assert!(result.contains("impl From<my_crate::Backend> for Backend"));
        assert!(result.contains("my_crate::Backend::Cpu => Self::Cpu"));
        assert!(result.contains("my_crate::Backend::Gpu => Self::Gpu"));
    }

    #[test]
    fn test_enum_from_core_to_binding_no_excluded_variants_no_catchall() {
        let enum_def = simple_enum();
        let result = gen_enum_from_core_to_binding(&enum_def, "my_crate");
        assert!(
            !result.contains("_ => Default::default()"),
            "catch-all arm should not be emitted when there are no excluded variants"
        );
    }

    #[test]
    fn test_enum_from_binding_to_core_no_excluded_variants_no_catchall() {
        let enum_def = simple_enum();
        let result = gen_enum_from_binding_to_core(&enum_def, "my_crate");
        assert!(
            !result.contains("_ => Default::default()"),
            "catch-all arm should not be emitted when there are no excluded variants"
        );
    }

    #[test]
    fn test_enum_from_core_to_binding_with_excluded_variants_has_catchall() {
        let mut enum_def = simple_enum();
        enum_def.excluded_variants.push(EnumVariant {
            name: "Tpu".into(),
            fields: vec![],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
        });
        let result = gen_enum_from_core_to_binding(&enum_def, "my_crate");
        assert!(
            result.contains("_ => Default::default()"),
            "catch-all arm should be emitted when there are excluded variants"
        );
    }

    #[test]
    fn test_enum_from_binding_to_core_with_excluded_variants_no_catchall() {
        // From<BindingEnum> for CoreEnum matches on the *binding* type, which never
        // contains excluded variants — the match is always exhaustive. A wildcard
        // arm would be unreachable and must not be emitted even when the core enum
        // has excluded (binding-skipped) variants.
        let mut enum_def = simple_enum();
        enum_def.excluded_variants.push(EnumVariant {
            name: "Tpu".into(),
            fields: vec![],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
        });
        let result = gen_enum_from_binding_to_core(&enum_def, "my_crate");
        assert!(
            !result.contains("_ => Default::default()"),
            "catch-all arm must not be emitted for From<BindingEnum>→core; the binding match is always exhaustive"
        );
    }

    #[test]
    fn test_enum_from_core_to_binding_unit_only_with_struct_variants_no_catchall() {
        // Regression: when the binding is unit-only (binding_enums_have_data=false) but the
        // core enum has named-field (struct) variants, every variant still gets its own
        // explicit arm (`CoreT::V { .. } => Self::V,`).  The match is exhaustive; emitting
        // `_ => Default::default()` produces an "unreachable pattern" error under -D warnings.
        let mut enum_def = simple_enum();
        // Add a named-field (struct) variant to simulate e.g. WebSocketMessage::Close { code, reason }.
        enum_def.variants.push(EnumVariant {
            name: "Disconnect".into(),
            fields: vec![FieldDef {
                name: "code".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
        });
        // Unit-only binding (default config has binding_enums_have_data=false).
        let result = gen_enum_from_core_to_binding(&enum_def, "my_crate");
        assert!(
            !result.contains("_ => Default::default()"),
            "catch-all must not be emitted when all core variants are covered by explicit arms; got:\n{result}"
        );
        // The struct variant must still get its own arm (not silently dropped).
        assert!(
            result.contains("Backend::Disconnect { .. } => Self::Disconnect"),
            "struct variant must have an explicit arm; got:\n{result}"
        );
    }

    fn untagged_tuple_enum() -> EnumDef {
        EnumDef {
            name: "UserContent".to_string(),
            rust_path: "my_crate::UserContent".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: CoreWrapper::None,
                        vec_inner_core_wrapper: CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Parts".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Vec(Box::new(TypeRef::String)),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: CoreWrapper::None,
                        vec_inner_core_wrapper: CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: None,
            serde_untagged: true,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }
    }

    #[test]
    fn test_enum_from_binding_to_core_untagged_tuple_emits_tuple_pattern() {
        // Regression: untagged enums with tuple variants emit tuple-form `Variant(T)` in
        // the binding (Magnus template since commit a715f378). Conversion match arms must
        // destructure tuple-form, not struct-form `Variant { _0 }`.
        let enum_def = untagged_tuple_enum();
        let config = ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_untagged_variants: true,
            ..ConversionConfig::default()
        };
        let result = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
        // MUST destructure as tuple, not struct
        assert!(
            result.contains("UserContent::Text(_0)"),
            "expected tuple-form binding pattern, got: {result}"
        );
        assert!(
            !result.contains("UserContent::Text { _0 }"),
            "must NOT use struct-form for untagged enums, got: {result}"
        );
        // Construct core as tuple
        assert!(result.contains("Self::Text("));
    }

    #[test]
    fn test_enum_from_core_to_binding_untagged_tuple_emits_tuple_constructor() {
        // Regression: untagged enums with tuple variants emit tuple-form `Variant(T)` in
        // the binding. Constructor must use tuple form, not `Self::Variant { _0 }`.
        let enum_def = untagged_tuple_enum();
        let config = ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_untagged_variants: true,
            ..ConversionConfig::default()
        };
        let result = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
        // Core destructured as tuple (already correct), binding constructed as tuple
        assert!(
            result.contains("my_crate::UserContent::Text(_0) => Self::Text("),
            "expected tuple-form binding constructor, got: {result}"
        );
        assert!(
            !result.contains("Self::Text { _0 }"),
            "must NOT use struct-form constructor for untagged enums, got: {result}"
        );
    }

    #[test]
    fn test_enum_tagged_data_keeps_struct_form_pattern() {
        // Counter-regression: tagged (non-untagged) data enums must keep struct-form
        // `Variant { _0 }` pattern/constructor — only untagged enums switch to tuple form.
        let mut enum_def = untagged_tuple_enum();
        enum_def.serde_untagged = false;
        enum_def.serde_tag = Some("type".to_string());
        let config = ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_untagged_variants: true,
            ..ConversionConfig::default()
        };
        let result = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
        assert!(
            result.contains("UserContent::Text { _0 }"),
            "tagged enums must keep struct-form, got: {result}"
        );
    }

    #[test]
    fn test_enum_untagged_keeps_struct_form_when_backend_does_not_opt_in() {
        // Counter-regression for the Rustler backend: untagged enums must remain in
        // struct-form when the backend's enum body emitter does not switch to tuple
        // form (every backend except Magnus). `binding_tuple_form_for_untagged_variants`
        // is the opt-in flag.
        let enum_def = untagged_tuple_enum();
        let config = ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_untagged_variants: false,
            ..ConversionConfig::default()
        };
        let result = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
        assert!(
            result.contains("UserContent::Text { _0 }"),
            "backends without the opt-in must keep struct-form, got: {result}"
        );
        let result2 = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
        assert!(
            result2.contains("Self::Text { _0:"),
            "backends without the opt-in must construct struct-form, got: {result2}"
        );
    }

    #[test]
    fn test_from_binding_to_core_with_cfg_gated_field() {
        // Create a type with a cfg-gated field
        let mut typ = simple_type();
        typ.has_stripped_cfg_fields = true;
        typ.fields.push(FieldDef {
            name: "layout".into(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: Some("feature = \"layout-detection\"".into()),
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        });

        let result = gen_from_binding_to_core(&typ, "my_crate");

        // The impl should exist
        assert!(result.contains("impl From<Config> for my_crate::Config"));
        // Regular fields should be present
        assert!(result.contains("name: val.name"));
        assert!(result.contains("timeout: val.timeout"));
        // Cfg-gated fields are now preserved on the binding struct, so the conversion
        // accesses them directly rather than padding with ..Default::default().
        assert!(result.contains("layout: val.layout"));
    }

    #[test]
    fn test_from_core_to_binding_with_cfg_gated_field() {
        // Create a type with a cfg-gated field
        let mut typ = simple_type();
        typ.fields.push(FieldDef {
            name: "layout".into(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: Some("feature = \"layout-detection\"".into()),
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        });

        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());

        // The impl should exist
        assert!(result.contains("impl From<my_crate::Config> for Config"));
        // Regular fields should be present
        assert!(result.contains("name: val.name"));
        // Cfg-gated fields are now preserved on the binding struct and round-tripped.
        assert!(result.contains("layout: val.layout"));
    }

    #[test]
    fn test_field_conversion_from_core_map_named_non_optional() {
        // Map<K, Named> non-optional: each value needs .into() core→binding
        let result = field_conversion_from_core(
            "tags",
            &TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Tag".into()))),
            false,
            false,
            &AHashSet::new(),
        );
        assert_eq!(
            result,
            "tags: val.tags.into_iter().map(|(k, v)| (k, v.into())).collect()"
        );
    }

    #[test]
    fn test_field_conversion_from_core_option_map_named() {
        // Option<Map<K, Named>>: .map() wrapper + per-element .into()
        let result = field_conversion_from_core(
            "tags",
            &TypeRef::Optional(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Tag".into())),
            ))),
            false,
            false,
            &AHashSet::new(),
        );
        assert_eq!(
            result,
            "tags: val.tags.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())"
        );
    }

    #[test]
    fn test_field_conversion_from_core_vec_named_non_optional() {
        // Vec<Named> non-optional: each element needs .into() core→binding
        let result = field_conversion_from_core(
            "items",
            &TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
            false,
            false,
            &AHashSet::new(),
        );
        assert_eq!(result, "items: val.items.into_iter().map(Into::into).collect()");
    }

    #[test]
    fn test_field_conversion_from_core_option_vec_named() {
        // Option<Vec<Named>>: .map() wrapper + per-element .into()
        let result = field_conversion_from_core(
            "items",
            &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))))),
            false,
            false,
            &AHashSet::new(),
        );
        assert_eq!(
            result,
            "items: val.items.map(|v| v.into_iter().map(Into::into).collect())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_option_map_named_applies_per_value_into() {
        // Bug A1 regression: Option<Map<K, Named>> must apply per-value .into() so that
        // binding-side wrapper types (e.g. PyO3 / Magnus structs) are converted correctly.
        let result = field_conversion_to_core(
            "patterns",
            &TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("ExtractionPattern".into())),
            ),
            true,
        );
        assert!(
            result.contains("m.into_iter().map(|(k, v)| (k.into(), v.into())).collect()"),
            "expected per-value v.into() in optional Map<Named> conversion, got: {result}"
        );
        assert_eq!(
            result,
            "patterns: val.patterns.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
        );
    }

    #[test]
    fn test_optionalized_defaultable_struct_uses_core_default_as_base() {
        let mut typ = simple_type();
        typ.has_default = true;
        typ.fields = vec![
            FieldDef {
                name: "language".into(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::Cow,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "structure".into(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ];
        let config = ConversionConfig {
            type_name_prefix: "Js",
            optionalize_defaults: true,
            ..ConversionConfig::default()
        };

        let result = gen_from_binding_to_core_cfg(&typ, "my_crate", &config);

        assert!(result.contains("let mut __result = my_crate::Config::default();"));
        assert!(result.contains("if let Some(__v) = val.language { __result.language = __v.into(); }"));
        assert!(result.contains("if let Some(__v) = val.structure { __result.structure = __v; }"));
        assert!(!result.contains("unwrap_or_default()"));
    }

    fn arc_field_type(field: FieldDef) -> TypeDef {
        TypeDef {
            name: "State".to_string(),
            rust_path: "my_crate::State".to_string(),
            original_rust_path: String::new(),
            fields: vec![field],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn arc_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::Arc,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    /// Regression: Option<Arc<serde_json::Value>> must not chain `(*v).clone().into()`
    /// on top of `as_ref().map(ToString::to_string)`, which would emit invalid
    /// `(*String).clone()` (str: !Clone).
    #[test]
    fn test_arc_json_option_field_no_double_chain() {
        let typ = arc_field_type(arc_field("registered_spec", TypeRef::Json, true));
        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            result.contains("val.registered_spec.as_ref().map(ToString::to_string)"),
            "expected as_ref().map(ToString::to_string) for Option<Arc<Value>>, got: {result}"
        );
        assert!(
            !result.contains("map(ToString::to_string).map("),
            "must not chain a second map() on top of ToString::to_string, got: {result}"
        );
    }

    /// Non-optional Arc<Value>: `(*val.X).clone().to_string()` is valid (Value: Clone).
    #[test]
    fn test_arc_json_non_optional_field() {
        let typ = arc_field_type(arc_field("spec", TypeRef::Json, false));
        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            result.contains("(*val.spec).clone().to_string()"),
            "expected (*val.spec).clone().to_string() for Arc<Value>, got: {result}"
        );
    }

    /// Option<Arc<String>>: the base string conversion already handles Arc via Deref/Display.
    /// Verifies the Arc wrapper does not append a second map over the converted String.
    #[test]
    fn test_arc_string_option_field_passthrough() {
        let typ = arc_field_type(arc_field("label", TypeRef::String, true));
        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            result.contains("val.label.map(|v| v.to_string())"),
            "expected single .map(|v| v.to_string()) for Option<Arc<String>>, got: {result}"
        );
        assert!(
            !result.contains("map(|v| v.to_string()).map("),
            "must not chain a second map() after string conversion, got: {result}"
        );
    }

    /// Regression: `Arc<HashMap<String, String>>` field — synthetic shape representative
    /// of structs that share an immutable map via Arc for zero-copy FFI. The plain `Arc`
    /// CoreWrapper must transparently unwrap the inner `val.<name>` reference via
    /// `(*val.<name>).clone()` so the downstream map iteration sees the owned `HashMap`,
    /// and the binding side reconstructs an `Arc` around the binding-shaped map.
    #[test]
    fn test_arc_hashmap_string_string_field_transparent() {
        let field = arc_field(
            "headers",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            false,
        );
        let typ = arc_field_type(field);
        let to_binding = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            to_binding.contains("(*val.headers).clone()"),
            "expected (*val.headers).clone() deref-clone for Arc<HashMap<...>>, got: {to_binding}"
        );
        let to_core = gen_from_binding_to_core(&typ, "my_crate");
        assert!(
            to_core.contains("headers:"),
            "expected headers field in binding→core conversion, got: {to_core}"
        );
    }

    /// Regression: `Arc<Vec<String>>` field — plain Arc unwraps via deref-clone on the
    /// non-optional branch, just like the HashMap shape.
    #[test]
    fn test_arc_vec_string_field_transparent() {
        let field = arc_field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false);
        let typ = arc_field_type(field);
        let to_binding = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            to_binding.contains("(*val.tags).clone()"),
            "expected (*val.tags).clone() deref-clone for Arc<Vec<...>>, got: {to_binding}"
        );
        let to_core = gen_from_binding_to_core(&typ, "my_crate");
        assert!(
            to_core.contains("tags:"),
            "expected tags field in binding→core conversion, got: {to_core}"
        );
    }

    /// Regression: `Arc<Mutex<String>>` field — the `ArcMutex` CoreWrapper drives
    /// codegen to emit `.lock().unwrap().clone()` on the read path (core→binding) and
    /// `Arc::new(Mutex::new(...))` on the write path (binding→core). Verifies the
    /// ArcMutex branch is wired end-to-end.
    #[test]
    fn test_arc_mutex_string_field_transparent() {
        let mut field = arc_field("state", TypeRef::String, false);
        field.core_wrapper = CoreWrapper::ArcMutex;
        let typ = arc_field_type(field);
        let to_binding = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());
        assert!(
            to_binding.contains("val.state.lock().unwrap().clone().into()"),
            "expected .lock().unwrap().clone().into() for Arc<Mutex<String>>, got: {to_binding}"
        );
        let to_core = gen_from_binding_to_core(&typ, "my_crate");
        assert!(
            to_core.contains("std::sync::Arc::new(std::sync::Mutex::new(val.state.into()))"),
            "expected Arc::new(Mutex::new(...)) construction for Arc<Mutex<String>>, got: {to_core}"
        );
    }
}
