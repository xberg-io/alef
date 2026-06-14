use super::*;

/// Regression test: tagged data enums with tuple variants holding distinct inner types
/// must produce per-variant flat field names, not a shared `_0` field that collapses all
/// variant types to the first one.  Mirrors the `Message` enum in sample-llm:
///   System(SystemMessage), User(UserMessage), Assistant(AssistantMessage)
/// The flat struct must have distinct fields `system`, `user`, `assistant` (not `_0`).
/// The From impls must reference those per-variant field names.
#[test]
fn test_tagged_data_enum_tuple_variants_get_distinct_fields() {
    let backend = PhpBackend;

    let message_enum = EnumDef {
        name: "Message".to_string(),
        rust_path: "test_lib::Message".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "System".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("system".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                version: Default::default(),
            },
            EnumVariant {
                name: "User".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("UserMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("user".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                version: Default::default(),
            },
            EnumVariant {
                name: "Assistant".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("AssistantMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("assistant".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                version: Default::default(),
            },
        ],
        doc: "Chat message".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("role".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![message_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed: {:?}", result.err());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // The flat struct must NOT have a shared `_0` field.
    assert!(
        !content.contains("pub _0:"),
        "Flat struct must not have a shared `_0` field; each tuple variant needs its own field"
    );

    // The flat struct must have per-variant fields named after the variant (snake_case).
    assert!(
        content.contains("pub system:"),
        "Flat struct must have `system` field for System variant; content:\n{content}"
    );
    assert!(
        content.contains("pub user:"),
        "Flat struct must have `user` field for User variant; content:\n{content}"
    );
    assert!(
        content.contains("pub assistant:"),
        "Flat struct must have `assistant` field for Assistant variant; content:\n{content}"
    );

    // Each field must carry a distinct type.
    assert!(
        content.contains("Option<SystemMessage>"),
        "Field `system` must have type Option<SystemMessage>; content:\n{content}"
    );
    assert!(
        content.contains("Option<UserMessage>"),
        "Field `user` must have type Option<UserMessage>; content:\n{content}"
    );
    assert!(
        content.contains("Option<AssistantMessage>"),
        "Field `assistant` must have type Option<AssistantMessage>; content:\n{content}"
    );

    // The core→binding From impl must assign per-variant fields.
    assert!(
        content.contains("system: Some(_0.into())"),
        "core→binding From impl must assign to `system`; content:\n{content}"
    );
    assert!(
        content.contains("user: Some(_0.into())"),
        "core→binding From impl must assign to `user`; content:\n{content}"
    );

    // The binding→core From impl must read from per-variant flat fields.
    assert!(
        content.contains("val.system.map(Into::into)"),
        "binding→core From impl must read from `val.system`; content:\n{content}"
    );
    assert!(
        content.contains("val.user.map(Into::into)"),
        "binding→core From impl must read from `val.user`; content:\n{content}"
    );
    assert!(
        content.contains("val.assistant.map(Into::into)"),
        "binding→core From impl must read from `val.assistant`; content:\n{content}"
    );

    // From impls must be present.
    assert!(
        content.contains("impl From<test_lib::Message> for Message"),
        "Must emit core→binding From impl; content:\n{content}"
    );
    assert!(
        content.contains("impl From<Message> for test_lib::Message"),
        "Must emit binding→core From impl; content:\n{content}"
    );
}

/// Regression test: tagged data enums (struct variants) must be lowered to flat PHP classes,
/// not string constants.  A `HashMap<String, DataEnum>` field on a struct must compile:
/// there must be a `From<core::DataEnum> for DataEnum` impl (not `From<DataEnum> for String`).
#[test]
fn test_tagged_data_enum_generates_flat_class_not_string_constants() {
    let backend = PhpBackend;

    let data_enum = EnumDef {
        name: "SecuritySchemeInfo".to_string(),
        rust_path: "test_lib::SecuritySchemeInfo".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Http".to_string(),
                fields: vec![
                    make_field("scheme", TypeRef::String, false),
                    make_field("bearer_format", TypeRef::String, true),
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("http".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            },
            EnumVariant {
                name: "ApiKey".to_string(),
                fields: vec![
                    make_field("location", TypeRef::String, false),
                    make_field("name", TypeRef::String, false),
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("apiKey".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            },
        ],
        doc: "Security scheme types".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: Some("lowercase".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let config_type = TypeDef {
        name: "OpenApiConfig".to_string(),
        rust_path: "test_lib::OpenApiConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field(
            "security_schemes",
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("SecuritySchemeInfo".to_string())),
            ),
            false,
        )],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![],
        enums: vec![data_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // Must NOT emit string constants for the data enum
    assert!(
        !content.contains("pub const SECURITYSCHEMEINFO_HTTP"),
        "Data enum must not generate string constants"
    );

    // Must emit a flat PHP class struct
    assert!(
        content.contains("pub struct SecuritySchemeInfo"),
        "Data enum must generate a flat PHP class struct"
    );

    // The struct must have a discriminator field named after the serde tag
    assert!(
        content.contains("type_tag"),
        "Flat struct must have a type_tag discriminator field"
    );

    // The struct must have variant fields
    assert!(content.contains("scheme"), "Flat struct must have scheme field");
    assert!(content.contains("location"), "Flat struct must have location field");

    // Must emit From<core::SecuritySchemeInfo> for SecuritySchemeInfo
    assert!(
        content.contains("impl From<test_lib::SecuritySchemeInfo> for SecuritySchemeInfo"),
        "Must emit core→binding From impl"
    );

    // Must emit From<SecuritySchemeInfo> for core::SecuritySchemeInfo
    assert!(
        content.contains("impl From<SecuritySchemeInfo> for test_lib::SecuritySchemeInfo"),
        "Must emit binding→core From impl"
    );

    // The HashMap field on OpenApiConfig must use the PHP class, not String
    assert!(
        content.contains("HashMap<String, SecuritySchemeInfo>"),
        "HashMap field must use the flat PHP class type, not String"
    );
}
