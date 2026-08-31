#[cfg(test)]
use super::*;
use crate::core::config::JavaBuilderMode;
use crate::core::ir::TypeDef;
use crate::core::ir::{CoreWrapper, DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

/// Builds the primitive literal-default shape that requires null to distinguish absence from zero. ~keep
fn make_config_type_with_primitive_default(primitive: PrimitiveType, default: i64) -> TypeDef {
    let mut typ = make_config_type_with_duration_default();
    typ.fields[0].name = "max_redirects".to_string();
    typ.fields[0].ty = TypeRef::Primitive(primitive);
    typ.fields[0].default = Some(default.to_string());
    typ.fields[0].typed_default = Some(DefaultValue::IntLiteral(default));
    typ
}

fn make_config_type_with_duration_default() -> TypeDef {
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

fn make_request_type_with_multiword_fields() -> TypeDef {
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
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
    assert!(
        out.contains("@JsonDeserialize(builder = ResponseTool.Builder.class)"),
        "flatten+Json type must emit Builder even with < 5 fields"
    );
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnySetter"),
        "Builder must have @JsonAnySetter to absorb unknown sibling fields"
    );
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnyGetter"),
        "record field must still carry @JsonAnyGetter for serialization"
    );
}

#[test]
fn opaque_handle_close_is_idempotent_and_rejects_post_close_use() {
    let typ = TypeDef {
        name: "ResourceHandle".to_string(),
        rust_path: "sample_crate::ResourceHandle".to_string(),
        original_rust_path: "sample_crate::ResourceHandle".to_string(),
        is_opaque: true,
        ..Default::default()
    };
    let out = gen_opaque_handle_class(
        "dev.sample_crate",
        &typ,
        "sample",
        &[],
        "SampleRs",
        &AHashSet::default(),
        &AHashSet::default(),
        &AHashSet::default(),
    );

    assert!(out.contains("private MemorySegment handle;"), "{out}");
    assert!(out.contains("import java.util.List;"), "{out}");
    assert!(out.contains("private final List<Throwable> failures"), "{out}");
    assert!(!out.contains("java.util.List<Throwable>"), "{out}");
    assert!(out.contains("synchronized MemorySegment handle()"), "{out}");
    assert!(
        out.contains("throw new IllegalStateException(\"ResourceHandle is closed\")"),
        "{out}"
    );
    assert!(out.contains("public synchronized void close()"), "{out}");
    assert!(out.contains("handle = MemorySegment.NULL;"), "{out}");
    assert!(out.contains("invoke(handleToFree)"), "{out}");
}

/// The defect: `max_redirects` is a bare `usize` with a literal default and no
/// `#[serde(default)]`, so it stayed an unboxed `long` and the compact constructor
/// restored the default with `maxRedirects == 0`. A caller passing an explicit 0 —
/// "follow no redirects" — silently got 10 instead.
///
/// This is the same contract `boxed_duration_compact_ctor_only_null_checks_not_zero`
/// already states for the boxed half; that test simply never exercised the primitive
/// path. Boxing the component is what makes `== null` available as the sentinel.
#[test]
fn boxed_long_literal_defaults_compile_without_coercing_zero() {
    for (primitive, default) in [
        (PrimitiveType::I64, 2),
        (PrimitiveType::U64, 80),
        (PrimitiveType::Isize, 1_024),
        (PrimitiveType::Usize, 5_242_880),
    ] {
        let typ = make_config_type_with_primitive_default(primitive, default);
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
            !out.contains("maxRedirects == 0"),
            "explicit zero must remain meaningful:\n{out}"
        );
        assert!(
            out.contains("maxRedirects == null"),
            "absence must select the default:\n{out}"
        );
        assert!(
            out.contains("private Long maxRedirects"),
            "the default-bearing Java component must be nullable and boxed:\n{out}"
        );
        assert!(
            out.contains(&format!("maxRedirects = {default}L")),
            "boxed Long defaults require a long literal:\n{out}"
        );
    }
}

