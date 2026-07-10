use super::*;

#[test]
fn test_default_value_bool_true_python() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "True");
}

#[test]
fn test_default_value_bool_false_go() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "go"), "false");
}

#[test]
fn test_default_value_string_literal() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "\"custom\"");
}

#[test]
fn test_default_value_float_literal() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "r"), "FALSE");
}

#[test]
fn test_default_value_string_literal_rust() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "rust"), "\"hello\".to_string()");
}

#[test]
fn test_default_value_string_literal_escapes_quotes() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "\"say \\\"hi\\\"\"");
}

#[test]
fn test_default_value_float_literal_whole_number() {
    let field = FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(default_value_for_field(&field, "python"), "0.0");
}

#[test]
fn test_default_value_empty_string_type() {
    let field = FieldDef {
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
