use super::enums::{gen_data_enum_type, gen_unit_enum_type};
use super::*;
use crate::codegen::naming::apply_serde_rename_all;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: crate::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

#[test]
fn test_is_tuple_field_detects_positional_names() {
    let positional = simple_field("_0", TypeRef::String);
    assert!(is_tuple_field(&positional));
    let named = simple_field("value", TypeRef::String);
    assert!(!is_tuple_field(&named));
}

#[test]
fn test_apply_serde_rename_all_camel_case() {
    assert_eq!(apply_serde_rename_all("my_field", Some("camelCase")), "myField");
    assert_eq!(apply_serde_rename_all("my_field", None), "my_field");
}

#[test]
fn test_gen_unit_enum_type_produces_type_string_and_const_block() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        variants: vec![EnumVariant {
            name: "Active".to_string(),
            doc: String::new(),
            fields: vec![],
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    };
    let out = gen_unit_enum_type(&enum_def);
    assert!(out.contains("type Status string"));
    assert!(out.contains("const ("));
    assert!(out.contains("StatusActive"));
}

#[test]
fn test_gen_struct_type_emits_json_tags() {
    let typ = TypeDef {
        name: "MyConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![simple_field("timeout", TypeRef::Primitive(PrimitiveType::U64))],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(out.contains("type MyConfig struct"));
    assert!(out.contains("json:\"timeout\""));
}

#[test]
fn test_gen_data_enum_sealed_interface() {
    // Test tagged-data enum (named fields): emits sealed interface pattern
    let enum_def = EnumDef {
        name: "AuthConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: "Authentication configuration.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        variants: vec![
            EnumVariant {
                name: "Basic".to_string(),
                doc: "Basic auth variant.".to_string(),
                fields: vec![
                    simple_field("username", TypeRef::String),
                    simple_field("password", TypeRef::String),
                ],
                is_default: false,
                serde_rename: Some("basic".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
            },
            EnumVariant {
                name: "Bearer".to_string(),
                doc: "Bearer token variant.".to_string(),
                fields: vec![simple_field("token", TypeRef::String)],
                is_default: false,
                serde_rename: Some("bearer".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    };
    let out = gen_data_enum_type(&enum_def);
    // Should emit sealed interface
    assert!(out.contains("type AuthConfig interface"));
    assert!(out.contains("isAuthConfig()"));
    assert!(out.contains("Type() string"));
    // Should emit concrete structs per variant, not flat struct with all nullables
    assert!(out.contains("type AuthConfigBasic struct"));
    assert!(out.contains("type AuthConfigBearer struct"));
    // Basic variant should have username/password non-null fields
    assert!(out.contains("Username string"));
    assert!(out.contains("Password string"));
    // Bearer variant should have token field
    assert!(out.contains("Token string"));
    // No nullable fields — each struct has only its own fields
    assert!(!out.contains("*string `json:\"username,omitempty\""));
    // Should emit Unmarshal helper
    assert!(out.contains("func UnmarshalAuthConfig(data []byte)"));
    assert!(out.contains("case \"basic\""));
    assert!(out.contains("case \"bearer\""));
}

/// Regression: an `Option<Bytes>` field becomes a non-pointer `[]byte` in the Go
/// struct (slices are already nullable in Go). The MarshalJSON helper must not
/// dereference `v.Data` with `*v.Data` — that produced
/// `invalid operation: cannot indirect v.Data (variable of type []byte)`.
#[test]
fn gen_struct_type_marshal_optional_bytes_field_does_not_dereference() {
    let mut data_field = simple_field("data", TypeRef::Bytes);
    data_field.optional = true;
    let typ = TypeDef {
        name: "EmailAttachment".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![data_field],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(!out.contains("*v.Data"), "expected no `*v.Data` dereference in:\n{out}");
    assert!(
        out.contains("len(v.Data)") && out.contains("range v.Data"),
        "expected `len(v.Data)` and `range v.Data` (no dereference) in:\n{out}"
    );
}

/// Regression: a non-optional field whose type is a sealed-interface (data) enum
/// must default to `nil` (the interface zero value), NOT `TypeName{}` — composite
/// literals are not valid for interface types in Go.
#[test]
fn gen_config_options_defaults_data_enum_field_to_nil_not_composite_literal() {
    let typ = TypeDef {
        name: "ChunkingConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![simple_field("sizing", TypeRef::Named("ChunkSizing".to_string()))],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let mut data_enum_names = std::collections::HashSet::new();
    data_enum_names.insert("ChunkSizing");
    let out = gen_config_options(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &data_enum_names,
        &[],
    );
    // BUG fixed: previously emitted `Sizing: ChunkSizing{}` which is a Go compile
    // error (`invalid composite literal type ChunkSizing` — ChunkSizing is an
    // interface). Verify the constructor now uses the interface zero value `nil`.
    assert!(
        !out.contains("Sizing: ChunkSizing{}") && !out.contains("Sizing:                ChunkSizing{}"),
        "expected no `Sizing: ChunkSizing{{}}` in:\n{out}"
    );
    assert!(
        out.contains("Sizing:") && out.contains("nil"),
        "expected `Sizing: ... nil` default in:\n{out}"
    );
}

/// Regression test for STY-9: By default, data DTOs should NOT emit functional-options
/// helpers. The plain struct type should be emitted without With* or New* helpers.
#[test]
fn test_gen_struct_type_emits_no_config_options_by_default() {
    let typ = TypeDef {
        name: "ContentConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![
            simple_field("output_format", TypeRef::String),
            simple_field("timeout", TypeRef::Primitive(PrimitiveType::U64)),
        ],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    // The struct type should be emitted
    assert!(out.contains("type ContentConfig struct"), "expected struct definition");
    assert!(out.contains("OutputFormat"), "expected OutputFormat field");
    // But no functional-options should be emitted
    assert!(
        !out.contains("WithContentConfig"),
        "expected no WithContentConfig helpers"
    );
    assert!(
        !out.contains("ContentConfigOption"),
        "expected no ContentConfigOption type"
    );
    assert!(
        !out.contains("NewContentConfig"),
        "expected no NewContentConfig constructor"
    );
}

/// Regression test for STY-9: When a struct is listed in the functional_options allowlist,
/// the struct type PLUS functional-options helpers should be emitted.
#[test]
fn test_gen_config_options_emitted_when_in_allowlist() {
    let typ = TypeDef {
        name: "DialOptions".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![
            simple_field("timeout", TypeRef::Primitive(PrimitiveType::U64)),
            simple_field("verify_ssl", TypeRef::Primitive(PrimitiveType::Bool)),
        ],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    // Simulate the config allowing DialOptions for functional-options
    let out = gen_config_options(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    // Should emit the WithTimeout and WithVerifySSL helpers
    assert!(
        out.contains("WithDialOptionsTimeout"),
        "expected WithDialOptionsTimeout"
    );
    assert!(
        out.contains("WithDialOptionsVerifySSL"),
        "expected WithDialOptionsVerifySSL"
    );
    // Should emit the DialOptionsOption type
    assert!(
        out.contains("type DialOptionsOption func"),
        "expected DialOptionsOption type"
    );
    // Should emit the NewDialOptions constructor
    assert!(
        out.contains("func NewDialOptions"),
        "expected NewDialOptions constructor"
    );
}