/// `std::time::Duration`'s serde derive produces `{"secs":<u64>,"nanos":<u32>}`, not a bare
/// integer. A record component typed `Long requestTimeout` with no converter serializes to a
/// plain number and the FFI layer's `serde_json::from_str::<RealCoreType>` rejects it with
/// `invalid type: integer ..., expected struct Duration`. The record component must carry both
/// converter annotations so the field round-trips against the real wire shape in both directions.
#[test]
fn duration_field_gets_wire_safe_converter_annotations() {
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
        out.contains("@JsonSerialize(using = DurationMillisSerializer.class)"),
        "Duration field must serialize through the millis<->Duration converter:\n{out}"
    );
    assert!(
        out.contains("@JsonDeserialize(using = DurationMillisDeserializer.class)"),
        "Duration field must deserialize through the millis<->Duration converter:\n{out}"
    );
    assert!(
        out.contains("import com.fasterxml.jackson.databind.annotation.JsonSerialize;"),
        "JsonSerialize import must be present when the Duration serializer annotation is emitted:\n{out}"
    );
    assert!(
        out.contains("import com.fasterxml.jackson.databind.annotation.JsonDeserialize;"),
        "JsonDeserialize import must be present when the Duration deserializer annotation is emitted:\n{out}"
    );
}

/// A type below the Auto builder threshold gets no `@JsonPOJOBuilder`, so deserialization
/// flows through the record's canonical constructor and the compact-constructor's own
/// annotations — no separate builder setter exists to carry a Duration converter.
#[test]
fn duration_field_without_builder_has_no_builder_setter_annotation() {
    let typ = make_config_type_with_duration_default();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Never,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );

    assert!(!out.contains("class Builder"), "builder must not be emitted:\n{out}");
    assert!(
        out.contains("@JsonSerialize(using = DurationMillisSerializer.class)"),
        "the record component itself must still carry the converter:\n{out}"
    );
}

/// When a `@JsonPOJOBuilder` is emitted, Jackson deserializes exclusively through the
/// builder's setters (`@JsonDeserialize(builder = ...)` at the type level bypasses the
/// record's own canonical-constructor annotations entirely) — so the Duration setter needs
/// its own `@JsonDeserialize` or the field silently reverts to the bare-integer wire shape.
#[test]
fn duration_field_with_builder_annotates_the_setter_too() {
    let typ = make_config_type_with_duration_default();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Always,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );

    assert!(out.contains("class Builder"), "builder must be emitted:\n{out}");
    assert!(
        out.contains(
            "@JsonDeserialize(using = DurationMillisDeserializer.class)\n        public Builder withRequestTimeout("
        ),
        "the builder setter must carry the Duration deserializer annotation directly \
         above its declaration:\n{out}"
    );
}

