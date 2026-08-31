use super::*;

#[test]
fn test_default_value_bool_true_python() {
    let field = FieldDef {
        version: Default::default(),
        name: "enabled".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::BoolLiteral(true)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "True");
}

#[test]
fn test_default_value_bool_false_go() {
    let field = FieldDef {
        version: Default::default(),
        name: "enabled".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::BoolLiteral(false)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "go"), "false");
}

#[test]
fn test_default_value_string_literal() {
    let field = FieldDef {
        version: Default::default(),
        name: "name".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::StringLiteral("hello".to_string())),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "\"hello\"");
    assert_eq!(default_value_for_field(&field, "java"), "\"hello\"");
}

#[test]
fn test_default_value_int_literal() {
    let field = FieldDef {
        version: Default::default(),
        name: "timeout".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U64),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::IntLiteral(42)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let result = default_value_for_field(&field, "python");
    assert_eq!(result, "42");
}

#[test]
fn test_default_value_none() {
    let field = FieldDef {
        version: Default::default(),
        name: "maybe".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::None),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "None");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
    assert_eq!(default_value_for_field(&field, "java"), "null");
    assert_eq!(default_value_for_field(&field, "csharp"), "null");
}

#[test]
fn test_default_value_fallback_string() {
    let field = FieldDef {
        version: Default::default(),
        name: "name".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: Some("\"custom\"".to_string()),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "\"custom\"");
}

#[test]
fn test_default_value_float_literal() {
    let field = FieldDef {
        version: Default::default(),
        name: "ratio".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::F64),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::FloatLiteral(1.5)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let result = default_value_for_field(&field, "python");
    assert!(result.contains("1.5"));
}

#[test]
fn test_default_value_no_typed_no_default() {
    let field = FieldDef {
        version: Default::default(),
        name: "count".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "0");
    assert_eq!(default_value_for_field(&field, "go"), "0");
}
#[test]
fn test_default_value_bool_literal_ruby() {
    let field = FieldDef {
        version: Default::default(),
        name: "flag".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::BoolLiteral(true)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "ruby"), "true");
    assert_eq!(default_value_for_field(&field, "php"), "true");
    assert_eq!(default_value_for_field(&field, "csharp"), "true");
    assert_eq!(default_value_for_field(&field, "java"), "true");
    assert_eq!(default_value_for_field(&field, "rust"), "true");
}

#[test]
fn test_default_value_bool_literal_r() {
    let field = FieldDef {
        version: Default::default(),
        name: "flag".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::BoolLiteral(false)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "r"), "FALSE");
}

#[test]
fn test_default_value_string_literal_rust() {
    let field = FieldDef {
        version: Default::default(),
        name: "label".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::StringLiteral("hello".to_string())),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "rust"), "\"hello\".to_string()");
}

#[test]
fn test_default_value_string_literal_escapes_quotes() {
    let field = FieldDef {
        version: Default::default(),
        name: "label".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::StringLiteral("say \"hi\"".to_string())),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "\"say \\\"hi\\\"\"");
}

#[test]
fn test_default_value_float_literal_whole_number() {
    let field = FieldDef {
        version: Default::default(),
        name: "scale".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::F32),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::FloatLiteral(2.0)),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let result = default_value_for_field(&field, "python");
    assert!(result.contains('.'), "whole-number float should contain '.': {result}");
}

#[test]
fn test_default_value_enum_variant_per_language() {
    let field = FieldDef {
        version: Default::default(),
        name: "format".to_string(),
        ty: TypeRef::Named("OutputFormat".to_string()),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::EnumVariant("JsonOutput".to_string())),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "OutputFormat.JSON_OUTPUT");
    assert_eq!(default_value_for_field(&field, "ruby"), "OutputFormat::JsonOutput");
    assert_eq!(default_value_for_field(&field, "go"), "OutputFormatJsonOutput");
    assert_eq!(default_value_for_field(&field, "java"), "OutputFormat.JSON_OUTPUT");
    assert_eq!(default_value_for_field(&field, "csharp"), "OutputFormat.JsonOutput");
    assert_eq!(default_value_for_field(&field, "php"), "OutputFormat::JsonOutput");
    assert_eq!(default_value_for_field(&field, "r"), "OutputFormat$JsonOutput");
    assert_eq!(default_value_for_field(&field, "rust"), "OutputFormat::JsonOutput");
}

