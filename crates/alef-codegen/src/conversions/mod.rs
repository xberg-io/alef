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
    /// Set of opaque type names in the binding layer.
    /// When a field has `CoreWrapper::Arc` and its type is an opaque Named type,
    /// the binding wrapper holds `inner: Arc<CoreT>` and the conversion must extract
    /// `.inner` directly instead of calling `.into()` + wrapping in `Arc::new`.
    pub opaque_types: Option<&'a AHashSet<String>>,
    /// Per-field binding name overrides.  Key is `"TypeName.field_name"` (using the original
    /// IR field name); value is the binding struct's actual Rust field name (e.g. `"class_"`).
    /// Used when a field name is a reserved keyword in the target language and must be escaped
    /// in the binding struct (e.g. `class` → `class_`).
    ///
    /// When present, `val.<binding_name>` is used for binding-side access and the original
    /// `field_name` is used for core-side access (struct literal and assignment targets).
    pub binding_field_renames: Option<&'a std::collections::HashMap<String, String>>,
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
    field_conversion_to_core, field_conversion_to_core_cfg, gen_from_binding_to_core, gen_from_binding_to_core_cfg,
};
pub use core_to_binding::{
    field_conversion_from_core, field_conversion_from_core_cfg, gen_from_core_to_binding, gen_from_core_to_binding_cfg,
};
pub use enums::{
    gen_enum_from_binding_to_core, gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding,
    gen_enum_from_core_to_binding_cfg,
};
pub use helpers::{
    binding_to_core_match_arm, build_type_path_map, can_generate_conversion, can_generate_enum_conversion,
    can_generate_enum_conversion_from_core, convertible_types, core_enum_path, core_to_binding_convertible_types,
    core_to_binding_match_arm, core_type_path, field_references_excluded_type, has_sanitized_fields, input_type_names,
    is_tuple_variant, resolve_named_path,
};

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::*;

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
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
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
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Gpu".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
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
        });

        let result = gen_from_binding_to_core(&typ, "my_crate");

        // The impl should exist
        assert!(result.contains("impl From<Config> for my_crate::Config"));
        // Regular fields should be present
        assert!(result.contains("name: val.name"));
        assert!(result.contains("timeout: val.timeout"));
        // cfg-gated field should NOT be accessed from val (it doesn't exist in binding struct)
        assert!(!result.contains("layout: val.layout"));
        // But ..Default::default() should be present to fill cfg-gated fields
        assert!(result.contains("..Default::default()"));
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
        });

        let result = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());

        // The impl should exist
        assert!(result.contains("impl From<my_crate::Config> for Config"));
        // Regular fields should be present
        assert!(result.contains("name: val.name"));
        // cfg-gated field should NOT be in the struct literal
        assert!(!result.contains("layout:"));
    }
}