/// End-to-end regression for the `ContentPart.ImageUrl` self-shadowing defect: a
/// `record ImageUrl(...)` nested inside a `sealed interface` whose own field is typed
/// `ImageUrl` (the sibling top-level struct) used to resolve to the enclosing variant record
/// itself (JLS member shadowing) rather than the intended type — silent data loss that still
/// compiled. The colliding field type must be package-qualified in the emitted tagged union. ~keep
#[test]
fn tagged_union_field_type_colliding_with_variant_name_is_package_qualified() {
    let enum_def = EnumDef {
        name: "ContentPart".to_string(),
        rust_path: "sample_crate::ContentPart".to_string(),
        serde_tag: Some("type".to_string()),
        variants: vec![EnumVariant {
            name: "ImageUrl".to_string(),
            fields: vec![
                FieldDef {
                    name: "image_url".to_string(),
                    ty: TypeRef::Named("ImageUrl".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "metadata".to_string(),
                    ty: TypeRef::Named("Metadata".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = gen_enum_class("io.xberg.literllm", &enum_def, "SampleCrawler", &[]);

    assert!(
        out.contains("io.xberg.literllm.ImageUrl imageUrl"),
        "the field whose type name collides with its own variant name must be package-qualified \
         to reach the sibling top-level type, not the shadowing nested record, got:\n{out}"
    );
    assert!(
        out.contains("Metadata metadata") && !out.contains("io.xberg.literllm.Metadata"),
        "positive control: a field type that does NOT collide with any variant name must stay \
         unqualified -- qualifying it too would make every generated type needlessly verbose, \
         got:\n{out}"
    );
}

/// Regression for the tagged-union field-naming defect (explicit-rename half): a struct-shaped
/// variant's own `#[serde(rename = "...")]` on a *field* is a different serde namespace from the
/// enum's `serde_rename_all`, which cases *variant* names. Before this fix, `gen_java_tagged_union`
/// applied no rule at all to variant payload field names -- it emitted the raw Rust field name
/// into `@JsonProperty(...)` and ignored `serde_rename` entirely, so wire deserialization from
/// the real Rust core (which honors `serde_rename`) would fail to match. ~keep
#[test]
fn tagged_union_struct_variant_field_honors_own_serde_rename() {
    let enum_def = EnumDef {
        name: "ContentPart".to_string(),
        rust_path: "sample_crate::ContentPart".to_string(),
        serde_tag: Some("kind".to_string()),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![
                FieldDef {
                    name: "field_type".to_string(),
                    ty: TypeRef::String,
                    serde_rename: Some("type".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = gen_enum_class("io.xberg.literllm", &enum_def, "SampleCrawler", &[]);

    assert!(
        out.contains("@JsonProperty(\"type\") String fieldType"),
        "a struct-variant field's own #[serde(rename)] must set the @JsonProperty wire name, \
         got:\n{out}"
    );
    assert!(
        !out.contains("@JsonProperty(\"field_type\")"),
        "the raw Rust field name must not leak onto the wire once serde_rename is set, got:\n{out}"
    );

    // Control: a field with neither an explicit serde_rename nor an enum-level
    // rename_all_fields must keep emitting its own raw name unchanged -- proving the fix does
    // not over-apply and rename fields that were never meant to be renamed. ~keep
    assert!(
        out.contains("@JsonProperty(\"value\") String value"),
        "a field without any rename must keep emitting its raw name unchanged, got:\n{out}"
    );
}

/// Regression for the tagged-union field-naming defect (container-rule half): a struct-shaped
/// variant field with NO explicit `serde_rename` must still be cased by the enum's own
/// `#[serde(rename_all_fields = "...")]`. Before this fix `gen_java_tagged_union` never
/// consulted `rename_all_fields` at all, so the raw Rust field name leaked onto the wire even
/// when the enum declared a container-wide casing rule for struct-variant fields. ~keep
#[test]
fn tagged_union_struct_variant_field_honors_container_rename_all_fields() {
    let enum_def = EnumDef {
        name: "ContentPart".to_string(),
        rust_path: "sample_crate::ContentPart".to_string(),
        serde_tag: Some("kind".to_string()),
        rename_all_fields: Some("SCREAMING_SNAKE_CASE".to_string()),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "user_name".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = gen_enum_class("io.xberg.literllm", &enum_def, "SampleCrawler", &[]);

    assert!(
        out.contains("@JsonProperty(\"USER_NAME\") String userName"),
        "a struct-variant field with no explicit serde_rename must still be cased by the \
         enum's container-level rename_all_fields rule, got:\n{out}"
    );
    assert!(
        !out.contains("@JsonProperty(\"user_name\")"),
        "the raw field name must not leak onto the wire once rename_all_fields applies, got:\n{out}"
    );
}

/// Precedence regression: when both an explicit field-level `serde_rename` and an enum-level
/// `rename_all_fields` apply to the same field, the field's own explicit rename must win --
/// mirroring serde's own precedence, where `#[serde(rename = "...")]` on a field always
/// overrides a container-wide `rename_all`/`rename_all_fields`.
#[test]
fn tagged_union_struct_variant_field_serde_rename_wins_over_rename_all_fields() {
    let enum_def = EnumDef {
        name: "ContentPart".to_string(),
        rust_path: "sample_crate::ContentPart".to_string(),
        serde_tag: Some("kind".to_string()),
        rename_all_fields: Some("SCREAMING_SNAKE_CASE".to_string()),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "field_type".to_string(),
                ty: TypeRef::String,
                serde_rename: Some("type".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = gen_enum_class("io.xberg.literllm", &enum_def, "SampleCrawler", &[]);

    assert!(
        out.contains("@JsonProperty(\"type\") String fieldType"),
        "an explicit field-level serde_rename must win over the enum's container-level \
         rename_all_fields rule, got:\n{out}"
    );
    assert!(
        !out.contains("@JsonProperty(\"FIELD_TYPE\")") && !out.contains("@JsonProperty(\"field_type\")"),
        "neither the container-cased name nor the raw name may leak once an explicit rename \
         wins, got:\n{out}"
    );
}

mod default_restoration;