#[test]
fn test_default_value_empty_vec_per_language() {
    let field = FieldDef {
        version: Default::default(),
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "[]");
    assert_eq!(default_value_for_field(&field, "ruby"), "[]");
    assert_eq!(default_value_for_field(&field, "csharp"), "[]");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
    assert_eq!(default_value_for_field(&field, "java"), "List.of()");
    assert_eq!(default_value_for_field(&field, "php"), "[]");
    assert_eq!(default_value_for_field(&field, "r"), "c()");
    assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
}

#[test]
fn test_default_value_empty_map_per_language() {
    let field = FieldDef {
        version: Default::default(),
        name: "meta".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "{}");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
    assert_eq!(default_value_for_field(&field, "java"), "Map.of()");
    assert_eq!(default_value_for_field(&field, "rust"), "Default::default()");
}

#[test]
fn test_default_value_empty_bool_primitive() {
    let field = FieldDef {
        version: Default::default(),
        name: "flag".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "False");
    assert_eq!(default_value_for_field(&field, "ruby"), "false");
    assert_eq!(default_value_for_field(&field, "go"), "false");
}

#[test]
fn test_default_value_empty_float_primitive() {
    let field = FieldDef {
        version: Default::default(),
        name: "ratio".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::F64),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "0.0");
}

#[test]
fn test_default_value_empty_string_type() {
    let field = FieldDef {
        version: Default::default(),
        name: "label".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "rust"), "String::new()");
    assert_eq!(default_value_for_field(&field, "python"), "\"\"");
}

#[test]
fn test_default_value_empty_bytes_type() {
    let field = FieldDef {
        version: Default::default(),
        name: "data".to_string(),
        ty: TypeRef::Bytes,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "b\"\"");
    assert_eq!(default_value_for_field(&field, "go"), "[]byte{}");
    assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
}

#[test]
fn test_default_value_empty_json_type() {
    let field = FieldDef {
        version: Default::default(),
        name: "payload".to_string(),
        ty: TypeRef::Json,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "{}");
    assert_eq!(default_value_for_field(&field, "ruby"), "{}");
    assert_eq!(default_value_for_field(&field, "go"), "json.RawMessage(nil)");
    assert_eq!(default_value_for_field(&field, "r"), "list()");
    assert_eq!(default_value_for_field(&field, "rust"), "serde_json::json!({})");
}

#[test]
fn test_default_value_none_ruby_php_r() {
    let field = FieldDef {
        version: Default::default(),
        name: "maybe".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::None),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "ruby"), "nil");
    assert_eq!(default_value_for_field(&field, "php"), "null");
    assert_eq!(default_value_for_field(&field, "r"), "NULL");
    assert_eq!(default_value_for_field(&field, "rust"), "None");
}

#[test]
fn test_default_value_fallback_bool_all_languages() {
    let field = make_field("flag", TypeRef::Primitive(PrimitiveType::Bool));
    assert_eq!(default_value_for_field(&field, "python"), "False");
    assert_eq!(default_value_for_field(&field, "ruby"), "false");
    assert_eq!(default_value_for_field(&field, "csharp"), "false");
    assert_eq!(default_value_for_field(&field, "java"), "false");
    assert_eq!(default_value_for_field(&field, "php"), "false");
    assert_eq!(default_value_for_field(&field, "r"), "FALSE");
    assert_eq!(default_value_for_field(&field, "rust"), "false");
}

