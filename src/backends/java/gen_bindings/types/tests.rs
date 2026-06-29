#[cfg(test)]
use super::*;
use crate::core::config::JavaBuilderMode;
use crate::core::ir::TypeDef;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

fn make_config_type_with_duration_default() -> TypeDef {
    TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "sample_crate::CrawlConfig".to_string(),
        original_rust_path: "sample_crate::CrawlConfig".to_string(),
        fields: vec![FieldDef {
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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_request_type_with_multiword_fields() -> TypeDef {
    TypeDef {
        name: "ChatCompletionRequest".to_string(),
        rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        original_rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        fields: vec![
            FieldDef {
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// Single-word builder fields like `model` MUST get `@JsonProperty`
/// Jackson's BuilderBasedDeserializer requires @JsonProperty on every setter
/// to correctly map JSON properties to setters.
#[test]
fn single_word_builder_field_gets_json_property() {
    let typ = make_request_type_with_multiword_fields();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    // `model` is single-word: Jackson still requires @JsonProperty on the builder setter
    // to map JSON fields to setters correctly.
    assert!(
        out.contains("@JsonProperty(\"model\")"),
        "single-word builder field must get @JsonProperty; got:\n{out}"
    );
}

/// Multi-word snake_case fields like `max_tokens` → `maxTokens` MUST get
/// `@JsonProperty("max_tokens")` so Jackson sends the snake_case wire name
/// that Rust's serde expects.
#[test]
fn multiword_snake_case_field_gets_json_property_annotation() {
    let typ = make_request_type_with_multiword_fields();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("@JsonProperty(\"max_tokens\")"),
        "multi-word field max_tokens must have @JsonProperty(\"max_tokens\") annotation; got:\n{out}"
    );
    assert!(
        out.contains("@JsonProperty(\"top_p\")"),
        "multi-word field top_p must have @JsonProperty(\"top_p\") annotation; got:\n{out}"
    );
    // The import must also be present.
    assert!(
        out.contains("import com.fasterxml.jackson.annotation.JsonProperty;"),
        "JsonProperty import must be present when @JsonProperty annotations are emitted"
    );
}

#[test]
fn boxed_duration_compact_ctor_only_null_checks_not_zero() {
    let typ = make_config_type_with_duration_default();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("requestTimeout == null"),
        "expected null-check in compact ctor"
    );
    assert!(
        !out.contains("requestTimeout == 0"),
        "must not coerce explicit 0 — that is a user-intentional value"
    );
}

/// A type with only 2 visible fields but one carrying `#[serde(flatten)]` on a
/// `serde_json::Value` field must still emit a Builder (with `@JsonAnySetter`)
/// regardless of the Auto field-count threshold.  Without the Builder, Jackson
/// cannot absorb unknown sibling keys and throws
/// `Unrecognized field "..." not marked as ignorable`.
#[test]
fn flatten_json_field_forces_builder_emission_below_auto_threshold() {
    use crate::core::ir::CoreWrapper;
    let typ = TypeDef {
        name: "ResponseTool".to_string(),
        rust_path: "sample_llm::ResponseTool".to_string(),
        original_rust_path: "sample_llm::ResponseTool".to_string(),
        fields: vec![
            FieldDef {
                name: "tool_type".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: Some("\"\"".to_string()),
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: Some("type".to_string()),
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "config".to_string(),
                ty: TypeRef::Json,
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
                serde_flatten: true,
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
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let out = gen_record_type(
        "dev.sample_crate.samplellm",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    // Builder must be emitted so @JsonAnySetter can absorb unknown sibling fields.
    assert!(
        out.contains("@JsonDeserialize(builder = ResponseTool.Builder.class)"),
        "flatten+Json type must emit Builder even with < 5 fields"
    );
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnySetter"),
        "Builder must have @JsonAnySetter to absorb unknown sibling fields"
    );
    // The record field itself should still use @JsonAnyGetter for serialization.
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnyGetter"),
        "record field must still carry @JsonAnyGetter for serialization"
    );
}
