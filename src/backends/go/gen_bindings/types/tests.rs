use super::enums::{gen_data_enum_type, gen_newtype_tuple_enum_type, gen_unit_enum_type};
use super::*;
use crate::codegen::naming::apply_serde_rename_all;
use crate::core::ir::{DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// Build a minimal `TypeDef` for struct-emission tests, varying only `fields` and
/// `has_default` — the axis these regression tests exercise.
fn test_struct_type(name: &str, fields: Vec<FieldDef>, has_default: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn adjacent_tagged_enum_preserves_tag_and_content() {
    let enum_def = EnumDef {
        name: "Action".to_string(),
        serde_tag: Some("type".to_string()),
        serde_content: Some("output".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![simple_field("_0", TypeRef::String)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let out = super::enums::gen_enum_type(&enum_def, &[]);
    assert!(out.contains("Type string `json:\"type\"`"));
    assert!(out.contains("Output *string `json:\"output,omitempty\"`"));
    assert!(out.contains("func NewActionContinue() Action"));
    assert!(out.contains("func NewActionCustom(output string) Action"));
    assert!(out.contains("case \"continue\":"));
    assert!(out.contains("case \"custom\":"));
    assert!(out.contains("unknown Action type"));
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
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
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
            cfg: None,
            version: Default::default(),
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
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
    let enum_def = EnumDef {
        name: "AuthConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: "Authentication configuration.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
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
                cfg: None,
                version: Default::default(),
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
                cfg: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let out = gen_data_enum_type(&enum_def);
    assert!(out.contains("type AuthConfig interface"));
    assert!(out.contains("isAuthConfig()"));
    assert!(out.contains("Type() string"));
    assert!(out.contains("type AuthConfigBasic struct"));
    assert!(out.contains("type AuthConfigBearer struct"));
    assert!(out.contains("Username string"));
    assert!(out.contains("Password string"));
    assert!(out.contains("Token string"));
    assert!(!out.contains("*string `json:\"username,omitempty\""));
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let mut data_enum_names = std::collections::HashSet::new();
    data_enum_names.insert("ChunkSizing");
    let out = gen_config_options(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &data_enum_names,
        &std::collections::HashSet::new(),
        &[],
    );
    // BUG fixed: previously emitted `Sizing: ChunkSizing{}` which is a Go compile
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(out.contains("type ContentConfig struct"), "expected struct definition");
    assert!(out.contains("OutputFormat"), "expected OutputFormat field");
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let out = gen_config_options(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("WithDialOptionsTimeout"),
        "expected WithDialOptionsTimeout"
    );
    assert!(
        out.contains("WithDialOptionsVerifySSL"),
        "expected WithDialOptionsVerifySSL"
    );
    assert!(
        out.contains("type DialOptionsOption func"),
        "expected DialOptionsOption type"
    );
    assert!(
        out.contains("func NewDialOptions"),
        "expected NewDialOptions constructor"
    );
}

/// Helper: build an AssistantContent-like EnumDef — two tuple variants, one String
/// and one Vec<Named>, which routes to gen_passthrough_raw_message_enum.
fn make_passthrough_enum() -> EnumDef {
    EnumDef {
        name: "AssistantContent".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: "Multimodal assistant content.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_content: None,
        serde_tag: None,
        serde_untagged: true,
        serde_rename_all: None,
        rename_all_fields: None,
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                doc: String::new(),
                fields: vec![simple_field("_0", TypeRef::String)],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Parts".to_string(),
                doc: String::new(),
                fields: vec![simple_field(
                    "_0",
                    TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                )],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// When `text_types` is empty, no `Text()` method is emitted.
#[test]
fn gen_enum_type_passthrough_without_text_types_does_not_emit_text_accessor() {
    let enum_def = make_passthrough_enum();
    assert!(super::enums::is_passthrough_raw_message_enum(&enum_def));
    let out = gen_enum_type(&enum_def, &[]);
    assert!(
        out.contains("type AssistantContent json.RawMessage"),
        "type declaration must be present:\n{out}"
    );
    assert!(out.contains("MarshalJSON"), "MarshalJSON must be present:\n{out}");
    assert!(
        !out.contains("func (e AssistantContent) Text()"),
        "Text() must NOT be emitted when text_types is empty:\n{out}"
    );
}

/// When the type name appears in `text_types`, `Text() string` is emitted with the
/// correct semantics: JSON string path and JSON array path.
#[test]
fn gen_enum_type_passthrough_with_text_types_emits_text_accessor() {
    let enum_def = make_passthrough_enum();
    let text_types = vec!["AssistantContent".to_string()];
    let out = gen_enum_type(&enum_def, &text_types);
    assert!(
        out.contains("type AssistantContent json.RawMessage"),
        "type declaration must be present:\n{out}"
    );
    assert!(
        out.contains("func (e AssistantContent) Text() string"),
        "Text() method must be emitted:\n{out}"
    );
    assert!(out.contains("e[0] == '\"'"), "must handle JSON string variant:\n{out}");
    assert!(out.contains("e[0] == '['"), "must handle JSON array variant:\n{out}");
    assert!(
        out.contains("p.Type == \"text\""),
        "must filter parts by type==\"text\":\n{out}"
    );
}

fn make_unit_variant(name: &str, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        doc: String::new(),
        fields: vec![],
        is_default: false,
        serde_rename: serde_rename.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_unit_enum(name: &str, rename_all: Option<&str>, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: rename_all.is_some(),
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: rename_all.map(str::to_string),
        rename_all_fields: None,
        variants,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_unit_enum_type_honors_serde_rename_all_lowercase() {
    let enum_def = make_unit_enum(
        "ChunkerType",
        Some("lowercase"),
        vec![make_unit_variant("Text", None), make_unit_variant("Markdown", None)],
    );
    let out = gen_unit_enum_type(&enum_def);
    assert!(
        out.contains(r#"ChunkerTypeText ChunkerType = "text""#),
        "wire value must be lowercase; got:\n{out}"
    );
    assert!(
        out.contains(r#"ChunkerTypeMarkdown ChunkerType = "markdown""#),
        "wire value must be lowercase; got:\n{out}"
    );
}

#[test]
fn gen_unit_enum_type_explicit_serde_rename_wins_over_rename_all() {
    let enum_def = make_unit_enum(
        "Mode",
        Some("lowercase"),
        vec![make_unit_variant("Custom", Some("bespoke"))],
    );
    let out = gen_unit_enum_type(&enum_def);
    assert!(
        out.contains(r#"= "bespoke""#),
        "explicit serde_rename must override rename_all; got:\n{out}"
    );
}

#[test]
fn gen_unit_enum_type_no_serde_keeps_rust_variant_name() {
    let enum_def = make_unit_enum("Status", None, vec![make_unit_variant("Active", None)]);
    let out = gen_unit_enum_type(&enum_def);
    assert!(
        out.contains(r#"= "Active""#),
        "no serde attributes must preserve PascalCase; got:\n{out}"
    );
}

fn make_tuple_variant(name: &str, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        doc: String::new(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: "_0".to_string(),
            ty: TypeRef::String,
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        is_default: false,
        serde_rename: serde_rename.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: true,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_newtype_tuple_enum(name: &str, rename_all: Option<&str>, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: rename_all.is_some(),
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: rename_all.map(str::to_string),
        rename_all_fields: None,
        variants,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_newtype_tuple_enum_type_honors_serde_rename_all_lowercase() {
    let enum_def = make_newtype_tuple_enum(
        "ChunkerType",
        Some("lowercase"),
        vec![
            make_unit_variant("Text", None),
            make_unit_variant("Markdown", None),
            make_tuple_variant("Custom", None),
        ],
    );
    let out = gen_newtype_tuple_enum_type(&enum_def);
    assert!(
        out.contains("type ChunkerType string"),
        "must emit string type; got:\n{out}"
    );
    assert!(
        out.contains(r#"ChunkerTypeText ChunkerType = "text""#),
        "unit variant wire value must be lowercase; got:\n{out}"
    );
    assert!(
        out.contains(r#"ChunkerTypeMarkdown ChunkerType = "markdown""#),
        "unit variant wire value must be lowercase; got:\n{out}"
    );
}

#[test]
fn gen_newtype_tuple_enum_type_explicit_serde_rename_wins() {
    let enum_def = make_newtype_tuple_enum(
        "Mode",
        Some("lowercase"),
        vec![
            make_unit_variant("Bespoke", Some("bespoke_wire")),
            make_tuple_variant("Custom", None),
        ],
    );
    let out = gen_newtype_tuple_enum_type(&enum_def);
    assert!(
        out.contains(r#"= "bespoke_wire""#),
        "explicit serde_rename must override rename_all; got:\n{out}"
    );
}

#[test]
fn gen_newtype_tuple_enum_type_no_serde_keeps_rust_variant_name() {
    let enum_def = make_newtype_tuple_enum(
        "Format",
        None,
        vec![make_unit_variant("Json", None), make_tuple_variant("Custom", None)],
    );
    let out = gen_newtype_tuple_enum_type(&enum_def);
    assert!(
        out.contains(r#"= "Json""#),
        "no serde attributes must preserve PascalCase; got:\n{out}"
    );
}

/// Regression (Defect 1 / Defect 3): a required `Duration` field — no `#[serde(default)]`,
/// regardless of whether the *struct* derives `Default` — must be emitted as the plain,
/// non-pointer `DurationMillis` wire type with a required `json` tag. Previously any
/// `Duration` field was unconditionally pointer+omitempty and typed as bare `uint64`,
/// which cannot deserialize against Rust's `{"secs":...,"nanos":...}` `Duration` shape.
#[test]
fn gen_struct_type_required_duration_field_is_non_pointer_wire_safe_type() {
    let typ = test_struct_type(
        "RateLimitConfig",
        vec![simple_field("window", TypeRef::Duration)],
        true, // struct derives Default, but the field itself has no serde default
    );
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("Window DurationMillis `json:\"window\"`"),
        "expected a required, non-pointer DurationMillis field; got:\n{out}"
    );
    assert!(
        !out.contains("*DurationMillis") && !out.contains(",omitempty"),
        "a required Duration field must not be a pointer or omitempty; got:\n{out}"
    );
}

/// Regression: a `Duration` field that genuinely has `#[serde(default...)]` (modeled here
/// via `field.default`) still gets pointer+omitempty, since the Rust side tolerates the
/// key being absent and the Go zero value would not match the real default.
#[test]
fn gen_struct_type_optional_duration_field_with_real_default_is_pointer() {
    let mut window_field = simple_field("window", TypeRef::Duration);
    window_field.default = Some("/* serde(default) */".to_string());
    let typ = test_struct_type("RateLimitConfig", vec![window_field], true);
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("Window *DurationMillis `json:\"window,omitempty\"`"),
        "expected a pointer, omitempty DurationMillis field; got:\n{out}"
    );
}

/// Regression (Defect 3): a required `Named` enum-typed field on a struct that derives
/// `Default` (e.g. `BudgetConfig.enforcement`) must stay a plain, required value — not
/// pointer, not `omitempty` — when the field itself carries no `#[serde(default)]`.
/// Previously `is_named_enum`/`use_default_pointer` were gated on the *struct's*
/// `has_default`, so any enum field of a `Default`-deriving struct was wrongly treated as
/// wire-optional even when serde would reject the key being missing.
#[test]
fn gen_struct_type_required_named_enum_field_in_default_struct_stays_required() {
    let mut enum_names = std::collections::HashSet::new();
    enum_names.insert("Enforcement");

    let mut field = simple_field("enforcement", TypeRef::Named("Enforcement".to_string()));
    // Mirrors a resolved `impl Default` body value — present even though there is no
    // `#[serde(default)]` on the field (`field.default` stays `None`). ~keep
    field.typed_default = Some(DefaultValue::EnumVariant("Soft".to_string()));

    let typ = test_struct_type("BudgetConfig", vec![field], true);
    let out = gen_struct_type(
        &typ,
        &enum_names,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("Enforcement Enforcement `json:\"enforcement\"`"),
        "expected a required, non-pointer, non-omitempty Enforcement field; got:\n{out}"
    );
}

/// Three fields whose Rust defaults all differ from the Go zero value — the only shape that
/// can tell "the key was omitted" apart from "the Go zero was marshaled". A default equal to
/// the zero value proves nothing here. `default` stays `None` on every field: none carries a
/// per-field `#[serde(default)]`, which is exactly the container-level case. ~keep
fn non_zero_default_fields() -> Vec<FieldDef> {
    let mut timeout = simple_field("timeout", TypeRef::Primitive(PrimitiveType::U32));
    timeout.typed_default = Some(DefaultValue::IntLiteral(30));
    let mut enabled = simple_field("enabled", TypeRef::Primitive(PrimitiveType::Bool));
    enabled.typed_default = Some(DefaultValue::BoolLiteral(true));
    let mut mode = simple_field("mode", TypeRef::String);
    mode.typed_default = Some(DefaultValue::StringLiteral("fast".to_string()));
    vec![timeout, enabled, mode]
}

/// A container-level `#[serde(default)]` makes every field absent-tolerant on the wire, so a
/// field whose Rust default differs from the Go zero must be pointer+omitempty. Without it
/// `json.Marshal` writes `{"timeout":0,"enabled":false,"mode":""}` for an untouched struct and
/// the Rust side deserializes those zeros instead of `30` / `true` / `"fast"`.
#[test]
fn gen_struct_type_container_serde_default_emits_omitempty_pointers() {
    let typ = TypeDef {
        serde_container_default: true,
        serde_container_conversion: Default::default(),
        ..test_struct_type("RetryPolicy", non_zero_default_fields(), true)
    };
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("Timeout *uint32 `json:\"timeout,omitempty\"`"),
        "container #[serde(default)] must make a non-zero-default u32 field a pointer; got:\n{out}"
    );
    assert!(
        out.contains("Enabled *bool `json:\"enabled,omitempty\"`"),
        "container #[serde(default)] must make a `true`-defaulting bool field a pointer; got:\n{out}"
    );
    assert!(
        out.contains("Mode *string `json:\"mode,omitempty\"`"),
        "container #[serde(default)] must make a non-empty-default string field a pointer; got:\n{out}"
    );
}

/// The other direction, and the regression the `needs_omitempty_pointer` doc warns about: the
/// same non-zero `impl Default` values on a struct carrying **no** container-level
/// `#[serde(default)]` describe required wire keys. Emitting them as pointer+omitempty drops
/// them from `json.Marshal` output and fails Rust deserialization with `missing field`.
/// `has_default` is `true` here on purpose — having a `Default` impl must not be mistaken for
/// carrying the serde attribute.
#[test]
fn gen_struct_type_without_container_serde_default_keeps_required_fields_non_pointer() {
    let typ = test_struct_type("RetryPolicy", non_zero_default_fields(), true);
    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );
    assert!(
        out.contains("Timeout uint32 `json:\"timeout\"`"),
        "a required field must stay a plain value; got:\n{out}"
    );
    assert!(
        out.contains("Enabled bool `json:\"enabled\"`"),
        "a required field must stay a plain value; got:\n{out}"
    );
    assert!(
        out.contains("Mode string `json:\"mode\"`"),
        "a required field must stay a plain value; got:\n{out}"
    );
    assert!(
        !out.contains(",omitempty") && !out.contains("*uint32") && !out.contains("*bool") && !out.contains("*string"),
        "no field of a struct without container #[serde(default)] may be pointer+omitempty; got:\n{out}"
    );
}

/// The defect this pins: a required field whose *type* is itself a plain data struct (e.g.
/// `HeuristicsConfig`, which derives `Serialize`/`Deserialize` but carries no container-level
/// `#[serde(default)]`) has no wire-tolerant "absent" state as a plain Go value — Go's zero for
/// a struct field is a fully-populated substructure, not a marker for "unset". Left as a plain
/// field it would silently `json.Marshal` an all-zero payload that Rust's `serde` accepts as
/// genuinely-provided data. It must become pointer+omitempty instead, so an unset Go value is
/// dropped from the wire and Rust fails loudly with `missing field` — strictly better than a
/// silent wrong value. Alongside it, this struct also carries the same required scalar fields
/// as the test above, with the SAME `has_default: true` parent that caused the original
/// `TypeDef::has_default`-gated regression — proving the new struct-typed-field signal is
/// additive and does not fall back to treating a `Default` impl as license to touch them. ~keep
#[test]
fn gen_struct_type_required_struct_field_becomes_pointer_without_disturbing_required_scalars() {
    let mut fields = non_zero_default_fields();
    fields.push(simple_field(
        "heuristics",
        TypeRef::Named("HeuristicsConfig".to_string()),
    ));
    let typ = test_struct_type("CrawlOptions", fields, true);
    let mut struct_names = std::collections::HashSet::new();
    struct_names.insert("HeuristicsConfig");

    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &struct_names,
        &[],
    );

    assert!(
        out.contains("Timeout uint32 `json:\"timeout\"`"),
        "a required scalar field must stay a plain value even when a sibling struct field is \
         pointer-worthy; got:\n{out}"
    );
    assert!(
        out.contains("Enabled bool `json:\"enabled\"`"),
        "a required scalar field must stay a plain value even when a sibling struct field is \
         pointer-worthy; got:\n{out}"
    );
    assert!(
        out.contains("Mode string `json:\"mode\"`"),
        "a required scalar field must stay a plain value even when a sibling struct field is \
         pointer-worthy; got:\n{out}"
    );
    assert!(
        out.contains("Heuristics *HeuristicsConfig `json:\"heuristics,omitempty\"`"),
        "a required struct-typed field with no serde default anywhere must become \
         pointer+omitempty so an unset Go value drops the key instead of silently marshaling \
         an all-zero substructure; got:\n{out}"
    );
}

/// A unit-enum field under a container-level `#[serde(default)]` whose default resolves to
/// `DefaultValue::Empty` (the `#[derive(Default)]` shape) gets `omitempty` on the value type:
/// the Go zero for a unit enum is `""`, which is never a valid variant, so marshaling it fails
/// Rust deserialization with `unknown variant`. Omitting the key lets the Rust default fill it.
/// Without a container default the same field is a required key and must keep a plain tag.
#[test]
fn gen_struct_type_container_serde_default_marks_unit_enum_fields_omitempty() {
    let mut enum_names = std::collections::HashSet::new();
    enum_names.insert("Enforcement");
    let mut field = simple_field("enforcement", TypeRef::Named("Enforcement".to_string()));
    field.typed_default = Some(DefaultValue::Empty);

    let emit = |typ: &TypeDef| {
        gen_struct_type(
            typ,
            &enum_names,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &[],
        )
    };

    let with_container_default = TypeDef {
        serde_container_default: true,
        serde_container_conversion: Default::default(),
        ..test_struct_type("BudgetConfig", vec![field.clone()], true)
    };
    let out = emit(&with_container_default);
    assert!(
        out.contains("Enforcement Enforcement `json:\"enforcement,omitempty\"`"),
        "expected an omitempty (but non-pointer) enum field; got:\n{out}"
    );

    let without_container_default = test_struct_type("BudgetConfig", vec![field], true);
    let out = emit(&without_container_default);
    assert!(
        out.contains("Enforcement Enforcement `json:\"enforcement\"`"),
        "a required enum field must keep a plain json tag; got:\n{out}"
    );
}

/// The bug this fix targets: alef could not read the real default out of `impl Default`
/// (`Unresolved`), and `needs_omitempty_pointer` used to fall through its `_ => false` catch-all
/// for it — the same path a field whose Rust default genuinely equals the Go zero takes. That
/// left the field a plain, non-pointer value, so an untouched Go caller marshals the *Go* zero
/// onto the wire as though it were a deliberate choice, even though alef has no idea whether the
/// real (unreadable) Rust default agrees with it. Pointer+omitempty is the fix: an unset field
/// drops the key and lets the real Rust default apply.
#[test]
fn gen_struct_type_unresolved_default_field_becomes_pointer_omitempty() {
    let mut field = simple_field("retries", TypeRef::Primitive(PrimitiveType::U32));
    field.default = Some("/* serde(default) */".to_string());
    field.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let typ = test_struct_type("RetryPolicy", vec![field], true);

    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );

    assert!(
        out.contains("Retries *uint32 `json:\"retries,omitempty\"`"),
        "an unresolved default must become a pointer+omitempty field, not a plain value; got:\n{out}"
    );
}

/// Negative control: `Empty` really does mean "the Rust default IS the Go zero", so a
/// wire-optional field carrying it must stay a plain, non-pointer value — pointer+omitempty would
/// cost ergonomics for no correctness gain. Without this, a fix that pointer-ized every default
/// (rather than only the unrenderable ones) would pass the positive test above while silently
/// regressing every already-correct field.
#[test]
fn gen_struct_type_empty_default_field_stays_the_plain_go_zero() {
    let mut field = simple_field("retries", TypeRef::Primitive(PrimitiveType::U32));
    field.default = Some("/* serde(default) */".to_string());
    field.typed_default = Some(DefaultValue::Empty);
    let typ = test_struct_type("RetryPolicy", vec![field], true);

    let out = gen_struct_type(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );

    assert!(
        out.contains("Retries uint32 `json:\"retries\"`"),
        "an `Empty` default must stay a plain, non-pointer, non-omitempty field; got:\n{out}"
    );
    assert!(
        !out.contains("*uint32") && !out.contains(",omitempty"),
        "an `Empty` default must not be pointer-ized; got:\n{out}"
    );
}

/// End-to-end: the functional-options `New()` constructor must never seed an unresolved-default
/// field with the fabricated Go zero (`0`). `nil` is the pointer zero — valid Go for any type,
/// including a scalar — and matches the pointer+omitempty component type
/// `gen_struct_type_unresolved_default_field_becomes_pointer_omitempty` pins for the same field.
#[test]
fn gen_config_options_unresolved_default_field_new_constructor_seeds_nil_not_zero() {
    let mut field = simple_field("retries", TypeRef::Primitive(PrimitiveType::U32));
    field.default = Some("/* serde(default) */".to_string());
    field.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let typ = test_struct_type("RetryPolicy", vec![field], true);

    let out = gen_config_options(
        &typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    );

    assert!(
        !out.contains("Retries: 0"),
        "the New() constructor must never seed an unresolved default with the fabricated Go \
         zero:\n{out}"
    );
    assert!(
        out.contains("Retries: nil"),
        "expected the New() constructor to seed `nil`, letting the real Rust default apply once \
         the pointer is omitted from the wire; got:\n{out}"
    );
}

/// Regression (Defect 4): the sealed-interface doc comment lists every variant, cased with
/// the same `to_go_name` initialism rule as the emitted struct identifiers — not just the
/// first two, and not the raw Rust variant spelling.
#[test]
fn gen_data_enum_sealed_interface_doc_lists_all_variants_with_go_casing() {
    let make_variant = |name: &str| EnumVariant {
        name: name.to_string(),
        doc: String::new(),
        fields: vec![simple_field("value", TypeRef::String)],
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };
    let enum_def = EnumDef {
        name: "ContentPart".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        variants: vec![
            make_variant("Text"),
            make_variant("ImageUrl"),
            make_variant("Document"),
            make_variant("InputAudio"),
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let out = gen_data_enum_type(&enum_def);
    assert!(
        out.contains(
            "// Sealed interface -- use one of ContentPartText, ContentPartImageURL, \
             ContentPartDocument, ContentPartInputAudio."
        ),
        "expected all four variants, Go-cased (ImageURL, not ImageUrl); got:\n{out}"
    );
    assert!(
        !out.contains("ContentPartImageUrl"),
        "doc text must use the same casing as the emitted struct name (ImageURL); got:\n{out}"
    );
}

/// The classifier and the emitter must agree, because consumers outside this backend now act
/// on the classifier's answer alone: `e2e::codegen::go::setup` refuses a fixture value whenever
/// `accepts_string_conversion()` is false, and publishes the conversion when it is true. If a
/// generator's emitted `type X ...` line ever stops matching `go_declaration()`, that decision
/// silently inverts — either deleting snippets that compile or publishing ones that do not. So
/// assert the agreement against the real emitted text rather than trusting either side. ~keep
#[test]
fn go_enum_representation_agrees_with_the_declaration_gen_enum_type_emits() {
    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            ..EnumVariant::default()
        }
    }
    fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("samplelib::{name}"),
            variants,
            ..EnumDef::default()
        }
    }

    let cases = [
        enum_def("SampleMode", vec![variant("Fast", vec![])]),
        enum_def(
            "SampleLabel",
            vec![
                variant("Preset", vec![]),
                variant("Custom", vec![simple_field("_0", TypeRef::String)]),
            ],
        ),
        enum_def(
            "SampleInput",
            vec![
                variant("Single", vec![simple_field("_0", TypeRef::String)]),
                variant(
                    "Multiple",
                    vec![simple_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
                ),
            ],
        ),
        EnumDef {
            // A tuple-tagged union is emitted only for a serde-TAGGED (or untagged) enum,
            // because its marshalers are built around the tag. `go_enum_representation` now
            // checks the tag at the branch point instead of leaving the emitter to discover its
            // absence, which is what turned the tagless case below into a panic. ~keep
            serde_tag: Some("kind".to_string()),
            ..enum_def(
                "SampleChoice",
                vec![variant(
                    "Explicit",
                    vec![simple_field("_0", TypeRef::Named("SampleTarget".to_string()))],
                )],
            )
        },
        // The same field shape with NEITHER serde attribute: externally tagged, serde's default. ~keep
        enum_def(
            "SampleExternal",
            vec![variant(
                "Explicit",
                vec![simple_field("_0", TypeRef::Named("SampleTarget".to_string()))],
            )],
        ),
        enum_def(
            "SampleDocument",
            vec![variant("Url", vec![simple_field("url", TypeRef::String)])],
        ),
    ];

    let mut declarations = std::collections::BTreeSet::new();
    let mut convertible = std::collections::BTreeSet::new();
    for case in &cases {
        let representation = super::enums::go_enum_representation(case);
        declarations.insert(representation.go_declaration());
        if representation.accepts_string_conversion() {
            convertible.insert(representation.go_declaration());
        }
        let emitted = gen_enum_type(case, &[]);
        let expected = format!("type {} {}", case.name, representation.go_declaration());
        assert!(
            emitted.contains(&expected),
            "`{}` classified as {representation:?} must be emitted as `{expected}`:\n{emitted}",
            case.name
        );
    }

    assert_eq!(
        declarations,
        ["interface", "json.RawMessage", "string", "struct"]
            .into_iter()
            .collect(),
        "the cases must cover every Go declaration the classifier can report, so a generator \
         cannot be added without a case here"
    );
    assert_eq!(
        convertible,
        ["json.RawMessage", "string"].into_iter().collect(),
        "only `string` and `json.RawMessage` underlying types accept a Go string conversion"
    );
}

/// A Rust enum carrying neither `#[serde(tag = "...")]` nor `#[serde(untagged)]` is externally
/// tagged — serde's DEFAULT — and serialises as the single-key object `{"Variant": payload}`.
/// The classifier used to select `TupleTaggedStruct` from field shape alone while the emitter
/// additionally required a tag, so this ordinary shape aborted the whole generator run through
/// `.expect("emit_tagged_union_marshalers called for untagged enum")`, naming nothing.
///
/// A Go struct of `omitempty` variant pointers keyed by each variant's serde wire name IS that
/// object, so assert the emitted keys, not merely that nothing panicked. ~keep
#[test]
fn externally_tagged_named_tuple_enum_emits_a_single_key_struct_instead_of_panicking() {
    let enum_def = EnumDef {
        name: "SampleChoice".to_string(),
        rust_path: "samplelib::SampleChoice".to_string(),
        variants: vec![
            EnumVariant {
                name: "Alpha".to_string(),
                fields: vec![simple_field("_0", TypeRef::Named("SampleAlphaPayload".to_string()))],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Beta".to_string(),
                fields: vec![simple_field("_0", TypeRef::Named("SampleBetaPayload".to_string()))],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };

    assert_eq!(
        super::enums::go_enum_representation(&enum_def),
        GoEnumRepresentation::ExternallyTaggedStruct,
        "a tagless, non-untagged data enum is externally tagged and must not be routed to the \
         tag-consuming tuple-union generator"
    );

    let out = gen_enum_type(&enum_def, &[]);

    assert!(
        out.contains("type SampleChoice struct {"),
        "the fixture must actually produce an enum declaration before any claim about its \
         content means anything; got:\n{out}"
    );
    assert!(
        out.contains("Alpha *SampleAlphaPayload `json:\"Alpha,omitempty\"`"),
        "the variant key must be the serde wire name serde itself writes; got:\n{out}"
    );
    assert!(
        out.contains("Beta *SampleBetaPayload `json:\"Beta,omitempty\"`"),
        "every variant needs its own pointer field or that variant cannot round-trip; got:\n{out}"
    );
    assert!(
        !out.contains("MarshalJSON"),
        "encoding/json already writes exactly the one non-nil key, so a custom marshaler could \
         only diverge from external tagging; got:\n{out}"
    );
    assert!(
        !out.contains("format_type") && !out.contains("string `json:"),
        "an externally tagged enum has no discriminator field on the wire; got:\n{out}"
    );
}

/// The wire key is `wire_variant_value`, not the snake-cased container-field name the
/// internally tagged generator uses — for external tagging the key IS the wire form, so
/// `#[serde(rename_all)]` has to reach it. ~keep
#[test]
fn externally_tagged_enum_applies_serde_rename_all_to_the_variant_key() {
    let enum_def = EnumDef {
        name: "SampleChoice".to_string(),
        rust_path: "samplelib::SampleChoice".to_string(),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![EnumVariant {
            name: "AlphaOne".to_string(),
            fields: vec![simple_field("_0", TypeRef::Named("SampleAlphaPayload".to_string()))],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };

    let out = gen_enum_type(&enum_def, &[]);

    assert!(
        out.contains("type SampleChoice struct {"),
        "expected an emitted struct declaration; got:\n{out}"
    );
    assert!(
        out.contains("AlphaOne *SampleAlphaPayload `json:\"alpha_one,omitempty\"`"),
        "expected the rename_all-applied wire key; got:\n{out}"
    );
}

/// Serde writes a bare `"Variant"` string for a unit variant and a JSON array for a multi-field
/// tuple variant, and neither fits a field of a single-key struct. Those enums get the raw
/// passthrough, which round-trips every shape verbatim, rather than a struct that would silently
/// drop the variant.
///
/// `is_passthrough_raw_message_enum` must agree, because `gen_bindings::binding_file` partitions
/// enums with it and would otherwise reference a `json.RawMessage` type as if it were a string
/// enum. ~keep
#[test]
fn externally_tagged_enum_with_an_unrepresentable_variant_falls_back_to_raw_passthrough() {
    for (label, extra_variant) in [
        (
            "unit variant",
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                ..EnumVariant::default()
            },
        ),
        (
            "multi-field tuple variant",
            EnumVariant {
                name: "Pair".to_string(),
                fields: vec![
                    simple_field("_0", TypeRef::Named("SampleAlphaPayload".to_string())),
                    simple_field("_1", TypeRef::String),
                ],
                ..EnumVariant::default()
            },
        ),
    ] {
        let enum_def = EnumDef {
            name: "SampleChoice".to_string(),
            rust_path: "samplelib::SampleChoice".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Alpha".to_string(),
                    fields: vec![simple_field("_0", TypeRef::Named("SampleAlphaPayload".to_string()))],
                    ..EnumVariant::default()
                },
                extra_variant,
            ],
            ..EnumDef::default()
        };

        assert_eq!(
            super::enums::go_enum_representation(&enum_def),
            GoEnumRepresentation::RawMessage,
            "a {label} has no single-key struct field, so the typed union is not lossless"
        );
        assert!(
            super::enums::is_passthrough_raw_message_enum(&enum_def),
            "the partition predicate must report the same answer the emitter acted on ({label})"
        );

        let out = gen_enum_type(&enum_def, &[]);
        assert!(
            out.contains("type SampleChoice json.RawMessage"),
            "expected a raw passthrough declaration for a {label}; got:\n{out}"
        );
        assert!(
            out.contains("func (e SampleChoice) MarshalJSON()")
                && out.contains("func (e *SampleChoice) UnmarshalJSON("),
            "the passthrough only round-trips with both marshalers present ({label}); got:\n{out}"
        );
    }
}

/// `#[serde(tag)]` and `#[serde(untagged)]` still reach the tuple-union generator: narrowing the
/// classifier must not have moved the shapes that already worked. ~keep
#[test]
fn tagged_and_untagged_named_tuple_enums_still_select_the_tuple_union_generator() {
    fn choice(mutate: impl FnOnce(&mut EnumDef)) -> EnumDef {
        let mut enum_def = EnumDef {
            name: "SampleChoice".to_string(),
            rust_path: "samplelib::SampleChoice".to_string(),
            variants: vec![EnumVariant {
                name: "Alpha".to_string(),
                fields: vec![simple_field("_0", TypeRef::Named("SampleAlphaPayload".to_string()))],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        };
        mutate(&mut enum_def);
        enum_def
    }

    let tagged = choice(|e| e.serde_tag = Some("kind".to_string()));
    assert_eq!(
        super::enums::go_enum_representation(&tagged),
        GoEnumRepresentation::TupleTaggedStruct
    );
    let out = gen_enum_type(&tagged, &[]);
    assert!(
        out.contains("type SampleChoice struct {"),
        "expected an emitted struct declaration; got:\n{out}"
    );
    assert!(
        out.contains("Kind string `json:\"kind\"`") && out.contains("switch t.Kind {"),
        "the internally tagged form keeps its discriminator field and tag-driven marshalers; \
         got:\n{out}"
    );

    let untagged = choice(|e| e.serde_untagged = true);
    assert_eq!(
        super::enums::go_enum_representation(&untagged),
        GoEnumRepresentation::TupleTaggedStruct
    );
    let out = gen_enum_type(&untagged, &[]);
    assert!(
        out.contains("type SampleChoice struct {"),
        "expected an emitted struct declaration; got:\n{out}"
    );
    assert!(
        !out.contains("string `json:"),
        "an untagged union carries no discriminator field; got:\n{out}"
    );
}

/// `is_passthrough_raw_message_enum` partitions enums for `gen_bindings::binding_file`, so it has
/// to report what the emitter actually did. It used to restate the classifier's later conditions
/// while skipping the adjacent-tagged check that runs first, and answered "passthrough" for an
/// enum `gen_enum_type` emits as a struct — the type was then declared one way and referenced
/// another. Assert the struct emission first, then the partition that must follow from it. ~keep
#[test]
fn adjacent_tagged_enum_with_collection_payloads_is_not_reported_as_a_raw_passthrough() {
    let enum_def = EnumDef {
        name: "SampleAdjacent".to_string(),
        rust_path: "samplelib::SampleAdjacent".to_string(),
        serde_tag: Some("kind".to_string()),
        serde_content: Some("value".to_string()),
        variants: vec![
            EnumVariant {
                name: "Many".to_string(),
                fields: vec![simple_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "One".to_string(),
                fields: vec![simple_field("_0", TypeRef::String)],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };

    let out = gen_enum_type(&enum_def, &[]);
    assert!(
        out.contains("type SampleAdjacent struct"),
        "expected the adjacent-tagged struct emission; got:\n{out}"
    );
    assert_eq!(
        super::enums::go_enum_representation(&enum_def),
        GoEnumRepresentation::AdjacentTaggedStruct
    );
    assert!(
        !super::enums::is_passthrough_raw_message_enum(&enum_def),
        "a type emitted as a struct must not be partitioned as a json.RawMessage passthrough"
    );
}

/// Regression (#242): with neither `#[serde(tag = ..)]` nor `#[serde(untagged)]`, serde's
/// default for a data-carrying enum is EXTERNAL tagging: `{"Variant": <inner>}`. This is the
/// same wire shape the kotlin, pyo3, and magnus backends already emit for the equivalent
/// case; Go was the sole outlier, both on marshal (wrote fields flat, no discriminator at
/// all) and unmarshal (looked for a `Type` field that marshal never wrote).
///
/// The oracle below is a real `#[derive(Serialize)]` enum fed through `serde_json`, not a
/// hand-typed JSON literal the emitter's own logic could happen to match by construction.
/// `serde_json`'s `Map` is a `BTreeMap` here (the `preserve_order` feature is off), so its
/// keys always come back sorted -- `password` before `username` -- never in declaration
/// order; the assertions below key off that sorted order rather than re-deriving it.
#[test]
fn gen_data_enum_type_externally_tagged_round_trip_matches_serde_wire_shape() {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    enum AuthConfigOracle {
        Basic { username: String, password: String },
        Bearer { token: String },
    }

    let oracle = serde_json::to_value(AuthConfigOracle::Basic {
        username: "u".to_string(),
        password: "p".to_string(),
    })
    .expect("serde_json serialization must succeed");
    let oracle_obj = oracle
        .as_object()
        .expect("external tagging wraps the payload in an object");
    assert_eq!(
        oracle_obj.len(),
        1,
        "external tagging has exactly one top-level key: {oracle}"
    );
    let (basic_key, basic_payload) = oracle_obj.iter().next().expect("external tag has one entry");
    assert_eq!(
        basic_key, "basic",
        "serde's rename_all=snake_case wire name for `Basic`"
    );
    let basic_payload_obj = basic_payload.as_object().expect("payload is an object");
    assert_eq!(
        basic_payload_obj.keys().collect::<Vec<_>>(),
        vec!["password", "username"],
        "BTreeMap-sorted, not declaration order: {oracle}"
    );

    let enum_def = EnumDef {
        name: "AuthConfig".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        rename_all_fields: None,
        variants: vec![
            EnumVariant {
                name: "Basic".to_string(),
                doc: String::new(),
                fields: vec![
                    simple_field("username", TypeRef::String),
                    simple_field("password", TypeRef::String),
                ],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Bearer".to_string(),
                doc: String::new(),
                fields: vec![simple_field("token", TypeRef::String)],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let out = gen_data_enum_type(&enum_def);

    // Marshal half: the payload is wrapped behind the same tag key the oracle produced,
    // not folded into the payload as a field and not left untagged.
    assert!(
        out.contains(&format!(
            "return json.Marshal(map[string]aux{{\n\t\t\"{basic_key}\": {{"
        )),
        "MarshalJSON must wrap the payload under the external tag key {basic_key:?}:\n{out}"
    );
    assert!(
        !out.contains("Type string `json:\"type\"`") && !out.contains("Type string `json:\"Type\"`"),
        "external tagging must not fold a discriminator field into the payload:\n{out}"
    );
    assert!(
        !out.contains("wire.Type"),
        "must not reference an undeclared `wire.Type` (the old, broken internal-tag default \
         that never compiled):\n{out}"
    );

    // Unmarshal half: reads the tag as the object's sole key, then decodes the matching
    // variant's payload from its value -- the mirror image of the marshal half above,
    // closing the round trip through the same wire shape.
    assert!(
        out.contains("var wire map[string]json.RawMessage"),
        "UnmarshalAuthConfig must read the wire object as key -> payload:\n{out}"
    );
    assert!(
        out.contains(&format!("case \"{basic_key}\":")),
        "Unmarshal must switch on the same external tag key Marshal writes:\n{out}"
    );
    // ~keep The second variant goes through the same oracle rather than being spelled by hand:
    // a literal "bearer" here would keep passing if serde's rename_all ever stopped applying to
    // this variant, which is exactly the disagreement this test exists to detect.
    let bearer_oracle =
        serde_json::to_value(AuthConfigOracle::Bearer { token: "t".to_string() }).expect("serialization must succeed");
    let bearer_key = bearer_oracle
        .as_object()
        .expect("external tagging wraps the payload in an object")
        .keys()
        .next()
        .expect("external tag has one entry")
        .clone();
    assert_ne!(
        &bearer_key, basic_key,
        "the two variants must produce distinct wire keys, or the switch below proves nothing"
    );
    assert!(
        out.contains(&format!("case \"{bearer_key}\":")),
        "expected the second variant's oracle-derived wire case `{bearer_key}`:\n{out}"
    );
}