#[test]
fn test_default_value_fallback_float() {
    let field = make_field("ratio", TypeRef::Primitive(PrimitiveType::F64));
    assert_eq!(default_value_for_field(&field, "python"), "0.0");
    assert_eq!(default_value_for_field(&field, "rust"), "0.0");
}

#[test]
fn test_default_value_fallback_string_all_languages() {
    let field = make_field("name", TypeRef::String);
    assert_eq!(default_value_for_field(&field, "python"), "\"\"");
    assert_eq!(default_value_for_field(&field, "ruby"), "\"\"");
    assert_eq!(default_value_for_field(&field, "go"), "\"\"");
    assert_eq!(default_value_for_field(&field, "java"), "\"\"");
    assert_eq!(default_value_for_field(&field, "csharp"), "\"\"");
    assert_eq!(default_value_for_field(&field, "php"), "\"\"");
    assert_eq!(default_value_for_field(&field, "r"), "\"\"");
    assert_eq!(default_value_for_field(&field, "rust"), "String::new()");
}

#[test]
fn test_default_value_fallback_bytes_all_languages() {
    let field = make_field("data", TypeRef::Bytes);
    assert_eq!(default_value_for_field(&field, "python"), "b\"\"");
    assert_eq!(default_value_for_field(&field, "ruby"), "\"\"");
    assert_eq!(default_value_for_field(&field, "go"), "[]byte{}");
    assert_eq!(default_value_for_field(&field, "java"), "new byte[]{}");
    assert_eq!(default_value_for_field(&field, "csharp"), "new byte[]{}");
    assert_eq!(default_value_for_field(&field, "php"), "\"\"");
    assert_eq!(default_value_for_field(&field, "r"), "raw()");
    assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
}

#[test]
fn test_default_value_fallback_optional() {
    let field = make_field("maybe", TypeRef::Optional(Box::new(TypeRef::String)));
    assert_eq!(default_value_for_field(&field, "python"), "None");
    assert_eq!(default_value_for_field(&field, "ruby"), "nil");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
    assert_eq!(default_value_for_field(&field, "java"), "null");
    assert_eq!(default_value_for_field(&field, "csharp"), "null");
    assert_eq!(default_value_for_field(&field, "php"), "null");
    assert_eq!(default_value_for_field(&field, "r"), "NULL");
    assert_eq!(default_value_for_field(&field, "rust"), "None");
}

#[test]
fn test_default_value_fallback_vec_all_languages() {
    let field = make_field("items", TypeRef::Vec(Box::new(TypeRef::String)));
    assert_eq!(default_value_for_field(&field, "python"), "[]");
    assert_eq!(default_value_for_field(&field, "ruby"), "[]");
    assert_eq!(default_value_for_field(&field, "go"), "[]interface{}{}");
    assert_eq!(default_value_for_field(&field, "java"), "new java.util.ArrayList<>()");
    assert_eq!(default_value_for_field(&field, "csharp"), "[]");
    assert_eq!(default_value_for_field(&field, "php"), "[]");
    assert_eq!(default_value_for_field(&field, "r"), "c()");
    assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
}

#[test]
fn test_default_value_fallback_map_all_languages() {
    let field = make_field(
        "meta",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    );
    assert_eq!(default_value_for_field(&field, "python"), "{}");
    assert_eq!(default_value_for_field(&field, "ruby"), "{}");
    assert_eq!(default_value_for_field(&field, "go"), "make(map[string]interface{})");
    assert_eq!(default_value_for_field(&field, "java"), "new java.util.HashMap<>()");
    assert_eq!(
        default_value_for_field(&field, "csharp"),
        "new Dictionary<string, object>()"
    );
    assert_eq!(default_value_for_field(&field, "php"), "[]");
    assert_eq!(default_value_for_field(&field, "r"), "list()");
    assert_eq!(
        default_value_for_field(&field, "rust"),
        "std::collections::HashMap::new()"
    );
}

