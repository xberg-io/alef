#[cfg(test)]
use super::*;
use crate::core::config::JavaBuilderMode;
use crate::core::ir::TypeDef;
use crate::core::ir::{CoreWrapper, DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

#[test]
fn externally_tagged_mixed_enum_uses_string_units_and_object_payloads() {
    let enum_def = EnumDef {
        name: "PiiCategory".into(),
        variants: vec![
            EnumVariant {
                name: "Email".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
        ],
        serde_rename_all: Some("snake_case".into()),
        ..EnumDef::default()
    };
    let emitted = gen_enum_class("io.xberg", &enum_def, "Xberg", &[]);
    assert!(emitted.contains("public sealed interface PiiCategory"));
    assert!(emitted.contains("if (wire.isTextual())"));
    assert!(emitted.contains("wire.size() != 1"));
    assert!(emitted.contains("gen.writeString(tag)"));
    assert!(emitted.contains("gen.writeFieldName(tag)"));
    assert!(emitted.contains("gen.writeTree(MAPPER.valueToTree(inner));\n    gen.writeEndObject();"));
    assert!(emitted.contains("Externally tagged unit variants must be strings"));
}

#[test]
fn externally_tagged_enum_with_untagged_string_fallback_round_trips_bare_strings() {
    let enum_def = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![
            EnumVariant {
                name: "Plain".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                serde_untagged: true,
                ..EnumVariant::default()
            },
        ],
        serde_rename_all: Some("lowercase".into()),
        ..EnumDef::default()
    };

    let emitted = gen_enum_class("io.xberg", &enum_def, "Xberg", &[]);

    assert!(emitted.contains("case \"plain\" -> new OutputFormat.Plain();"));
    assert!(emitted.contains("default -> new OutputFormat.Custom(wire.asText());"));
    assert!(emitted.contains("if (value instanceof OutputFormat.Custom untaggedCustom)"));
    assert!(
        !emitted.contains("return switch (tagValue)"),
        "an object branch whose every variant throws cannot be a Java switch expression: {emitted}"
    );
    assert!(emitted.contains("gen.writeString(untaggedCustom.value());"));
    assert!(emitted.contains("Untagged variant must be a string"));
}

#[test]
fn external_string_union_default_constructs_the_nested_unit_record() {
    let enum_def = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![
            EnumVariant {
                name: "Plain".into(),
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                serde_untagged: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };
    let mut typ = make_config_type_with_duration_default();
    typ.name = "ExtractionConfig".into();
    typ.fields[0].name = "output_format".into();
    typ.fields[0].ty = TypeRef::Named("OutputFormat".into());
    typ.fields[0].optional = false;
    typ.fields[0].default = Some("/* serde(default) */".into());
    typ.fields[0].typed_default = None;
    let mut enum_defaults = ahash::AHashMap::default();
    enum_defaults.insert(
        "OutputFormat".to_string(),
        crate::extract::default_value_for_enum::DefaultEnumVariant {
            variant_name: "Plain".to_string(),
            is_zero_field: true,
        },
    );
    let sealed_interfaces = if emits_sealed_interface(&enum_def) {
        AHashSet::from(["OutputFormat".to_string()])
    } else {
        AHashSet::new()
    };

    let emitted = gen_record_type(
        "io.xberg",
        &typ,
        &AHashSet::default(),
        &sealed_interfaces,
        "SNAKE_CASE",
        &[],
        "Xberg",
        JavaBuilderMode::Always,
        &enum_defaults,
        &sealed_interfaces,
        &HashSet::default(),
    );

    assert!(emitted.contains("new OutputFormat.Plain()"), "got:\n{emitted}");
    assert!(!emitted.contains("OutputFormat.PLAIN"), "got:\n{emitted}");
}

#[test]
fn externally_tagged_complex_payloads_stay_on_the_existing_plain_enum_path() {
    let cases = [
        vec![FieldDef {
            name: "_0".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            ..FieldDef::default()
        }],
        vec![
            FieldDef {
                name: "_0".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
            FieldDef {
                name: "_1".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
        ],
    ];
    for fields in cases {
        let enum_def = EnumDef {
            name: "PiiCategory".into(),
            variants: vec![
                EnumVariant {
                    name: "Email".into(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Custom".into(),
                    fields,
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        };
        assert!(emits_get_value(&enum_def));
        assert!(!gen_enum_class("io.xberg", &enum_def, "Xberg", &[]).contains("sealed interface PiiCategory"));
    }

    let excluded = EnumDef {
        name: "PiiCategory".into(),
        variants: vec![
            EnumVariant {
                name: "Email".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                binding_excluded: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };
    assert!(emits_get_value(&excluded));
}

/// Builds the primitive literal-default shape that requires null to distinguish absence from zero. ~keep
pub(super) fn make_config_type_with_primitive_default(primitive: PrimitiveType, default: i64) -> TypeDef {
    let mut typ = make_config_type_with_duration_default();
    typ.fields[0].name = "max_redirects".to_string();
    typ.fields[0].ty = TypeRef::Primitive(primitive);
    typ.fields[0].default = Some(default.to_string());
    typ.fields[0].typed_default = Some(DefaultValue::IntLiteral(default));
    typ
}

pub(super) fn make_config_type_with_duration_default() -> TypeDef {
    TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "sample_crate::CrawlConfig".to_string(),
        original_rust_path: "sample_crate::CrawlConfig".to_string(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: "request_timeout".to_string(),
            ty: TypeRef::Duration,
            optional: false,
            default: Some("30000".to_string()),
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::IntLiteral(30000)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            sensitive: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

pub(super) fn make_request_type_with_multiword_fields() -> TypeDef {
    TypeDef {
        name: "ChatCompletionRequest".to_string(),
        rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        original_rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        fields: vec![
            FieldDef {
                version: Default::default(),
                name: "model".to_string(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "max_tokens".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "top_p".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

mod default_restoration;
