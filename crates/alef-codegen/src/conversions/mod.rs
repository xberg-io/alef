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

    // -----------------------------------------------------------------------
    // Shared test helpers
    // -----------------------------------------------------------------------

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty,
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
        }
    }

    fn make_opt_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            optional: true,
            ..make_field(name, ty)
        }
    }

    fn make_type(name: &str, rust_path: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.into(),
            rust_path: rust_path.into(),
            original_rust_path: String::new(),
            fields,
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

    fn make_enum(name: &str, rust_path: &str, variants: &[&str]) -> EnumDef {
        EnumDef {
            name: name.into(),
            rust_path: rust_path.into(),
            original_rust_path: String::new(),
            variants: variants
                .iter()
                .map(|v| EnumVariant {
                    name: (*v).into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                })
                .collect(),
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }
    }

    fn no_opaques() -> AHashSet<String> {
        AHashSet::new()
    }

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
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Gpu".into(),
                    fields: vec![],
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

    // -----------------------------------------------------------------------
    // helpers.rs: field_conversion_to_core (binding → core)
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_to_core_string() {
        let result = field_conversion_to_core("label", &TypeRef::String, false);
        assert_eq!(result, "label: val.label");
    }

    #[test]
    fn test_field_conversion_to_core_primitive() {
        let result = field_conversion_to_core("count", &TypeRef::Primitive(PrimitiveType::I32), false);
        assert_eq!(result, "count: val.count");
    }

    #[test]
    fn test_field_conversion_to_core_bytes() {
        let result = field_conversion_to_core("data", &TypeRef::Bytes, false);
        assert_eq!(result, "data: val.data");
    }

    #[test]
    fn test_field_conversion_to_core_unit() {
        let result = field_conversion_to_core("nothing", &TypeRef::Unit, false);
        assert_eq!(result, "nothing: val.nothing");
    }

    #[test]
    fn test_field_conversion_to_core_duration_non_optional() {
        let result = field_conversion_to_core("timeout", &TypeRef::Duration, false);
        assert_eq!(result, "timeout: std::time::Duration::from_millis(val.timeout)");
    }

    #[test]
    fn test_field_conversion_to_core_duration_optional() {
        let result = field_conversion_to_core("timeout", &TypeRef::Duration, true);
        assert_eq!(result, "timeout: val.timeout.map(std::time::Duration::from_millis)");
    }

    #[test]
    fn test_field_conversion_to_core_path_non_optional() {
        let result = field_conversion_to_core("file", &TypeRef::Path, false);
        assert_eq!(result, "file: val.file.into()");
    }

    #[test]
    fn test_field_conversion_to_core_path_optional() {
        let result = field_conversion_to_core("file", &TypeRef::Path, true);
        assert_eq!(result, "file: val.file.map(Into::into)");
    }

    #[test]
    fn test_field_conversion_to_core_json_non_optional() {
        let result = field_conversion_to_core("meta", &TypeRef::Json, false);
        assert_eq!(result, "meta: serde_json::from_str(&val.meta).unwrap_or_default()");
    }

    #[test]
    fn test_field_conversion_to_core_json_optional() {
        let result = field_conversion_to_core("meta", &TypeRef::Json, true);
        assert_eq!(
            result,
            "meta: val.meta.as_ref().and_then(|s| serde_json::from_str(s).ok())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_char_non_optional() {
        let result = field_conversion_to_core("sep", &TypeRef::Char, false);
        assert_eq!(result, "sep: val.sep.chars().next().unwrap_or('*')");
    }

    #[test]
    fn test_field_conversion_to_core_char_optional() {
        let result = field_conversion_to_core("sep", &TypeRef::Char, true);
        assert_eq!(result, "sep: val.sep.and_then(|s| s.chars().next())");
    }

    #[test]
    fn test_field_conversion_to_core_named_non_optional() {
        let result = field_conversion_to_core("backend", &TypeRef::Named("Backend".into()), false);
        assert_eq!(result, "backend: val.backend.into()");
    }

    #[test]
    fn test_field_conversion_to_core_named_optional() {
        let result = field_conversion_to_core("backend", &TypeRef::Named("Backend".into()), true);
        assert_eq!(result, "backend: val.backend.map(Into::into)");
    }

    #[test]
    fn test_field_conversion_to_core_named_tuple_type_is_passthrough() {
        // Tuple type names (starting with '(') are passthrough — no conversion
        let result = field_conversion_to_core("pair", &TypeRef::Named("(String, u32)".into()), false);
        assert_eq!(result, "pair: val.pair");
    }

    #[test]
    fn test_field_conversion_to_core_vec_named() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_to_core("items", &ty, false);
        assert_eq!(result, "items: val.items.into_iter().map(Into::into).collect()");
    }

    #[test]
    fn test_field_conversion_to_core_vec_named_optional() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_to_core("items", &ty, true);
        assert_eq!(
            result,
            "items: val.items.map(|v| v.into_iter().map(Into::into).collect())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_vec_tuple_passthrough() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("(u32, u32)".into())));
        let result = field_conversion_to_core("pairs", &ty, false);
        assert_eq!(result, "pairs: val.pairs");
    }

    #[test]
    fn test_field_conversion_to_core_vec_json() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Json));
        let result = field_conversion_to_core("items", &ty, false);
        assert_eq!(
            result,
            "items: val.items.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect()"
        );
    }

    #[test]
    fn test_field_conversion_to_core_optional_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("Config".into())));
        let result = field_conversion_to_core("config", &ty, false);
        assert_eq!(result, "config: val.config.map(Into::into)");
    }

    #[test]
    fn test_field_conversion_to_core_optional_vec_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".into())))));
        let result = field_conversion_to_core("items", &ty, false);
        assert_eq!(
            result,
            "items: val.items.map(|v| v.into_iter().map(Into::into).collect())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_map_string_string() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let result = field_conversion_to_core("map", &ty, false);
        assert_eq!(result, "map: val.map.into_iter().collect()");
    }

    #[test]
    fn test_field_conversion_to_core_map_string_json() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("serde_json::from_str(&v)"));
    }

    #[test]
    fn test_field_conversion_to_core_map_named_values() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Val".into())));
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("v.into()"));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: field_conversion_from_core (core → binding)
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_from_core_string() {
        let result = field_conversion_from_core("label", &TypeRef::String, false, false, &no_opaques());
        assert_eq!(result, "label: val.label");
    }

    #[test]
    fn test_field_conversion_from_core_duration_non_optional() {
        let result = field_conversion_from_core("timeout", &TypeRef::Duration, false, false, &no_opaques());
        assert_eq!(result, "timeout: val.timeout.as_millis() as u64");
    }

    #[test]
    fn test_field_conversion_from_core_duration_optional() {
        let result = field_conversion_from_core("timeout", &TypeRef::Duration, true, false, &no_opaques());
        assert_eq!(result, "timeout: val.timeout.map(|d| d.as_millis() as u64)");
    }

    #[test]
    fn test_field_conversion_from_core_path_non_optional() {
        let result = field_conversion_from_core("file", &TypeRef::Path, false, false, &no_opaques());
        assert_eq!(result, "file: val.file.to_string_lossy().to_string()");
    }

    #[test]
    fn test_field_conversion_from_core_path_optional() {
        let result = field_conversion_from_core("file", &TypeRef::Path, true, false, &no_opaques());
        assert_eq!(result, "file: val.file.map(|p| p.to_string_lossy().to_string())");
    }

    #[test]
    fn test_field_conversion_from_core_char_non_optional() {
        let result = field_conversion_from_core("sep", &TypeRef::Char, false, false, &no_opaques());
        assert_eq!(result, "sep: val.sep.to_string()");
    }

    #[test]
    fn test_field_conversion_from_core_char_optional() {
        let result = field_conversion_from_core("sep", &TypeRef::Char, true, false, &no_opaques());
        assert_eq!(result, "sep: val.sep.map(|c| c.to_string())");
    }

    #[test]
    fn test_field_conversion_from_core_bytes_non_optional() {
        let result = field_conversion_from_core("data", &TypeRef::Bytes, false, false, &no_opaques());
        assert_eq!(result, "data: val.data.to_vec()");
    }

    #[test]
    fn test_field_conversion_from_core_bytes_optional() {
        let result = field_conversion_from_core("data", &TypeRef::Bytes, true, false, &no_opaques());
        assert_eq!(result, "data: val.data.map(|v| v.to_vec())");
    }

    #[test]
    fn test_field_conversion_from_core_json_non_optional() {
        let result = field_conversion_from_core("meta", &TypeRef::Json, false, false, &no_opaques());
        assert_eq!(result, "meta: val.meta.to_string()");
    }

    #[test]
    fn test_field_conversion_from_core_json_optional() {
        let result = field_conversion_from_core("meta", &TypeRef::Json, true, false, &no_opaques());
        assert_eq!(result, "meta: val.meta.as_ref().map(ToString::to_string)");
    }

    #[test]
    fn test_field_conversion_from_core_named_non_opaque() {
        // Non-opaque Named uses .into() (symmetric with binding_to_core)
        let result = field_conversion_from_core(
            "backend",
            &TypeRef::Named("Backend".into()),
            false,
            false,
            &no_opaques(),
        );
        assert_eq!(result, "backend: val.backend.into()");
    }

    #[test]
    fn test_field_conversion_from_core_opaque_non_optional() {
        let mut opaques = AHashSet::new();
        opaques.insert("Client".to_string());
        let result = field_conversion_from_core("client", &TypeRef::Named("Client".into()), false, false, &opaques);
        assert_eq!(result, "client: Client { inner: Arc::new(val.client) }");
    }

    #[test]
    fn test_field_conversion_from_core_opaque_optional() {
        let mut opaques = AHashSet::new();
        opaques.insert("Client".to_string());
        let result = field_conversion_from_core("client", &TypeRef::Named("Client".into()), true, false, &opaques);
        assert_eq!(result, "client: val.client.map(|v| Client { inner: Arc::new(v) })");
    }

    #[test]
    fn test_field_conversion_from_core_vec_json() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Json));
        let result = field_conversion_from_core("items", &ty, false, false, &no_opaques());
        assert_eq!(result, "items: val.items.iter().map(ToString::to_string).collect()");
    }

    #[test]
    fn test_field_conversion_from_core_map_json_values() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
        let result = field_conversion_from_core("map", &ty, false, false, &no_opaques());
        assert!(result.contains("v.to_string()"));
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_string() {
        // Sanitized String field uses Debug formatting
        let result = field_conversion_from_core("value", &TypeRef::String, false, true, &no_opaques());
        assert_eq!(result, "value: format!(\"{:?}\", val.value)");
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_string_optional() {
        let result = field_conversion_from_core("value", &TypeRef::String, true, true, &no_opaques());
        assert_eq!(result, "value: val.value.as_ref().map(|v| format!(\"{v:?}\"))");
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_named_non_optional() {
        // Sanitized Named non-optional → empty String (type may not have Debug)
        let result = field_conversion_from_core("obj", &TypeRef::Named("Opaque".into()), false, true, &no_opaques());
        assert_eq!(result, "obj: String::new()");
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_named_optional() {
        // Sanitized Named optional → None
        let result = field_conversion_from_core("obj", &TypeRef::Named("Opaque".into()), true, true, &no_opaques());
        assert_eq!(result, "obj: None");
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_vec_string() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        let result = field_conversion_from_core("tags", &ty, false, true, &no_opaques());
        // Generated code contains a format!("{:?}", i) expression inside .map()
        assert!(result.contains(r#"format!("{:?}", i)"#));
        assert!(result.contains(".iter().map("));
    }

    #[test]
    fn test_field_conversion_from_core_sanitized_map_string_string() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let result = field_conversion_from_core("headers", &ty, false, true, &no_opaques());
        assert!(result.contains("k.to_string()"));
        assert!(result.contains("v.to_string()"));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: is_tuple_variant / is_newtype / is_tuple_type_name
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_tuple_variant_true_for_positional_fields() {
        let fields = vec![
            make_field("_0", TypeRef::String),
            make_field("_1", TypeRef::Primitive(PrimitiveType::I32)),
        ];
        assert!(is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_variant_false_for_named_fields() {
        let fields = vec![make_field("name", TypeRef::String)];
        assert!(!is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_variant_false_for_empty_fields() {
        assert!(!is_tuple_variant(&[]));
    }

    #[test]
    fn test_is_tuple_type_name_true() {
        assert!(helpers::is_tuple_type_name("(String, u32)"));
    }

    #[test]
    fn test_is_tuple_type_name_false() {
        assert!(!helpers::is_tuple_type_name("Config"));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: field_references_excluded_type
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_references_excluded_type_direct_match() {
        let ty = TypeRef::Named("JsValue".into());
        assert!(field_references_excluded_type(&ty, &["JsValue".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_no_match() {
        let ty = TypeRef::Named("Config".into());
        assert!(!field_references_excluded_type(&ty, &["JsValue".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_inside_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("Excluded".into())));
        assert!(field_references_excluded_type(&ty, &["Excluded".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_inside_vec() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Excluded".into())));
        assert!(field_references_excluded_type(&ty, &["Excluded".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_inside_map_key() {
        let ty = TypeRef::Map(Box::new(TypeRef::Named("Excluded".into())), Box::new(TypeRef::String));
        assert!(field_references_excluded_type(&ty, &["Excluded".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_inside_map_value() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Excluded".into())));
        assert!(field_references_excluded_type(&ty, &["Excluded".to_string()]));
    }

    #[test]
    fn test_field_references_excluded_type_primitive_not_excluded() {
        let ty = TypeRef::Primitive(PrimitiveType::I32);
        assert!(!field_references_excluded_type(&ty, &["JsValue".to_string()]));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: can_generate_enum_conversion / can_generate_enum_conversion_from_core
    // -----------------------------------------------------------------------

    #[test]
    fn test_can_generate_enum_conversion_with_variants() {
        let e = make_enum("Color", "crate::Color", &["Red", "Green"]);
        assert!(can_generate_enum_conversion(&e));
    }

    #[test]
    fn test_can_generate_enum_conversion_empty_variants() {
        let e = make_enum("Empty", "crate::Empty", &[]);
        assert!(!can_generate_enum_conversion(&e));
    }

    #[test]
    fn test_can_generate_enum_conversion_from_core_with_variants() {
        let e = make_enum("Color", "crate::Color", &["Red"]);
        assert!(can_generate_enum_conversion_from_core(&e));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: core_type_path / core_enum_path
    // -----------------------------------------------------------------------

    #[test]
    fn test_core_type_path_with_full_path() {
        let typ = make_type("Config", "my_crate::types::Config", vec![]);
        assert_eq!(core_type_path(&typ, "my_crate"), "my_crate::types::Config");
    }

    #[test]
    fn test_core_type_path_with_bare_name() {
        // When rust_path has no "::", prefix with core_import::name
        let typ = make_type("Config", "Config", vec![]);
        assert_eq!(core_type_path(&typ, "my_crate"), "my_crate::Config");
    }

    #[test]
    fn test_core_type_path_normalizes_hyphens() {
        let typ = make_type("Config", "my-crate::Config", vec![]);
        assert_eq!(core_type_path(&typ, "my_crate"), "my_crate::Config");
    }

    #[test]
    fn test_core_enum_path_with_full_path() {
        let e = make_enum("Backend", "my_crate::Backend", &[]);
        assert_eq!(core_enum_path(&e, "my_crate"), "my_crate::Backend");
    }

    #[test]
    fn test_core_enum_path_bare_name_gets_prefixed() {
        let e = make_enum("Backend", "Backend", &[]);
        assert_eq!(core_enum_path(&e, "my_crate"), "my_crate::Backend");
    }

    // -----------------------------------------------------------------------
    // helpers.rs: build_type_path_map / resolve_named_path
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_type_path_map_includes_types_and_enums() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![make_type("Config", "my_crate::Config", vec![])],
            functions: vec![],
            enums: vec![make_enum("Mode", "my_crate::Mode", &["A"])],
            errors: vec![],
        };
        let map = build_type_path_map(&surface, "my_crate");
        assert_eq!(map.get("Config").map(String::as_str), Some("my_crate::Config"));
        assert_eq!(map.get("Mode").map(String::as_str), Some("my_crate::Mode"));
    }

    #[test]
    fn test_resolve_named_path_found_in_map() {
        let mut map = ahash::AHashMap::new();
        map.insert("Config".to_string(), "my_crate::types::Config".to_string());
        assert_eq!(
            resolve_named_path("Config", "my_crate", &map),
            "my_crate::types::Config"
        );
    }

    #[test]
    fn test_resolve_named_path_not_found_falls_back() {
        let map = ahash::AHashMap::new();
        assert_eq!(resolve_named_path("Unknown", "my_crate", &map), "my_crate::Unknown");
    }

    // -----------------------------------------------------------------------
    // helpers.rs: input_type_names
    // -----------------------------------------------------------------------

    #[test]
    fn test_input_type_names_from_function_params() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "process".into(),
                rust_path: "my_crate::process".into(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "config".into(),
                    ty: TypeRef::Named("Config".into()),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                }],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        };
        let names = input_type_names(&surface);
        assert!(names.contains("Config"));
    }

    #[test]
    fn test_input_type_names_from_return_types() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "get_result".into(),
                rust_path: "my_crate::get_result".into(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Named("Result".into()),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        };
        let names = input_type_names(&surface);
        assert!(names.contains("Result"));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: binding_to_core_match_arm / core_to_binding_match_arm
    // -----------------------------------------------------------------------

    #[test]
    fn test_binding_to_core_match_arm_unit_variant() {
        let result = binding_to_core_match_arm("MyEnum", "Foo", &[]);
        assert_eq!(result, "MyEnum::Foo => Self::Foo,");
    }

    #[test]
    fn test_binding_to_core_match_arm_data_variant_no_binding_data() {
        // When binding is unit-only and core has named fields, use Default
        let fields = vec![make_field("value", TypeRef::String)];
        let result = helpers::binding_to_core_match_arm_ext("MyEnum", "Foo", &fields, false);
        assert!(result.contains("value: Default::default()"));
    }

    #[test]
    fn test_binding_to_core_match_arm_tuple_variant_no_binding_data() {
        let fields = vec![make_field("_0", TypeRef::String)];
        let result = helpers::binding_to_core_match_arm_ext("MyEnum", "Foo", &fields, false);
        assert!(result.contains("Default::default()"));
        assert!(result.contains("Self::Foo("));
    }

    #[test]
    fn test_binding_to_core_match_arm_data_variant_with_binding_data_named() {
        // Binding has data — destructure and convert
        let fields = vec![make_field("value", TypeRef::Named("Inner".into()))];
        let result = helpers::binding_to_core_match_arm_ext("MyEnum", "Foo", &fields, true);
        assert!(result.contains("value: value.into()"));
    }

    #[test]
    fn test_binding_to_core_match_arm_data_variant_with_binding_data_tuple() {
        let fields = vec![make_field("_0", TypeRef::Named("Inner".into()))];
        let result = helpers::binding_to_core_match_arm_ext("MyEnum", "Foo", &fields, true);
        assert!(result.contains("_0.into()"));
    }

    #[test]
    fn test_core_to_binding_match_arm_unit_variant() {
        let result = core_to_binding_match_arm("CoreEnum", "Bar", &[]);
        assert_eq!(result, "CoreEnum::Bar => Self::Bar,");
    }

    #[test]
    fn test_core_to_binding_match_arm_data_named_no_binding_data() {
        let fields = vec![make_field("x", TypeRef::Primitive(PrimitiveType::I32))];
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &fields, false);
        assert!(result.contains("{ .. }"));
        assert!(result.contains("Self::Bar"));
    }

    #[test]
    fn test_core_to_binding_match_arm_data_tuple_no_binding_data() {
        let fields = vec![make_field("_0", TypeRef::Primitive(PrimitiveType::I32))];
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &fields, false);
        assert!(result.contains("(..)"));
        assert!(result.contains("Self::Bar"));
    }

    // -----------------------------------------------------------------------
    // helpers.rs: has_sanitized_fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_has_sanitized_fields_false() {
        let typ = make_type("Foo", "crate::Foo", vec![make_field("x", TypeRef::String)]);
        assert!(!has_sanitized_fields(&typ));
    }

    #[test]
    fn test_has_sanitized_fields_true() {
        let mut field = make_field("x", TypeRef::String);
        field.sanitized = true;
        let typ = make_type("Foo", "crate::Foo", vec![field]);
        assert!(has_sanitized_fields(&typ));
    }

    // -----------------------------------------------------------------------
    // binding_to_core.rs: gen_from_binding_to_core — various field types
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_from_binding_to_core_string_field() {
        let typ = make_type("S", "c::S", vec![make_field("title", TypeRef::String)]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("impl From<S> for c::S"));
        assert!(result.contains("title: val.title"));
    }

    #[test]
    fn test_gen_from_binding_to_core_duration_field() {
        let typ = make_type("S", "c::S", vec![make_field("timeout", TypeRef::Duration)]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("std::time::Duration::from_millis(val.timeout)"));
    }

    #[test]
    fn test_gen_from_binding_to_core_optional_named_field() {
        let field = make_opt_field("backend", TypeRef::Named("Backend".into()));
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("backend: val.backend.map(Into::into)"));
    }

    #[test]
    fn test_gen_from_binding_to_core_vec_named_field() {
        let field = make_field("items", TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))));
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("into_iter().map(Into::into).collect()"));
    }

    #[test]
    fn test_gen_from_binding_to_core_sanitized_field_uses_default() {
        let mut field = make_field("complex", TypeRef::String);
        field.sanitized = true;
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("complex: Default::default()"));
    }

    #[test]
    fn test_gen_from_binding_to_core_with_stripped_cfg_fields_uses_default_update() {
        let mut typ = make_type("S", "c::S", vec![make_field("x", TypeRef::String)]);
        typ.has_stripped_cfg_fields = true;
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("..Default::default()"));
        assert!(result.contains("#[allow(clippy::needless_update)]"));
    }

    #[test]
    fn test_gen_from_binding_to_core_with_type_name_prefix() {
        let typ = make_type("Config", "c::Config", vec![make_field("x", TypeRef::String)]);
        let config = ConversionConfig {
            type_name_prefix: "Js",
            ..Default::default()
        };
        let result = gen_from_binding_to_core_cfg(&typ, "c", &config);
        assert!(result.contains("impl From<JsConfig> for c::Config"));
    }

    #[test]
    fn test_gen_from_binding_to_core_path_field() {
        let field = make_field("file", TypeRef::Path);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("file: val.file.into()"));
    }

    #[test]
    fn test_gen_from_binding_to_core_json_field() {
        let field = make_field("meta", TypeRef::Json);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("serde_json::from_str(&val.meta).unwrap_or_default()"));
    }

    #[test]
    fn test_gen_from_binding_to_core_newtype_struct() {
        // A newtype is a struct with single field named "_0"
        let field = make_field("_0", TypeRef::Primitive(PrimitiveType::U32));
        let typ = make_type("NodeIndex", "c::NodeIndex", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("Self(val._0)"));
    }

    #[test]
    fn test_gen_from_binding_to_core_newtype_named_field() {
        let field = make_field("_0", TypeRef::Named("Inner".into()));
        let typ = make_type("Wrapper", "c::Wrapper", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("Self(val._0.into())"));
    }

    #[test]
    fn test_gen_from_binding_to_core_newtype_path_field() {
        let field = make_field("_0", TypeRef::Path);
        let typ = make_type("PathWrapper", "c::PathWrapper", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("Self(val._0.into())"));
    }

    #[test]
    fn test_gen_from_binding_to_core_newtype_duration_field() {
        let field = make_field("_0", TypeRef::Duration);
        let typ = make_type("DurWrapper", "c::DurWrapper", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("Self(std::time::Duration::from_millis(val._0))"));
    }

    #[test]
    fn test_gen_from_binding_to_core_boxed_named_field() {
        let mut field = make_field("child", TypeRef::Named("Child".into()));
        field.is_boxed = true;
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("Box::new("));
    }

    #[test]
    fn test_gen_from_binding_to_core_cast_large_ints_to_i64() {
        let field = make_field("count", TypeRef::Primitive(PrimitiveType::U64));
        let typ = make_type("S", "c::S", vec![field]);
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = gen_from_binding_to_core_cfg(&typ, "c", &config);
        assert!(result.contains("val.count as u64"));
    }

    #[test]
    fn test_gen_from_binding_to_core_exclude_types_skips_field() {
        let field = make_field("js_val", TypeRef::Named("JsValue".into()));
        let typ = make_type("S", "c::S", vec![field]);
        let excluded = vec!["JsValue".to_string()];
        let config = ConversionConfig {
            exclude_types: &excluded,
            ..Default::default()
        };
        let result = gen_from_binding_to_core_cfg(&typ, "c", &config);
        assert!(result.contains("js_val: Default::default()"));
    }

    // -----------------------------------------------------------------------
    // core_to_binding.rs: gen_from_core_to_binding — various field types
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_from_core_to_binding_string_field() {
        let typ = make_type("S", "c::S", vec![make_field("title", TypeRef::String)]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("impl From<c::S> for S"));
        assert!(result.contains("title: val.title"));
    }

    #[test]
    fn test_gen_from_core_to_binding_duration_field() {
        let field = make_field("timeout", TypeRef::Duration);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.timeout.as_millis() as u64"));
    }

    #[test]
    fn test_gen_from_core_to_binding_path_field() {
        let field = make_field("path", TypeRef::Path);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("to_string_lossy().to_string()"));
    }

    #[test]
    fn test_gen_from_core_to_binding_json_field() {
        let field = make_field("meta", TypeRef::Json);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.meta.to_string()"));
    }

    #[test]
    fn test_gen_from_core_to_binding_opaque_field() {
        let mut opaques = AHashSet::new();
        opaques.insert("Client".to_string());
        let field = make_field("client", TypeRef::Named("Client".into()));
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &opaques);
        assert!(result.contains("Arc::new(val.client)"));
        assert!(result.contains("Client { inner:"));
    }

    #[test]
    fn test_gen_from_core_to_binding_with_type_name_prefix_opaque() {
        let mut opaques = AHashSet::new();
        opaques.insert("Client".to_string());
        let field = make_field("client", TypeRef::Named("Client".into()));
        let typ = make_type("S", "c::S", vec![field]);
        let config = ConversionConfig {
            type_name_prefix: "Js",
            ..Default::default()
        };
        let result = gen_from_core_to_binding_cfg(&typ, "c", &opaques, &config);
        assert!(result.contains("JsClient { inner: Arc::new(val.client) }"));
    }

    #[test]
    fn test_gen_from_core_to_binding_sanitized_field() {
        let mut field = make_field("complex", TypeRef::String);
        field.sanitized = true;
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("format!("));
    }

    #[test]
    fn test_gen_from_core_to_binding_newtype_struct() {
        let field = make_field("_0", TypeRef::Primitive(PrimitiveType::U32));
        let typ = make_type("NodeIndex", "c::NodeIndex", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("Self { _0: val.0 }"));
    }

    #[test]
    fn test_gen_from_core_to_binding_newtype_path_field() {
        let field = make_field("_0", TypeRef::Path);
        let typ = make_type("PathWrapper", "c::PathWrapper", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.0.to_string_lossy().to_string()"));
    }

    #[test]
    fn test_gen_from_core_to_binding_newtype_duration_field() {
        let field = make_field("_0", TypeRef::Duration);
        let typ = make_type("DurWrapper", "c::DurWrapper", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.0.as_millis() as u64"));
    }

    #[test]
    fn test_gen_from_core_to_binding_newtype_named_field() {
        let field = make_field("_0", TypeRef::Named("Inner".into()));
        let typ = make_type("Wrapper", "c::Wrapper", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.0.into()"));
    }

    #[test]
    fn test_gen_from_core_to_binding_cast_large_ints_to_i64() {
        let field = make_field("count", TypeRef::Primitive(PrimitiveType::U64));
        let typ = make_type("S", "c::S", vec![field]);
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = gen_from_core_to_binding_cfg(&typ, "c", &no_opaques(), &config);
        assert!(result.contains("val.count as i64"));
    }

    #[test]
    fn test_gen_from_core_to_binding_cast_f32_to_f64() {
        let field = make_field("score", TypeRef::Primitive(PrimitiveType::F32));
        let typ = make_type("S", "c::S", vec![field]);
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let result = gen_from_core_to_binding_cfg(&typ, "c", &no_opaques(), &config);
        assert!(result.contains("val.score as f64"));
    }

    #[test]
    fn test_gen_from_core_to_binding_exclude_types_skips_field() {
        let field = make_field("js_val", TypeRef::Named("JsValue".into()));
        let typ = make_type("S", "c::S", vec![field]);
        let excluded = vec!["JsValue".to_string()];
        let config = ConversionConfig {
            exclude_types: &excluded,
            ..Default::default()
        };
        let result = gen_from_core_to_binding_cfg(&typ, "c", &no_opaques(), &config);
        // The field must be absent from the generated struct literal
        assert!(!result.contains("js_val:"));
    }

    #[test]
    fn test_gen_from_core_to_binding_vec_json_field() {
        let field = make_field("items", TypeRef::Vec(Box::new(TypeRef::Json)));
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("map(ToString::to_string)"));
    }

    #[test]
    fn test_gen_from_core_to_binding_char_field() {
        let field = make_field("sep", TypeRef::Char);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.sep.to_string()"));
    }

    #[test]
    fn test_gen_from_core_to_binding_bytes_field() {
        let field = make_field("data", TypeRef::Bytes);
        let typ = make_type("S", "c::S", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("val.data.to_vec()"));
    }

    // -----------------------------------------------------------------------
    // cfg_field: field_conversion_to_core_cfg — backend-specific variations
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_to_core_cfg_no_flags_delegates_to_base() {
        let config = ConversionConfig::default();
        let result = field_conversion_to_core_cfg("x", &TypeRef::String, false, &config);
        assert_eq!(result, "x: val.x");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_u64_to_i64() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::U64), false, &config);
        assert_eq!(result, "n: val.n as u64");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_usize_to_i64() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::Usize), false, &config);
        assert_eq!(result, "n: val.n as usize");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_isize_to_i64() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::Isize), false, &config);
        assert_eq!(result, "n: val.n as isize");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_f32_cast() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("s", &TypeRef::Primitive(PrimitiveType::F32), false, &config);
        assert_eq!(result, "s: val.s as f32");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_duration_cast() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("t", &TypeRef::Duration, false, &config);
        assert_eq!(result, "t: std::time::Duration::from_millis(val.t as u64)");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_json_to_string() {
        let config = ConversionConfig {
            json_to_string: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("m", &TypeRef::Json, false, &config);
        assert_eq!(result, "m: Default::default()");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_vec_named_to_string() {
        let config = ConversionConfig {
            vec_named_to_string: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_to_core_cfg("items", &ty, false, &config);
        assert_eq!(result, "items: serde_json::from_str(&val.items).unwrap_or_default()");
    }

    // -----------------------------------------------------------------------
    // field_conversion_from_core_cfg — backend-specific variations
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_from_core_cfg_no_flags_delegates_to_base() {
        let config = ConversionConfig::default();
        let result = field_conversion_from_core_cfg("x", &TypeRef::String, false, false, &no_opaques(), &config);
        assert_eq!(result, "x: val.x");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_cast_u64_to_i64() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "n",
            &TypeRef::Primitive(PrimitiveType::U64),
            false,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "n: val.n as i64");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_cast_f32_to_f64() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "s",
            &TypeRef::Primitive(PrimitiveType::F32),
            false,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "s: val.s as f64");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_duration_cast_to_i64() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg("t", &TypeRef::Duration, false, false, &no_opaques(), &config);
        assert_eq!(result, "t: val.t.as_millis() as u64 as i64");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_json_to_string() {
        let config = ConversionConfig {
            json_to_string: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg("m", &TypeRef::Json, false, false, &no_opaques(), &config);
        assert_eq!(result, "m: val.m.to_string()");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_named_to_string() {
        let config = ConversionConfig {
            vec_named_to_string: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_from_core_cfg("items", &ty, false, false, &no_opaques(), &config);
        assert_eq!(result, "items: serde_json::to_string(&val.items).unwrap_or_default()");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_u64_cast() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
        let result = field_conversion_from_core_cfg("ids", &ty, false, false, &no_opaques(), &config);
        assert!(result.contains("as i64"));
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_f32_cast() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32)));
        let result = field_conversion_from_core_cfg("scores", &ty, false, false, &no_opaques(), &config);
        assert!(result.contains("as f64"));
    }

    #[test]
    fn test_field_conversion_from_core_cfg_map_u64_values_cast() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::U64)),
        );
        let result = field_conversion_from_core_cfg("map", &ty, false, false, &no_opaques(), &config);
        assert!(result.contains("as i64"));
    }

    // -----------------------------------------------------------------------
    // convertible_types / core_to_binding_convertible_types
    // -----------------------------------------------------------------------

    #[test]
    fn test_convertible_types_simple_struct() {
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![make_type("Config", "c::Config", vec![make_field("x", TypeRef::String)])],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let result = convertible_types(&surface);
        assert!(result.contains("Config"));
    }

    #[test]
    fn test_convertible_types_excludes_type_with_unconvertible_named_field() {
        // "Unknown" is not in the surface — types referencing it are removed
        let field = make_field("inner", TypeRef::Named("Unknown".into()));
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![make_type("Wrapper", "c::Wrapper", vec![field])],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let result = convertible_types(&surface);
        assert!(!result.contains("Wrapper"));
    }

    #[test]
    fn test_core_to_binding_convertible_types_simple() {
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![make_type("Config", "c::Config", vec![make_field("x", TypeRef::String)])],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let result = core_to_binding_convertible_types(&surface);
        assert!(result.contains("Config"));
    }

    #[test]
    fn test_can_generate_conversion_true_when_in_set() {
        let mut set = AHashSet::new();
        set.insert("Config".to_string());
        let typ = make_type("Config", "c::Config", vec![]);
        assert!(can_generate_conversion(&typ, &set));
    }

    #[test]
    fn test_can_generate_conversion_false_when_absent() {
        let set = AHashSet::new();
        let typ = make_type("Config", "c::Config", vec![]);
        assert!(!can_generate_conversion(&typ, &set));
    }

    // -----------------------------------------------------------------------
    // field_conversion_to_core_cfg — optional variants of cast flags
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_to_core_cfg_cast_u64_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::U64), true, &config);
        assert_eq!(result, "n: val.n.map(|v| v as u64)");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_usize_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::Usize), true, &config);
        assert_eq!(result, "n: val.n.map(|v| v as usize)");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_isize_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("n", &TypeRef::Primitive(PrimitiveType::Isize), true, &config);
        assert_eq!(result, "n: val.n.map(|v| v as isize)");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_cast_f32_optional() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("s", &TypeRef::Primitive(PrimitiveType::F32), true, &config);
        assert_eq!(result, "s: val.s.map(|v| v as f32)");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_duration_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("t", &TypeRef::Duration, true, &config);
        assert_eq!(result, "t: val.t.map(|v| std::time::Duration::from_millis(v as u64))");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_json_to_string_optional() {
        let config = ConversionConfig {
            json_to_string: true,
            ..Default::default()
        };
        let result = field_conversion_to_core_cfg("m", &TypeRef::Json, true, &config);
        // json_to_string is lossy; optional still falls through to Default::default() path
        assert_eq!(result, "m: Default::default()");
    }

    #[test]
    fn test_field_conversion_to_core_cfg_vec_named_to_string_optional() {
        let config = ConversionConfig {
            vec_named_to_string: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_to_core_cfg("items", &ty, true, &config);
        assert_eq!(
            result,
            "items: val.items.as_ref().and_then(|s| serde_json::from_str(s).ok())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_cfg_vec_u64_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
        let result = field_conversion_to_core_cfg("ids", &ty, true, &config);
        assert!(result.contains("as u64"));
    }

    #[test]
    fn test_field_conversion_to_core_cfg_vec_f32_cast_optional() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32)));
        let result = field_conversion_to_core_cfg("scores", &ty, true, &config);
        assert!(result.contains("as f32"));
    }

    #[test]
    fn test_field_conversion_to_core_cfg_map_u64_values_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::U64)),
        );
        let result = field_conversion_to_core_cfg("map", &ty, true, &config);
        assert!(result.contains("as u64"));
    }

    #[test]
    fn test_field_conversion_to_core_cfg_optional_inner_u64_cast() {
        // Optional(Primitive(U64)) should map element with cast
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
        let result = field_conversion_to_core_cfg("n", &ty, false, &config);
        assert!(result.contains("as u64"));
    }

    // -----------------------------------------------------------------------
    // field_conversion_from_core_cfg — optional variants and missing branches
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_from_core_cfg_cast_u64_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "n",
            &TypeRef::Primitive(PrimitiveType::U64),
            true,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "n: val.n.map(|v| v as i64)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_cast_usize_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "n",
            &TypeRef::Primitive(PrimitiveType::Usize),
            true,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "n: val.n.map(|v| v as i64)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_cast_isize_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "n",
            &TypeRef::Primitive(PrimitiveType::Isize),
            true,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "n: val.n.map(|v| v as i64)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_cast_f32_optional() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg(
            "s",
            &TypeRef::Primitive(PrimitiveType::F32),
            true,
            false,
            &no_opaques(),
            &config,
        );
        assert_eq!(result, "s: val.s.map(|v| v as f64)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_duration_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg("t", &TypeRef::Duration, true, false, &no_opaques(), &config);
        assert_eq!(result, "t: val.t.map(|d| d.as_millis() as u64 as i64)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_json_to_string_optional() {
        let config = ConversionConfig {
            json_to_string: true,
            ..Default::default()
        };
        let result = field_conversion_from_core_cfg("m", &TypeRef::Json, true, false, &no_opaques(), &config);
        assert_eq!(result, "m: val.m.as_ref().map(ToString::to_string)");
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_named_to_string_optional() {
        let config = ConversionConfig {
            vec_named_to_string: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".into())));
        let result = field_conversion_from_core_cfg("items", &ty, true, false, &no_opaques(), &config);
        assert_eq!(
            result,
            "items: val.items.as_ref().and_then(|v| serde_json::to_string(v).ok())"
        );
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_u64_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
        let result = field_conversion_from_core_cfg("ids", &ty, true, false, &no_opaques(), &config);
        assert!(result.contains("as i64"));
    }

    #[test]
    fn test_field_conversion_from_core_cfg_vec_f32_cast_optional() {
        let config = ConversionConfig {
            cast_f32_to_f64: true,
            ..Default::default()
        };
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32)));
        let result = field_conversion_from_core_cfg("scores", &ty, true, false, &no_opaques(), &config);
        assert!(result.contains("as f64"));
    }

    #[test]
    fn test_field_conversion_from_core_cfg_optional_inner_u64_cast() {
        // Optional(Primitive(U64)) with cast should map element
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
        let result = field_conversion_from_core_cfg("n", &ty, false, false, &no_opaques(), &config);
        assert!(result.contains("as i64"));
    }

    #[test]
    fn test_field_conversion_from_core_cfg_map_u64_values_cast_optional() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::U64)),
        );
        let result = field_conversion_from_core_cfg("map", &ty, true, false, &no_opaques(), &config);
        assert!(result.contains("as i64"));
    }

    // -----------------------------------------------------------------------
    // Complex nested types: Optional<Vec<Named>>, Map<String, Optional<Named>>,
    // Vec<Map<String, String>>
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_conversion_to_core_optional_vec_string() {
        // Optional(Vec(String)) — passthrough
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        let result = field_conversion_to_core("items", &ty, false);
        assert_eq!(result, "items: val.items");
    }

    #[test]
    fn test_field_conversion_to_core_optional_vec_named_inner() {
        // Optional(Vec(Named)) — map into
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".into())))));
        let result = field_conversion_to_core("items", &ty, false);
        assert_eq!(
            result,
            "items: val.items.map(|v| v.into_iter().map(Into::into).collect())"
        );
    }

    #[test]
    fn test_field_conversion_to_core_map_string_optional_named() {
        // Map(String, Optional(Named)) — named value uses .into()
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Optional(Box::new(TypeRef::Named("Val".into())))),
        );
        // Optional inner Named doesn't trigger `has_named_val` (only TypeRef::Named at top), so falls
        // through to plain collect (there's no special map for Optional-value Named).
        let result = field_conversion_to_core("map", &ty, false);
        assert_eq!(result, "map: val.map.into_iter().collect()");
    }

    #[test]
    fn test_field_conversion_to_core_map_string_vec_named_value() {
        // Map(String, Vec(Named)) — Vec<Named> values need per-vector Into mapping
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".into())))),
        );
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("v.into_iter().map(Into::into).collect()"));
    }

    #[test]
    fn test_field_conversion_to_core_map_string_vec_json_value() {
        // Map(String, Vec(Json)) — Vec<Json> values need per-vector serde deserialization
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Vec(Box::new(TypeRef::Json))),
        );
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("filter_map(|s| serde_json::from_str(&s).ok()).collect()"));
    }

    #[test]
    fn test_field_conversion_to_core_map_string_string_optional() {
        // Map(String, String) with optional=true
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let result = field_conversion_to_core("map", &ty, true);
        assert_eq!(result, "map: val.map.map(|m| m.into_iter().collect())");
    }

    #[test]
    fn test_field_conversion_from_core_optional_vec_named() {
        // Optional(Vec(Named)) — per-element .into() mapping
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".into())))));
        let result = field_conversion_from_core("items", &ty, false, false, &no_opaques());
        // falls through to field_conversion_to_core (symmetric case)
        assert!(result.contains("map(Into::into)") || result.contains("into_iter().map(Into::into)"));
    }

    #[test]
    fn test_field_conversion_from_core_map_string_named_values() {
        // Map(String, Named) — Named values need .into()
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Val".into())));
        let result = field_conversion_from_core("map", &ty, false, false, &no_opaques());
        // No asymmetric logic for Named map values in from_core — falls through to to_core symmetric
        assert!(result.contains("v.into()"));
    }

    #[test]
    fn test_field_conversion_to_core_vec_string_passthrough() {
        // Vec(String) — passthrough (not a special case)
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        let result = field_conversion_to_core("tags", &ty, false);
        assert_eq!(result, "tags: val.tags");
    }

    #[test]
    fn test_field_conversion_to_core_vec_string_optional() {
        // Vec(String) optional
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        let result = field_conversion_to_core("tags", &ty, true);
        assert_eq!(result, "tags: val.tags");
    }

    #[test]
    fn test_field_conversion_to_core_map_named_key() {
        // Map(Named, String) — named key needs .into()
        let ty = TypeRef::Map(Box::new(TypeRef::Named("Key".into())), Box::new(TypeRef::String));
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("k.into()"));
    }

    #[test]
    fn test_field_conversion_to_core_map_json_key() {
        // Map(Json, String) — Json key gets deserialized
        let ty = TypeRef::Map(Box::new(TypeRef::Json), Box::new(TypeRef::String));
        let result = field_conversion_to_core("map", &ty, false);
        assert!(result.contains("serde_json::from_str(&k)"));
    }

    #[test]
    fn test_field_conversion_from_core_optional_json_inner() {
        // Optional(Json) — binding uses Option<String> via .to_string()
        let ty = TypeRef::Optional(Box::new(TypeRef::Json));
        let result = field_conversion_from_core("meta", &ty, false, false, &no_opaques());
        assert_eq!(result, "meta: val.meta.as_ref().map(ToString::to_string)");
    }

    #[test]
    fn test_field_conversion_from_core_optional_path_inner() {
        // Optional(Path) — binding uses to_string_lossy
        let ty = TypeRef::Optional(Box::new(TypeRef::Path));
        let result = field_conversion_from_core("file", &ty, false, false, &no_opaques());
        assert_eq!(result, "file: val.file.map(|p| p.to_string_lossy().to_string())");
    }

    #[test]
    fn test_field_conversion_from_core_map_json_keys() {
        // Map(Json, String) — Json key gets .to_string()
        let ty = TypeRef::Map(Box::new(TypeRef::Json), Box::new(TypeRef::String));
        let result = field_conversion_from_core("map", &ty, false, false, &no_opaques());
        assert!(result.contains("k.to_string()"));
    }

    // -----------------------------------------------------------------------
    // is_tuple_variant edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_tuple_variant_single_positional_field() {
        let fields = vec![make_field("_0", TypeRef::String)];
        assert!(is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_variant_true_for_underscore_only() {
        // "_".strip_prefix('_') == Some("") and "".chars().all(is_ascii_digit) is vacuously true
        let fields = vec![make_field("_", TypeRef::String)];
        assert!(is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_variant_false_for_field_starting_with_underscore_then_alpha() {
        // "_foo" — digits check fails
        let fields = vec![make_field("_foo", TypeRef::String)];
        assert!(!is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_variant_three_positional_fields() {
        let fields = vec![
            make_field("_0", TypeRef::String),
            make_field("_1", TypeRef::Primitive(PrimitiveType::I32)),
            make_field("_2", TypeRef::Primitive(PrimitiveType::F64)),
        ];
        assert!(is_tuple_variant(&fields));
    }

    #[test]
    fn test_is_tuple_type_name_empty_string_is_false() {
        assert!(!helpers::is_tuple_type_name(""));
    }

    #[test]
    fn test_is_tuple_type_name_space_is_false() {
        assert!(!helpers::is_tuple_type_name("String"));
    }

    // -----------------------------------------------------------------------
    // core_type_path edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_core_type_path_with_hyphen_and_double_colon() {
        // Hyphens in path should be replaced; path still contains "::" so used verbatim
        let typ = make_type("Config", "my-crate::module::Config", vec![]);
        assert_eq!(core_type_path(&typ, "my_crate"), "my_crate::module::Config");
    }

    #[test]
    fn test_core_type_path_rust_path_matches_core_import_prefix() {
        // Path already starts with core_import → used verbatim
        let typ = make_type("Config", "my_crate::Config", vec![]);
        assert_eq!(core_type_path(&typ, "my_crate"), "my_crate::Config");
    }

    // -----------------------------------------------------------------------
    // build_type_path_map — multiple types, hyphens, enums
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_type_path_map_multiple_types() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![
                make_type("Config", "my_crate::Config", vec![]),
                make_type("Result", "my_crate::types::Result", vec![]),
            ],
            functions: vec![],
            enums: vec![
                make_enum("Mode", "my_crate::Mode", &["A"]),
                make_enum("Status", "Status", &["Ok"]),
            ],
            errors: vec![],
        };
        let map = build_type_path_map(&surface, "my_crate");
        assert_eq!(map.get("Config").map(String::as_str), Some("my_crate::Config"));
        assert_eq!(map.get("Result").map(String::as_str), Some("my_crate::types::Result"));
        assert_eq!(map.get("Mode").map(String::as_str), Some("my_crate::Mode"));
        // "Status" path has no "::" and doesn't start with core_import → prefixed
        assert_eq!(map.get("Status").map(String::as_str), Some("my_crate::Status"));
    }

    #[test]
    fn test_build_type_path_map_normalizes_hyphens() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![make_type("Config", "my-crate::Config", vec![])],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let map = build_type_path_map(&surface, "my_crate");
        assert_eq!(map.get("Config").map(String::as_str), Some("my_crate::Config"));
    }

    #[test]
    fn test_build_type_path_map_empty_surface() {
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let map = build_type_path_map(&surface, "c");
        assert!(map.is_empty());
    }

    // -----------------------------------------------------------------------
    // Edge cases: empty fields, single-field structs, all-optional fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_from_binding_to_core_empty_fields() {
        let typ = make_type("Empty", "c::Empty", vec![]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("impl From<Empty> for c::Empty"));
        assert!(result.contains("Self {"));
    }

    #[test]
    fn test_gen_from_core_to_binding_empty_fields() {
        let typ = make_type("Empty", "c::Empty", vec![]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("impl From<c::Empty> for Empty"));
        assert!(result.contains("Self {"));
    }

    #[test]
    fn test_gen_from_binding_to_core_all_optional_fields() {
        let typ = make_type(
            "Config",
            "c::Config",
            vec![
                make_opt_field("name", TypeRef::String),
                make_opt_field("count", TypeRef::Primitive(PrimitiveType::I32)),
            ],
        );
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("name: val.name"));
        assert!(result.contains("count: val.count"));
    }

    #[test]
    fn test_gen_from_binding_to_core_single_string_field() {
        let typ = make_type("S", "c::S", vec![make_field("value", TypeRef::String)]);
        let result = gen_from_binding_to_core(&typ, "c");
        assert!(result.contains("value: val.value"));
    }

    #[test]
    fn test_gen_from_core_to_binding_single_optional_named_field() {
        let field = make_opt_field("inner", TypeRef::Named("Inner".into()));
        let typ = make_type("Wrapper", "c::Wrapper", vec![field]);
        let result = gen_from_core_to_binding(&typ, "c", &no_opaques());
        assert!(result.contains("inner: val.inner.map(Into::into)"));
    }

    // -----------------------------------------------------------------------
    // binding_to_core_match_arm_ext_cfg — config-aware match arms
    // -----------------------------------------------------------------------

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_unit_variant() {
        let config = ConversionConfig::default();
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Foo", &[], false, &config);
        assert_eq!(result, "MyEnum::Foo => Self::Foo,");
    }

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_no_binding_data_named_fields() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("value", TypeRef::String)];
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Bar", &fields, false, &config);
        assert!(result.contains("value: Default::default()"));
    }

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_no_binding_data_tuple_fields() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("_0", TypeRef::String)];
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Bar", &fields, false, &config);
        assert!(result.contains("Default::default()"));
        assert!(result.contains("Self::Bar("));
    }

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_with_binding_data_named() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("value", TypeRef::Named("Inner".into()))];
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Bar", &fields, true, &config);
        assert!(result.contains("value: value.into()"));
    }

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_with_binding_data_tuple() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("_0", TypeRef::Named("Inner".into()))];
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Bar", &fields, true, &config);
        assert!(result.contains("_0.into()"));
    }

    #[test]
    fn test_binding_to_core_match_arm_ext_cfg_cast_u64_field() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let fields = vec![make_field("count", TypeRef::Primitive(PrimitiveType::U64))];
        let result = helpers::binding_to_core_match_arm_ext_cfg("MyEnum", "Bar", &fields, true, &config);
        assert!(result.contains("as u64"));
    }

    // -----------------------------------------------------------------------
    // core_to_binding_match_arm_ext_cfg — config-aware match arms
    // -----------------------------------------------------------------------

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_unit_variant() {
        let config = ConversionConfig::default();
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Foo", &[], false, &config);
        assert_eq!(result, "CoreEnum::Foo => Self::Foo,");
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_no_binding_data_named() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("x", TypeRef::Primitive(PrimitiveType::I32))];
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Foo", &fields, false, &config);
        assert!(result.contains("{ .. }"));
        assert!(result.contains("Self::Foo"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_no_binding_data_tuple() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("_0", TypeRef::Primitive(PrimitiveType::I32))];
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Foo", &fields, false, &config);
        assert!(result.contains("(..)"));
        assert!(result.contains("Self::Foo"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_with_binding_data_named_fields() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("value", TypeRef::Named("Inner".into()))];
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Foo", &fields, true, &config);
        assert!(result.contains("value: value.into()"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_with_binding_data_tuple() {
        let config = ConversionConfig::default();
        let fields = vec![make_field("_0", TypeRef::Named("Inner".into()))];
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Foo", &fields, true, &config);
        assert!(result.contains("_0: _0.into()"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_cfg_cast_u64_field() {
        let config = ConversionConfig {
            cast_large_ints_to_i64: true,
            ..Default::default()
        };
        let fields = vec![make_field("count", TypeRef::Primitive(PrimitiveType::U64))];
        let result = helpers::core_to_binding_match_arm_ext_cfg("CoreEnum", "Bar", &fields, true, &config);
        assert!(result.contains("as i64"));
    }

    // -----------------------------------------------------------------------
    // core_to_binding_match_arm_ext with binding_has_data=true
    // -----------------------------------------------------------------------

    #[test]
    fn test_core_to_binding_match_arm_ext_binding_has_data_named_fields() {
        let fields = vec![make_field("value", TypeRef::Named("Inner".into()))];
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &fields, true);
        assert!(result.contains("value: value.into()"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_binding_has_data_tuple_fields() {
        let fields = vec![make_field("_0", TypeRef::Named("Inner".into()))];
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &fields, true);
        assert!(result.contains("_0: _0.into()"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_binding_has_data_plain_field() {
        let fields = vec![make_field("x", TypeRef::Primitive(PrimitiveType::I32))];
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &fields, true);
        assert!(result.contains("x: x"));
    }

    #[test]
    fn test_core_to_binding_match_arm_ext_binding_has_data_sanitized_field() {
        let mut field = make_field("complex", TypeRef::String);
        field.sanitized = true;
        let result = helpers::core_to_binding_match_arm_ext("CoreEnum", "Bar", &[field], true);
        assert!(result.contains("serde_json::to_string("));
    }

    // -----------------------------------------------------------------------
    // input_type_names — method params and transitive closure
    // -----------------------------------------------------------------------

    #[test]
    fn test_input_type_names_from_method_params() {
        let surface = ApiSurface {
            crate_name: "my_crate".into(),
            version: "1.0.0".into(),
            types: vec![TypeDef {
                name: "Client".into(),
                rust_path: "my_crate::Client".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "process".into(),
                    params: vec![ParamDef {
                        name: "config".into(),
                        ty: TypeRef::Named("Config".into()),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                    }],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: None,
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                }],
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
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let names = input_type_names(&surface);
        assert!(names.contains("Config"));
    }

    #[test]
    fn test_input_type_names_transitive_closure() {
        // Config has a field of type Backend — Backend should also be in input_type_names
        let config_type = make_type(
            "Config",
            "c::Config",
            vec![make_field("backend", TypeRef::Named("Backend".into()))],
        );
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![config_type],
            functions: vec![FunctionDef {
                name: "run".into(),
                rust_path: "c::run".into(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "config".into(),
                    ty: TypeRef::Named("Config".into()),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                }],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        };
        let names = input_type_names(&surface);
        assert!(names.contains("Config"));
        assert!(names.contains("Backend"));
    }

    // -----------------------------------------------------------------------
    // convertible_types — sanitized fields with/without has_default
    // -----------------------------------------------------------------------

    #[test]
    fn test_convertible_types_sanitized_field_with_has_default() {
        let mut field = make_field("complex", TypeRef::String);
        field.sanitized = true;
        let mut typ = make_type("Config", "c::Config", vec![field]);
        typ.has_default = true;
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![typ],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        // String has Default::default() — convertible
        let result = convertible_types(&surface);
        assert!(result.contains("Config"));
    }

    #[test]
    fn test_convertible_types_opaque_type_excluded() {
        let mut typ = make_type("Client", "c::Client", vec![]);
        typ.is_opaque = true;
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![typ],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        // Opaque types are not in the candidate set initially
        let result = convertible_types(&surface);
        assert!(!result.contains("Client"));
    }

    #[test]
    fn test_convertible_types_type_with_named_field_in_surface() {
        // Both Config (with Backend field) and Backend present — both convertible
        let config_field = make_field("backend", TypeRef::Named("Backend".into()));
        let config = make_type("Config", "c::Config", vec![config_field]);
        let backend = make_type("Backend", "c::Backend", vec![]);
        let surface = ApiSurface {
            crate_name: "c".into(),
            version: "1.0".into(),
            types: vec![config, backend],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let result = convertible_types(&surface);
        assert!(result.contains("Config"));
        assert!(result.contains("Backend"));
    }

    // -----------------------------------------------------------------------
    // core_enum_path edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_core_enum_path_with_hyphen_normalization() {
        let e = make_enum("Status", "my-crate::Status", &[]);
        assert_eq!(core_enum_path(&e, "my_crate"), "my_crate::Status");
    }

    #[test]
    fn test_core_enum_path_already_starts_with_core_import() {
        // When path already starts with core_import, use verbatim
        let e = make_enum("Mode", "my_crate::inner::Mode", &[]);
        assert_eq!(core_enum_path(&e, "my_crate"), "my_crate::inner::Mode");
    }

    // -----------------------------------------------------------------------
    // needs_i64_cast / core_prim_str / binding_prim_str (helper coverage)
    // -----------------------------------------------------------------------

    #[test]
    fn test_needs_i64_cast_true_for_large_ints() {
        use super::helpers::*;
        assert!(needs_i64_cast(&PrimitiveType::U64));
        assert!(needs_i64_cast(&PrimitiveType::Usize));
        assert!(needs_i64_cast(&PrimitiveType::Isize));
    }

    #[test]
    fn test_needs_i64_cast_false_for_small_ints() {
        use super::helpers::*;
        assert!(!needs_i64_cast(&PrimitiveType::I32));
        assert!(!needs_i64_cast(&PrimitiveType::U32));
        assert!(!needs_i64_cast(&PrimitiveType::F64));
    }

    #[test]
    fn test_core_prim_str_all_variants() {
        use super::helpers::core_prim_str;
        assert_eq!(core_prim_str(&PrimitiveType::U64), "u64");
        assert_eq!(core_prim_str(&PrimitiveType::Usize), "usize");
        assert_eq!(core_prim_str(&PrimitiveType::Isize), "isize");
        assert_eq!(core_prim_str(&PrimitiveType::F32), "f32");
        assert_eq!(core_prim_str(&PrimitiveType::Bool), "bool");
        assert_eq!(core_prim_str(&PrimitiveType::U8), "u8");
        assert_eq!(core_prim_str(&PrimitiveType::U16), "u16");
        assert_eq!(core_prim_str(&PrimitiveType::U32), "u32");
        assert_eq!(core_prim_str(&PrimitiveType::I8), "i8");
        assert_eq!(core_prim_str(&PrimitiveType::I16), "i16");
        assert_eq!(core_prim_str(&PrimitiveType::I32), "i32");
        assert_eq!(core_prim_str(&PrimitiveType::I64), "i64");
        assert_eq!(core_prim_str(&PrimitiveType::F64), "f64");
    }

    #[test]
    fn test_binding_prim_str_large_ints_map_to_i64() {
        use super::helpers::binding_prim_str;
        assert_eq!(binding_prim_str(&PrimitiveType::U64), "i64");
        assert_eq!(binding_prim_str(&PrimitiveType::Usize), "i64");
        assert_eq!(binding_prim_str(&PrimitiveType::Isize), "i64");
    }

    #[test]
    fn test_binding_prim_str_small_ints_map_to_i32() {
        use super::helpers::binding_prim_str;
        assert_eq!(binding_prim_str(&PrimitiveType::U8), "i32");
        assert_eq!(binding_prim_str(&PrimitiveType::U16), "i32");
        assert_eq!(binding_prim_str(&PrimitiveType::U32), "i32");
        assert_eq!(binding_prim_str(&PrimitiveType::I8), "i32");
        assert_eq!(binding_prim_str(&PrimitiveType::I16), "i32");
        assert_eq!(binding_prim_str(&PrimitiveType::I32), "i32");
    }

    #[test]
    fn test_binding_prim_str_float_and_i64() {
        use super::helpers::binding_prim_str;
        assert_eq!(binding_prim_str(&PrimitiveType::F32), "f64");
        assert_eq!(binding_prim_str(&PrimitiveType::F64), "f64");
        assert_eq!(binding_prim_str(&PrimitiveType::I64), "i64");
        assert_eq!(binding_prim_str(&PrimitiveType::Bool), "bool");
    }
}