#[test]
fn test_default_value_fallback_json_all_languages() {
    let field = make_field("payload", TypeRef::Json);
    assert_eq!(default_value_for_field(&field, "python"), "{}");
    assert_eq!(default_value_for_field(&field, "ruby"), "{}");
    assert_eq!(default_value_for_field(&field, "go"), "json.RawMessage(nil)");
    assert_eq!(default_value_for_field(&field, "r"), "list()");
    assert_eq!(default_value_for_field(&field, "rust"), "serde_json::json!({})");
}

#[test]
fn test_default_value_fallback_named_type() {
    let field = make_field("config", TypeRef::Named("MyConfig".to_string()));
    assert_eq!(default_value_for_field(&field, "rust"), "MyConfig::default()");
    assert_eq!(default_value_for_field(&field, "python"), "None");
    assert_eq!(default_value_for_field(&field, "ruby"), "nil");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
    assert_eq!(default_value_for_field(&field, "java"), "null");
    assert_eq!(default_value_for_field(&field, "csharp"), "null");
    assert_eq!(default_value_for_field(&field, "php"), "null");
    assert_eq!(default_value_for_field(&field, "r"), "NULL");
}

#[test]
fn test_default_value_fallback_duration() {
    let field = make_field("timeout", TypeRef::Duration);
    assert_eq!(default_value_for_field(&field, "python"), "None");
    assert_eq!(default_value_for_field(&field, "rust"), "Default::default()");
}

#[test]
fn test_serde_default_marker_falls_through_to_type_zero() {
    // The extractor encodes `#[serde(default = "path")]` as a `serde(default = "...")`
    let string_field = FieldDef {
        default: Some("serde(default = \"crate::serde_defaults::default_jwt_algorithm\")".to_string()),
        ..make_field("algorithm", TypeRef::String)
    };
    assert_eq!(default_value_for_field(&string_field, "rust"), "String::new()");
    assert_eq!(default_value_for_field(&string_field, "ruby"), "\"\"");
    assert_eq!(default_value_for_field(&string_field, "python"), "\"\"");
    assert_eq!(default_value_for_field(&string_field, "java"), "\"\"");

    let bool_field = FieldDef {
        default: Some("serde(default = \"crate::serde_defaults::default_true\")".to_string()),
        ..make_field("index_file", TypeRef::Primitive(PrimitiveType::Bool))
    };
    assert_eq!(default_value_for_field(&bool_field, "rust"), "false");
    assert_eq!(default_value_for_field(&bool_field, "ruby"), "false");
}

#[test]
fn test_serde_default_bare_placeholder_falls_through_to_type_zero() {
    // The legacy `#[serde(default)]` placeholder must keep falling through too.
    let field = FieldDef {
        default: Some("/* serde(default) */".to_string()),
        ..make_field("name", TypeRef::String)
    };
    assert_eq!(default_value_for_field(&field, "rust"), "String::new()");
    assert_eq!(default_value_for_field(&field, "python"), "\"\"");
}

/// A `#[serde(default = "path")]` function is private to the crate that declares it and
/// is often `#[cfg(feature = "serde")]`-gated, so a generated binding crate cannot call
/// it. Emitting `path()` produced Rust that did not compile (E0425). Every language must
/// fall back to a value it can actually name.
///
/// This exercises `default_value_for_field` directly, i.e. the no-owning-type-context
/// path. Every production "rust"-emitting caller (Magnus, PHP, NAPI, Rustler) instead goes
/// through `default_value_for_field_in_type`, which recovers the real source-crate value —
/// see `function_call_default_delegates_to_source_deserialize_when_type_context_is_known`
/// below.
#[test]
fn serde_default_function_is_never_emitted_as_a_callable_in_generated_rust() {
    let field = FieldDef {
        version: Default::default(),
        name: "row_span".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::FunctionCall("default_span".to_string())),
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    // A caller with no `TypeDef` cannot recover the real value, so it must FAIL rather than
    // emit anything: `default_span()` does not compile from a generated crate, and substituting
    // `Default::default()` compiles while silently shipping `0` where the source crate says `1`.
    // A wrong-but-compiling default is far harder to find than a generation error. ~keep
    let message = default_value_for_field(&field, "rust");
    assert!(
        message.starts_with("compile_error!") && message.contains("default_span") && message.contains("row_span"),
        "the failure must name the uncallable function and the field so the author can act on it: {message}"
    );

    for language in ["python", "ruby", "go"] {
        let rendered = default_value_for_field(&field, language);
        assert!(
            !rendered.contains("default_span"),
            "{language} must not reference the source-crate default fn: {rendered}"
        );
    }
}

/// Shape of `html-to-markdown`'s `GridCell`: no `#[derive(Default)]`, but
/// `Serialize`/`Deserialize`, with `row_span`/`col_span` behind
/// `#[serde(default = "default_span")]` (private, `#[cfg(feature = "serde")]`-gated) and
/// `content`/`row`/`col` genuinely required. `is_header` carries a bare `#[serde(default)]`.
///
/// `row_span`/`col_span` carry both `default` (the raw attribute text) and `typed_default`
/// (`FunctionCall`), mirroring `extract::extractor::helpers::fields::extract_field`, which
/// always sets both together from the same `#[serde(default = "path")]` attribute — never only
/// one (see `codegen::config_gen::tests::derive_default_probe` and
/// `extract::extractor::tests::defaults::derived::
/// derive_default_seeds_empty_over_a_genuine_field_level_serde_default` for the production
/// proof). A fixture carrying `typed_default` alone models a state real extraction never
/// produces and would make `has_own_default` skip these fields for the wrong reason. ~keep
fn grid_cell_type() -> TypeDef {
    let content = FieldDef {
        ..make_field("content", TypeRef::String)
    };
    let row = FieldDef {
        ..make_field("row", TypeRef::Primitive(PrimitiveType::U32))
    };
    let col = FieldDef {
        ..make_field("col", TypeRef::Primitive(PrimitiveType::U32))
    };
    let row_span = FieldDef {
        default: Some("serde(default = \"default_span\")".to_string()),
        typed_default: Some(DefaultValue::FunctionCall("default_span".to_string())),
        ..make_field("row_span", TypeRef::Primitive(PrimitiveType::U32))
    };
    let col_span = FieldDef {
        default: Some("serde(default = \"default_span\")".to_string()),
        typed_default: Some(DefaultValue::FunctionCall("default_span".to_string())),
        ..make_field("col_span", TypeRef::Primitive(PrimitiveType::U32))
    };
    let is_header = FieldDef {
        default: Some("/* serde(default) */".to_string()),
        ..make_field("is_header", TypeRef::Primitive(PrimitiveType::Bool))
    };

    // `TypeDef` derives `Default`; every field left out below is at its Default::default()
    // value (false/None/empty), same spread pattern as
    // `codegen::config_gen::tests::derive_default_probe::derived_config_type`. ~keep
    TypeDef {
        name: "GridCell".to_string(),
        rust_path: "html_to_markdown::types::tables::GridCell".to_string(),
        fields: vec![content, row, col, row_span, col_span, is_header],
        is_clone: true,
        has_serde: true,
        ..Default::default()
    }
}

/// `default_value_for_field_in_type` is what every "rust"-emitting caller (Magnus, PHP,
/// NAPI, Rustler) must use. Given the owning `TypeDef`, it must recover `default_span()`'s
/// real value (1) instead of silently substituting `u32::default()` (0) — the exact
/// regression the reviewer rejected in the `Default::default()` fallback.
#[test]
fn function_call_default_delegates_to_source_deserialize_when_type_context_is_known() {
    let typ = grid_cell_type();
    let row_span = typ.fields.iter().find(|f| f.name == "row_span").unwrap();

    let rendered = default_value_for_field_in_type(row_span, "rust", &typ);

    assert_eq!(
        rendered,
        "serde_json::from_str::<html_to_markdown::types::tables::GridCell>(r#\"{\"content\":\"\",\"row\":0,\"col\":0}\"#)\
         .expect(\"alef-generated default JSON for `GridCell` failed to deserialize\").row_span",
        "must build a JSON stub of only the genuinely-required siblings and project the real field"
    );
    assert!(
        !rendered.contains("default_span"),
        "the uncallable source function must never leak into generated Rust: {rendered}"
    );
    assert!(
        !rendered.contains("Default::default()"),
        "a type-context-aware caller must not silently substitute the field type's zero value: {rendered}"
    );
}

/// Proves the empty-field-deserialize technique itself is sound, independent of codegen:
/// omitting a `#[serde(default = "path")]` field from the JSON and deserializing through
/// the real type's `Deserialize` impl recovers `path()`'s actual result, not the field
/// type's `Default::default()`. This is the exact discrepancy (`default_span() == 1` vs.
/// `u32::default() == 0`) that made the rejected commit's substitution wrong.
#[test]
fn empty_field_deserialize_recovers_the_real_serde_default_not_the_type_zero_value() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct RealGridCell {
        content: String,
        row: u32,
        col: u32,
        #[serde(default = "real_default_span")]
        row_span: u32,
        #[serde(default = "real_default_span")]
        col_span: u32,
        #[serde(default)]
        is_header: bool,
    }
    fn real_default_span() -> u32 {
        1
    }

    let recovered: RealGridCell = serde_json::from_str(r#"{"content":"","row":0,"col":0}"#)
        .expect("the JSON stub built from only the required siblings must deserialize");

    assert_eq!(
        recovered.row_span, 1,
        "must recover the source crate's real default, not the field type's zero value"
    );
    assert_ne!(
        recovered.row_span,
        u32::default(),
        "u32::default() (0) is exactly the wrong value the rejected commit silently substituted"
    );
}

/// When a required sibling field's type has no safe JSON placeholder (a nested named
/// type here), delegation is not possible, and generation must fail loudly rather than
/// guess. The message must name the crate, the type, the field being solved for, and the
/// uncallable source function, so the failure is actionable.
#[test]
fn contextual_failure_names_crate_type_field_and_uncallable_function() {
    let mut typ = grid_cell_type();
    typ.fields.push(FieldDef {
        ..make_field("owner", TypeRef::Named("Author".to_string()))
    });
    let row_span = typ.fields.iter().find(|f| f.name == "row_span").unwrap().clone();

    let message = default_value_for_field_in_type(&row_span, "rust", &typ);

    assert!(message.starts_with("compile_error!"));

    for needle in ["html_to_markdown", "GridCell", "row_span", "default_span"] {
        assert!(
            message.contains(needle),
            "contextual failure must name `{needle}`: {message}"
        );
    }
}

#[test]
fn public_associated_default_bypasses_structural_deserialize_placeholders() {
    let mut typ = grid_cell_type();
    typ.name = "ClientConfig".to_string();
    typ.rust_path = "sample_core::ClientConfig".to_string();
    typ.fields.push(FieldDef {
        ..make_field("nested", TypeRef::Named("RequiredSettings".to_string()))
    });
    let field = FieldDef {
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "sample_core::NetworkPolicy::from_environment".to_string(),
        )),
        ..make_field("policy", TypeRef::Named("NetworkPolicy".to_string()))
    };

    assert_eq!(
        default_value_for_field_in_type(&field, "rust", &typ),
        "sample_core::NetworkPolicy::from_environment()"
    );
}

/// The bug this fix targets: alef could not constant-fold `impl Default`'s body
/// (`Unresolved`), and the shared renderer used to answer that exactly like `Empty` — the
/// field type's own zero — underneath a doc comment quoting the real (unreadable) default.
/// `Unresolved` only exists because a real `fn default()` was found and could not be read
/// through, so `TypeDef::has_default` is guaranteed `true`; that impl is real, compiled Rust
/// the generated crate can call directly, so `default_value_for_field_in_type` must recover
/// the actual value the same way it already does for `TupleVariant`/`StructVariant`, rather
/// than merely refusing to guess.
#[test]
fn unresolved_default_recovers_the_real_value_via_the_owning_types_default_impl() {
    let typ = TypeDef {
        has_default: true,
        rust_path: "demo::Settings".to_string(),
        ..make_test_type()
    };
    let field = FieldDef {
        typed_default: Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
        ..make_field("mode", TypeRef::Primitive(PrimitiveType::U64))
    };

    let rendered = default_value_for_field_in_type(&field, "rust", &typ);

    assert_eq!(
        rendered, "<demo::Settings as ::core::default::Default>::default().mode",
        "must read the real default back off the owning type's own `Default` impl"
    );
    assert!(
        !rendered.contains("Default::default()") && !rendered.trim_start().starts_with('0'),
        "must not substitute the field type's own zero: {rendered}"
    );
}

/// When the owning type has no `Default` impl to read back from — `has_default: false`, the
/// mark-unresolved path can only be reached via a nested/inherited context, but the guard
/// itself must not assume one exists — generation must fail loudly rather than guess. The
/// message must name the crate, the type, and the field, the same contract
/// `contextual_failure_names_crate_type_field_and_uncallable_function` pins for `FunctionCall`.
#[test]
fn unresolved_default_without_a_default_impl_to_read_back_from_fails_loudly() {
    let typ = TypeDef {
        has_default: false,
        rust_path: "demo::Settings".to_string(),
        ..make_test_type()
    };
    let field = FieldDef {
        typed_default: Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
        ..make_field("mode", TypeRef::Primitive(PrimitiveType::U64))
    };

    let message = default_value_for_field_in_type(&field, "rust", &typ);

    assert!(
        message.starts_with("compile_error!"),
        "must fail rather than guess: {message}"
    );
    for needle in ["demo::Settings", "Settings", "mode"] {
        assert!(
            message.contains(needle),
            "contextual failure must name `{needle}`: {message}"
        );
    }
    assert!(
        !message.contains("Default::default()"),
        "a failed recovery must not fall back to the field type's own zero: {message}"
    );
}

/// `default_value_for_field` (no owning-`TypeDef` context) mirrors the `FunctionCall` contract:
/// `"rust"` fails loudly rather than guess, and every other language answers "no value" instead
/// of the field type's zero. Every production "rust"-emitting caller (Magnus, PHP, NAPI,
/// Rustler) goes through `default_value_for_field_in_type` instead — see
/// `unresolved_default_recovers_the_real_value_via_the_owning_types_default_impl` above — so this
/// exercises only the context-free path directly.
#[test]
fn unresolved_default_without_type_context_never_fabricates_a_zero() {
    let field = FieldDef {
        typed_default: Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
        ..make_field("retries", TypeRef::Primitive(PrimitiveType::U32))
    };

    let message = default_value_for_field(&field, "rust");
    assert!(
        message.starts_with("compile_error!") && message.contains("retries"),
        "must fail rather than guess, naming the field: {message}"
    );

    assert_eq!(default_value_for_field(&field, "python"), "None");
    assert_eq!(default_value_for_field(&field, "ruby"), "nil");
    assert_eq!(default_value_for_field(&field, "go"), "nil");
}

/// Negative control: `Empty` really does mean "the default IS the type's own zero", so it must
/// still render the type-zero table `Unresolved` no longer shares. Without this, a fix that
/// suppressed every default (rather than only `Unresolved`) would pass the positive tests above
/// while silently dropping a legitimate one.
#[test]
fn empty_default_still_renders_the_type_zero_table_go() {
    let field = FieldDef {
        typed_default: Some(DefaultValue::Empty),
        ..make_field("retries", TypeRef::Primitive(PrimitiveType::U32))
    };

    assert_eq!(default_value_for_field(&field, "go"), "0");
}

/// Regression for alef#156: a non-empty `Vec<String>` default (`#[serde(default =
/// "default_tags")]` folding to `vec!["noscript"]` in the source crate) rendered its elements as
/// bare string literals for `"rust"`. A bare literal's element type is `&'static str`, which does
/// not coerce to `Vec<String>` — this shipped as `E0308: mismatched types` in every generated
/// crate that renders its config constructor in real Rust (Magnus/Ruby, Rustler/Elixir, NAPI,
/// PHP), because they all call `default_value_for_field_in_type(field, "rust", typ)`. It killed
/// every `Build Ruby gem` and `Build Elixir NIF` leg of a real release. Every other language's
/// collection literal already accepts a bare literal element, so this only affects `"rust"`.
#[test]
fn list_literal_of_strings_renders_owned_strings_for_rust() {
    let field = FieldDef {
        typed_default: Some(DefaultValue::ListLiteral(vec![
            DefaultValue::StringLiteral("noscript".to_string()),
            DefaultValue::StringLiteral("script".to_string()),
        ])),
        ..make_field("tags", TypeRef::Vec(Box::new(TypeRef::String)))
    };

    assert_eq!(
        default_value_for_field(&field, "rust"),
        r#"vec!["noscript".to_string(), "script".to_string()]"#
    );

    // Every other language's list literal already accepts a bare element and must not gain a
    // spurious conversion.
    assert_eq!(default_value_for_field(&field, "python"), r#"["noscript", "script"]"#);
    assert_eq!(default_value_for_field(&field, "ruby"), r#"["noscript", "script"]"#);
    assert_eq!(default_value_for_field(&field, "csharp"), r#"["noscript", "script"]"#);
    assert_eq!(default_value_for_field(&field, "php"), r#"["noscript", "script"]"#);
    assert_eq!(
        default_value_for_field(&field, "java"),
        r#"List.of("noscript", "script")"#
    );
}

/// The same fixture as `list_literal_of_strings_renders_owned_strings_for_rust`, but proving the
/// rendered expression is real, compiling Rust rather than just the expected string — a
/// hand-typed `assert_eq!` on the output would not have caught alef#156, since the bug was that
/// the emitted text *looked* right (a `vec![...]` of quoted strings) while failing to type-check
/// against `Vec<String>`. This mirrors the exact shape the Magnus/Rustler/NAPI/PHP constructors
/// emit: `<expr>.unwrap_or(<rendered default>)` assigned into a `Vec<String>` field.
#[test]
fn list_literal_of_strings_compiles_against_vec_string() {
    let field = FieldDef {
        typed_default: Some(DefaultValue::ListLiteral(vec![
            DefaultValue::StringLiteral("noscript".to_string()),
            DefaultValue::StringLiteral("script".to_string()),
        ])),
        ..make_field("tags", TypeRef::Vec(Box::new(TypeRef::String)))
    };
    let rendered = default_value_for_field(&field, "rust");

    let source = format!(
        r#"
fn build_tags(candidate: Option<Vec<String>>) -> Vec<String> {{
    candidate.unwrap_or({rendered})
}}

fn main() {{
    let tags = build_tags(None);
    assert_eq!(tags, vec!["noscript".to_string(), "script".to_string()]);
}}
"#
    );

    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("list_literal_of_strings_compiles.rs");
    let binary_path = directory.path().join("list-literal-of-strings-compiles-test");
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "generated Vec<String> default must compile: {}\n---source---\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}
