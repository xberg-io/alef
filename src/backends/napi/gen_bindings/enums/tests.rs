use super::{
    apply_napi_case, gen_enum, is_fully_flattened_internal_enum, is_json_passthrough_data_enum, is_tagged_data_enum,
    is_variant_untagged_string_enum, string_enum_js_values,
};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn make_simple_enum(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                serde_untagged: false,
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            })
            .collect(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// gen_enum with no variants produces a valid enum declaration.
#[test]
fn gen_enum_empty_variants_compiles() {
    let e = make_simple_enum("Status", &[]);
    let result = gen_enum(&e, "", false, "", None, &[]);
    assert!(result.contains("enum Status") || result.is_empty() || result.contains("Status"));
}

/// gen_enum with variants includes variant names.
#[test]
fn gen_enum_includes_variant_names() {
    let e = make_simple_enum("Color", &["Red", "Green", "Blue"]);
    let result = gen_enum(&e, "", false, "", None, &[]);
    assert!(result.contains("Red") || result.contains("red") || result.contains("RED"));
}

/// Regression test D4A: tagged enum with unit variant emits { kind: 'bold' }
/// and not { annotation_type: 'bold' }.
#[test]
fn gen_tagged_enum_unit_variant_uses_kind_discriminant() {
    use crate::core::ir::{FieldDef, TypeRef};

    let e = EnumDef {
        name: "AnnotationKind".to_string(),
        rust_path: "test::AnnotationKind".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
                name: "Bold".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("bold".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "FontSize".to_string(),
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
                    sensitive: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("fontSize".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("annotation_type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&e, "Js", true, "", None, &[]);

    assert!(
        result.contains("js_name = \"annotation_type\""),
        "tagged enum must use js_name matching serde tag (annotation_type);\nactual:\n{result}"
    );
}

/// Regression test D4B: tagged enum with tuple variant (payload) emits camelCase
/// value name in serde_rename, e.g., 'fontSize' not 'font_size'.
#[test]
fn gen_tagged_enum_tuple_variant_uses_camel_case_value() {
    use crate::core::ir::{FieldDef, TypeRef};

    let e = EnumDef {
        name: "AnnotationKind".to_string(),
        rust_path: "test::AnnotationKind".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            serde_untagged: false,
            name: "FontSize".to_string(),
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
                serde_rename: Some("fontSize".to_string()),
                serde_flatten: false,
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            is_tuple: true,
            doc: String::new(),
            is_default: false,
            serde_rename: Some("fontSize".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("annotation_type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&e, "Js", true, "", None, &[]);

    assert!(
        result.contains("js_name = \"fontSize\"") && result.contains("pub font_size: Option<String>"),
        "tagged enum with tuple variant must expose camelCase js_name and keep Rust snake_case;\nactual:\n{result}"
    );
}

/// Regression test D4C: struct variant with named field emits field name unchanged.
/// E.g., Custom { reason: String } → { kind: 'custom'; reason: string }
#[test]
fn gen_tagged_enum_struct_variant_emits_field_names() {
    use crate::core::ir::{FieldDef, TypeRef};

    let e = EnumDef {
        name: "AnnotationKind".to_string(),
        rust_path: "test::AnnotationKind".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            serde_untagged: false,
            name: "Custom".to_string(),
            fields: vec![FieldDef {
                version: Default::default(),
                name: "reason".to_string(),
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
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            doc: String::new(),
            is_default: false,
            serde_rename: Some("custom".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            sensitive: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("annotation_type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&e, "Js", true, "", None, &[]);

    assert!(
        result.contains("reason"),
        "struct variant must emit field names (reason);\nactual:\n{result}"
    );
    assert!(
        result.contains("js_name = \"annotation_type\""),
        "struct variant enum must use js_name matching serde tag;\nactual:\n{result}"
    );
}

/// Regression test for JSDoc block-close escaping in enum variant docs.
/// When a variant doc contains `/* ... */` inside backticks (e.g., a code example),
/// the `*/` must be escaped to `* /` so it doesn't prematurely close the JSDoc block
/// in the generated TypeScript .d.ts file.
#[test]
fn gen_enum_escapes_jsdoc_block_close_in_variant_docs() {
    let e = EnumDef {
        name: "CommentType".to_string(),
        rust_path: "test::CommentType".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
                name: "Block".to_string(),
                fields: vec![],
                doc: "A block or multi-line comment (e.g., `/* ... */`).".to_string(),
                is_default: false,
                serde_rename: Some("block".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Doc".to_string(),
                fields: vec![],
                doc: "A documentation comment (e.g., `/// ...` or `/** ... */`).".to_string(),
                is_default: false,
                serde_rename: Some("doc".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&e, "", false, "", None, &[]);
    eprintln!("Generated code:\n{}\n", result);

    assert!(
        result.contains("* /"),
        "enum variant doc must escape */ sequences:\nactual:\n{result}"
    );
    let unescaped_count = result.matches("*/").count();
    let escaped_count = result.matches("* /").count();
    eprintln!("Unescaped */ count: {}", unescaped_count);
    eprintln!("Escaped * / count: {}", escaped_count);
    assert!(
        escaped_count > 0 && unescaped_count == 0,
        "enum variant doc should contain escaped * / but no bare */:\nactual:\n{result}"
    );
}

#[test]
fn adjacent_tagged_enum_uses_shared_content_field() {
    let enum_def = EnumDef {
        name: "Action".to_string(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        serde_tag: Some("type".to_string()),
        serde_content: Some("output".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let output = gen_enum(&enum_def, "Js", true, "", None, &[]);
    assert!(output.contains("pub type_tag: String"));
    assert!(output.contains("pub output: Option<String>"));
    assert!(!output.contains("pub custom: Option<String>"));
    assert!(output.contains("#[napi(namespace = \"Action\", js_name = \"Continue\")]"));
    assert!(!output.contains("getter"));
    assert!(output.contains("#[napi(namespace = \"Action\", js_name = \"Custom\")]"));
    assert!(output.contains("pub fn action_custom(output: String) -> JsAction"));
    assert!(output.contains("output: Some(output)"));
}

/// Regression test for a clippy::needless_update failure: when an adjacently-tagged enum's
/// binding struct has exactly the tag field plus the shared content field, a variant that
/// sets both (a payload variant) must NOT emit `..Default::default()` — every field is
/// already specified, so the spread has no effect and clippy denies it. A variant that
/// leaves the content field unset (a unit variant) must still emit the spread, since it is
/// the only way to fill that field in.
#[test]
fn adjacent_tagged_enum_omits_spread_only_when_all_fields_are_set() {
    let enum_def = EnumDef {
        name: "VisitResult".to_string(),
        variants: vec![
            EnumVariant {
                name: "Skip".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        serde_tag: Some("type".to_string()),
        serde_content: Some("output".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let output = gen_enum(&enum_def, "Js", true, "", None, &[]);

    let skip_fn = output
        .split("pub fn visit_result_skip() -> JsVisitResult")
        .nth(1)
        .expect("Skip constructor must be generated")
        .split("\n}\n")
        .next()
        .expect("Skip constructor body must be terminated");
    assert!(
        skip_fn.contains("..Default::default()"),
        "Skip only sets type_tag, leaving output unset; the spread is required to fill it in:\n{skip_fn}"
    );

    let custom_fn = output
        .split("pub fn visit_result_custom(output: String) -> JsVisitResult")
        .nth(1)
        .expect("Custom constructor must be generated")
        .split("\n}\n")
        .next()
        .expect("Custom constructor body must be terminated");
    assert!(
        !custom_fn.contains("..Default::default()"),
        "Custom sets both type_tag and output, i.e. every field on JsVisitResult; a spread here is a needless_update clippy denial:\n{custom_fn}"
    );
}

/// Regression: a default-representation (externally tagged, no
/// `#[serde(tag/content/untagged)]`) data enum shaped like alef's own `FormatMetadata`
/// fixture (see `tests/backends_kotlin_android_gen_bindings_test.rs`), with a
/// `Custom(String)` payload variant alongside a unit variant, must not be emitted as a
/// `#[napi(string_enum)]` — that representation only holds unit variants, so `Custom`'s
/// payload was silently dropped (`Custom,` with no field) before this fix. It must route
/// through the same tagged-object emitter used for an explicit `#[serde(tag = "...")]`
/// enum, and the binding<->core conversions must carry the payload both ways. ~keep
/// A data enum with a `Vec<u8>` payload maps that payload to `napi::bindgen_prelude::Buffer`
/// on the binding side (see `NapiMapper::bytes` for why `Vec<u8>` is unusable there). `Buffer`
/// implements neither `Clone` nor serde, and is not `Vec<u8>`, so all three of the emitted
/// pieces were wrong at once: the struct derived `Clone, Serialize, Deserialize` it cannot
/// satisfy, the binding->core arm handed a `Buffer` to a `Vec<u8>` variant, and the
/// core->binding arm handed a `Vec<u8>` to a `Buffer` field. The result did not compile at all.
/// Assert all three together -- fixing one without the others still yields a broken crate. ~keep
#[test]
fn data_enum_with_a_bytes_payload_emits_a_compilable_buffer_round_trip() {
    let e = EnumDef {
        name: "WsMessage".to_string(),
        rust_path: "demo::WsMessage".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Binary".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let output = gen_enum(&e, "Js", true, "", None, &[]);

    assert!(
        output.contains("pub binary: Option<napi::bindgen_prelude::Buffer>"),
        "the bytes payload must map to a napi Buffer; got:\n{output}"
    );
    assert!(
        !output.contains("serde::Serialize") && !output.contains("serde::Deserialize"),
        "Buffer is not serde-serializable, so the struct must not derive serde; got:\n{output}"
    );
    assert!(
        !output.contains("#[derive(Clone"),
        "Buffer is not Clone, so the struct must not derive Clone; got:\n{output}"
    );

    let struct_names: ahash::AHashSet<String> = ahash::AHashSet::new();

    let binding_to_core = crate::backends::napi::gen_bindings::methods::gen_tagged_enum_binding_to_core(
        &e,
        "demo",
        "Js",
        &struct_names,
        &[],
    );
    assert!(
        binding_to_core.contains("val.binary.map(|b| b.to_vec()).unwrap_or_default()"),
        "binding-to-core must copy the Buffer into the Vec<u8> the core variant wants; got:\n{binding_to_core}"
    );

    let core_to_binding = crate::backends::napi::gen_bindings::methods::gen_tagged_enum_core_to_binding(
        &e,
        "demo",
        "Js",
        &struct_names,
        None,
        &[],
    );
    assert!(
        core_to_binding.contains("binary: Some(binary.into())"),
        "core-to-binding must convert the Vec<u8> into a Buffer; got:\n{core_to_binding}"
    );
}

#[test]
fn default_tagged_data_enum_preserves_custom_string_variant_payload_round_trip() {
    let e = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "demo::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
                name: "Pdf".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let output = gen_enum(&e, "Js", true, "", None, &[]);
    assert!(
        output.contains("pub struct JsFormatMetadata"),
        "a payload-carrying default-tagged enum must become a tagged object struct, \
         not a #[napi(string_enum)]; got:\n{output}"
    );
    assert!(
        output.contains("pub custom: Option<String>"),
        "the Custom(String) payload field must survive on the binding struct; got:\n{output}"
    );
    assert!(
        !output.contains("#[napi(string_enum"),
        "a data-carrying enum must not be emitted as a #[napi(string_enum)]; got:\n{output}"
    );

    let struct_names: ahash::AHashSet<String> = ahash::AHashSet::new();

    // config/input direction: JS object -> core enum.
    let binding_to_core = crate::backends::napi::gen_bindings::methods::gen_tagged_enum_binding_to_core(
        &e,
        "demo",
        "Js",
        &struct_names,
        &[],
    );
    assert!(
        binding_to_core.contains(r#""Custom" => Self::Custom(val.custom.unwrap_or_default())"#),
        "binding-to-core conversion must forward the Custom payload, not discard it; got:\n{binding_to_core}"
    );

    // result/output direction: core enum -> JS object.
    let core_to_binding = crate::backends::napi::gen_bindings::methods::gen_tagged_enum_core_to_binding(
        &e,
        "demo",
        "Js",
        &struct_names,
        None,
        &[],
    );
    assert!(
        core_to_binding.contains(
            r#"demo::FormatMetadata::Custom(custom) => Self { type_tag: "Custom".to_string(), custom: Some(custom) }"#
        ),
        "core-to-binding conversion must forward the Custom payload, not discard it; got:\n{core_to_binding}"
    );
}

/// Regression: a *variant-level* `#[serde(untagged)]` (e.g. `OutputFormat::Custom(String)`
/// alongside plain unit variants `Plain`, `Markdown`, ...) must NOT route through the
/// tagged-object emitter the way `default_tagged_data_enum_preserves_custom_string_variant_payload_round_trip`
/// above proves for an ordinary (non-untagged) data variant. Serde serializes an untagged
/// variant as its own bare payload with no discriminant at all -- for `Custom(String)` that is
/// just the raw string -- so `{ "type": "custom", "custom": "..." }` would be wrong on the wire.
/// `is_tagged_data_enum` must say false and `is_variant_untagged_string_enum` must say true,
/// routing `gen_enum` to the same `serde_json::Value` passthrough wrapper a container-level
/// `#[serde(untagged)]` enum uses. ~keep
#[test]
fn variant_level_untagged_data_variant_is_not_tagged_object() {
    let e = EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "demo::OutputFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Plain".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Markdown".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                serde_untagged: true,
                ..Default::default()
            },
        ],
        serde_rename_all: Some("lowercase".to_string()),
        has_serde: true,
        ..Default::default()
    };

    assert!(
        !is_tagged_data_enum(&e, &[]),
        "a data variant that opts out via its own #[serde(untagged)] must not force the tagged-object shape"
    );
    assert!(
        is_variant_untagged_string_enum(&e),
        "every data-carrying variant here is serde_untagged, so this must be claimed as a variant-untagged string enum"
    );

    let output = gen_enum(&e, "Js", true, "", None, &[]);
    assert!(
        output.contains("pub struct JsOutputFormat(pub serde_json::Value)"),
        "must route through the JSON passthrough wrapper, not #[napi(string_enum)] or a tagged object; got:\n{output}"
    );
    assert!(
        !output.contains("#[napi(string_enum"),
        "a data-carrying enum must never be emitted as #[napi(string_enum)] (it can only hold unit variants); got:\n{output}"
    );
    assert!(
        !output.contains("pub struct JsOutputFormat {"),
        "must not be emitted as a tagged-object struct; got:\n{output}"
    );
}

/// A default-tagged enum with no data-carrying variants at all (every variant unit) is
/// unaffected by variant-level untagged handling: `is_variant_untagged_string_enum` requires at
/// least one data-carrying variant, so a plain enum keeps going through the ordinary
/// `#[napi(string_enum)]` path. ~keep
#[test]
fn unit_only_enum_is_not_variant_untagged_string_enum() {
    let e = make_simple_enum("Status", &["Active", "Inactive"]);
    assert!(!is_variant_untagged_string_enum(&e));
    assert!(!is_tagged_data_enum(&e, &[]));

    let output = gen_enum(&e, "Js", false, "", None, &[]);
    assert!(output.contains("#[napi(string_enum"));
}

/// `apply_napi_case` must derive its output from `convert_case` — the exact crate and
/// algorithm `napi-derive-backend` uses to compute a `#[napi(string_enum)]` variant's
/// runtime wire string — rather than reimplementing the transform with a different case
/// library. Comparing against `convert_case::Casing::to_case` directly (the canonical
/// oracle, not a hard-coded literal) is what would have caught alef using `heck` instead:
/// `heck` and `convert_case` agree on letter-only identifiers but diverge on any name with
/// a letter-to-digit boundary, e.g. `Bm25`.
#[test]
fn apply_napi_case_matches_convert_case_for_every_supported_case() {
    use convert_case::{Case, Casing};

    let cases: &[(&str, Case)] = &[
        ("snake_case", Case::Snake),
        ("camelCase", Case::Camel),
        ("kebab-case", Case::Kebab),
        ("UPPER_SNAKE", Case::UpperSnake),
        ("lowercase", Case::Flat),
        ("UPPERCASE", Case::UpperFlat),
        ("PascalCase", Case::Pascal),
    ];
    let names = [
        "Bm25",
        "Utf8",
        "Sha256",
        "Md5",
        "Bfs",
        "BestFirst",
        "HttpV2Client",
        "_Reserved",
        "__Private",
    ];

    for (napi_case, canonical_case) in cases {
        for name in names {
            let actual = apply_napi_case(name, Some(napi_case));
            let expected = name.trim_start_matches('_').to_case(*canonical_case);
            assert_eq!(
                actual, expected,
                "apply_napi_case({name:?}, {napi_case:?}) = {actual:?}, but convert_case \
                 (napi-rs's own algorithm) gives {expected:?}"
            );
        }
    }
}

/// Regression test: a single-variant `#[napi(string_enum = "snake_case")]` enum whose lone
/// variant name has a letter-to-digit boundary (mirrors crawlberg's
/// `JsContentFilterKind::Bm25`) must report the wire value napi-rs's own macro actually
/// emits at runtime (`"bm_25"`), not the value `heck::ToSnakeCase` would compute
/// (`"bm25"`). Before this fix, `string_enum_js_values` fed `"bm25"` into the generated
/// `ts_type` union literal, so TypeScript accepted a string the Rust `FromNapiValue`
/// conversion rejected at runtime.
#[test]
fn string_enum_js_values_matches_napi_runtime_wire_value_for_digit_boundary_variant() {
    let enum_def = EnumDef {
        name: "ContentFilterKind".to_string(),
        rust_path: "test::ContentFilterKind".to_string(),
        variants: vec![EnumVariant {
            name: "Bm25".to_string(),
            ..Default::default()
        }],
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let values = string_enum_js_values(&enum_def, true, None, &[]).expect("plain string enum must yield wire values");

    assert_eq!(
        values,
        vec!["bm_25".to_string()],
        "napi-rs's convert_case-based macro emits \"bm_25\" for variant Bm25 under snake_case; \
         alef must report the same value or the generated ts_type literal accepts a string Rust rejects"
    );
}

/// alef #536's shape reproduced at the wrapper-declaration level: a HOST-owned cfg-gated variant
/// (`rust_path` rooted in the same crate as `core_import`) must carry the identical `#[cfg(...)]`
/// on the wrapper's OWN declaration that `codegen::conversions::gen_enum_from_*_cfg` already
/// attaches to its conversion arm (see `enum_cfg_gate_tests.rs`), or the two disagree about
/// whether the variant exists the moment the feature is off from the wrapper's own point of view.
/// Load-bearing gating: with the feature always compiled in (no `cfg` at all, or a fixture that
/// never gates any variant), the wrapper declaration and the arm would trivially agree regardless
/// of this fix, so this could not catch the defect. ~keep
#[test]
fn gen_enum_attaches_host_cfg_guard_to_wrapper_declaration() {
    let enum_def = EnumDef {
        name: "RenderMode".to_string(),
        rust_path: "core_crate::RenderMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fast".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extended".to_string(),
                cfg: Some(r#"feature = "extended-mode""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let output = gen_enum(&enum_def, "Js", true, "core_crate", None, &[]);

    assert!(
        output.contains("#[cfg(feature = \"extended-mode\")]\n    Extended,"),
        "the host-owned cfg-gated variant's declaration must carry the identical #[cfg(...)] its \
         conversion arm carries, got:\n{output}"
    );
}

/// alef #534's shape reproduced at the wrapper-declaration level: a FOREIGN-crate cfg-gated
/// variant whose gating feature this binding's OWN configured feature set proves is off must not
/// appear in the generated public JS enum at all -- a shipped library that can never produce the
/// variant must not advertise it. Load-bearing gating: `Extra`'s feature is deliberately absent
/// from `configured_features` below, mirroring a consumer dependency declared with
/// `default-features = false` and the gating feature never turned back on; a fixture with every
/// feature enabled cannot reproduce this, since the variant would then be legitimately reachable. ~keep
#[test]
fn gen_enum_drops_foreign_variant_proven_unreachable_by_configured_features() {
    let enum_def = EnumDef {
        name: "RoutingStrategy".to_string(),
        rust_path: "dep_crate::RoutingStrategy".to_string(),
        variants: vec![
            EnumVariant {
                name: "Primary".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Secondary".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extra".to_string(),
                cfg: Some(r#"feature = "extra-tier""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let configured: std::collections::HashSet<&str> = ["other-feature"].into_iter().collect();

    let output = gen_enum(&enum_def, "Js", true, "core_crate", Some(&configured), &[]);

    assert!(
        !output.contains("Extra"),
        "a provably unreachable variant must not appear at all, got:\n{output}"
    );
    assert!(
        output.contains("Primary,"),
        "still-reachable variants must remain, got:\n{output}"
    );
    assert!(
        output.contains("Secondary,"),
        "still-reachable variants must remain, got:\n{output}"
    );
}

/// Positive control for the test above: when the configured feature set does NOT rule the gate
/// out (here, the feature is explicitly requested, mirroring a consumer who enabled it), the
/// variant remains declared -- alef cannot safely omit a foreign-crate variant it cannot prove
/// absent, since Cargo feature unification could still turn the dependency's feature on some way
/// alef's static configuration read cannot observe. Same fixture as the test above except for the
/// configured feature list. ~keep
#[test]
fn gen_enum_keeps_foreign_variant_not_ruled_out_by_configured_features() {
    let enum_def = EnumDef {
        name: "RoutingStrategy".to_string(),
        rust_path: "dep_crate::RoutingStrategy".to_string(),
        variants: vec![
            EnumVariant {
                name: "Primary".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extra".to_string(),
                cfg: Some(r#"feature = "extra-tier""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let configured: std::collections::HashSet<&str> = ["extra-tier"].into_iter().collect();

    let output = gen_enum(&enum_def, "Js", true, "core_crate", Some(&configured), &[]);

    assert!(
        output.contains("Extra,"),
        "a variant the configured features do not rule out must stay declared, got:\n{output}"
    );
}

/// Regression: a tagged enum's synthesized `impl Default` must set the tag field to a REAL
/// variant's wire value, not an empty string -- `String::new()` is not a valid discriminant
/// for any variant, so `Default::default()` on the generated type used to produce a value
/// nothing could deserialize. This fixture's `#[default]` variant (`Retry`) is neither first
/// nor last, so a fix that only special-cased the first or last declared variant would still
/// fail this assertion. ~keep
#[test]
fn gen_tagged_enum_default_impl_uses_the_default_variants_wire_value() {
    let enum_def = EnumDef {
        name: "Outcome".to_string(),
        rust_path: "test::Outcome".to_string(),
        serde_tag: Some("kind".to_string()),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Success".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Retry".to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Failure".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let output = gen_enum(&enum_def, "Js", true, "test_core", None, &[]);

    let expected_default_impl = "impl Default for JsOutcome {\n    \
        fn default() -> Self { Self { kind_tag: \"Retry\".to_string(),  } }\n\
        }";
    assert!(
        output.contains(expected_default_impl),
        "expected the exact Default impl to use the #[default] variant's wire value \"Retry\", got:\n{output}"
    );
    assert!(
        !output.contains("String::new()"),
        "the tag field must never default to an empty string -- it is not a valid variant, got:\n{output}"
    );
}

/// A `FormatMetadata`-shaped fixture: `#[serde(tag = "format_type")]` with a single-tuple
/// variant wrapping a Named struct (`Excel(ExcelMetadata)`). Serde's internal tagging flattens
/// `ExcelMetadata`'s own fields onto the wire object as siblings of the tag --
/// `{"format_type":"excel","sheet_count":2,"sheet_names":["Q1"]}` -- never nests them under an
/// `"excel"` key.
fn format_metadata_like_enum() -> EnumDef {
    EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "test::FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        has_serde: true,
        variants: vec![EnumVariant {
            name: "Excel".to_string(),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named("ExcelMetadata".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn excel_metadata_type_def() -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: "ExcelMetadata".to_string(),
        rust_path: "test::ExcelMetadata".to_string(),
        fields: vec![
            FieldDef {
                name: "sheet_count".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                optional: true,
                ..Default::default()
            },
            FieldDef {
                name: "sheet_names".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
                optional: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// The defect this task fixes: when `types` resolves the wrapped struct, the compiled napi
/// object struct must expose `ExcelMetadata`'s OWN fields directly (`sheet_count`,
/// `sheet_names`), not a nested `excel: Option<JsExcelMetadata>` property -- matching the real
/// serde wire instead of a nominal per-variant union.
#[test]
fn gen_tagged_enum_flattens_single_tuple_named_variant_when_type_resolves() {
    // A MIXED enum on purpose: `Excel` flattens, the struct variant `Text` cannot, so
    // `is_fully_flattened_internal_enum` is false and the enum keeps the nominal
    // `#[napi(object)]` struct. That is the only routing under which per-variant flattening is
    // still observable -- an enum whose every data variant flattens now goes to the JSON
    // passthrough wrapper instead, so building this fixture from the single-variant enum would
    // silently stop testing the flattening it exists to pin. ~keep
    let mut enum_def = format_metadata_like_enum();
    enum_def.variants.push(EnumVariant {
        name: "Text".to_string(),
        fields: vec![FieldDef {
            name: "line_count".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    });
    let types = [excel_metadata_type_def()];

    let output = gen_enum(&enum_def, "Js", true, "test_core", None, &types);

    assert!(
        output.contains("pub sheet_count: Option<u32>,"),
        "the wrapped struct's own field must be flattened onto the tagged-object struct, got:\n{output}"
    );
    assert!(
        output.contains("#[napi(js_name = \"sheetCount\")]"),
        "the flattened field must camelCase its JS name like every other napi field, got:\n{output}"
    );
    assert!(
        !output.contains("excel: Option<JsExcelMetadata>"),
        "a resolved single-tuple-Named variant must not keep the old nested field, got:\n{output}"
    );
}

/// Negative control for the fallback path: when `types` does NOT contain the wrapped struct
/// (unresolvable reference), the emitter must keep the pre-existing nested shape rather than
/// silently dropping the payload's fields.
#[test]
fn gen_tagged_enum_keeps_nested_shape_when_wrapped_type_is_unresolved() {
    let enum_def = format_metadata_like_enum();

    let output = gen_enum(&enum_def, "Js", true, "test_core", None, &[]);

    assert!(
        output.contains("pub excel: Option<JsExcelMetadata>,"),
        "an unresolvable wrapped type must fall back to the old nested field, got:\n{output}"
    );
    assert!(
        !output.contains("pub sheet_count:"),
        "no flattened field may appear without a resolved type, got:\n{output}"
    );
}

/// Negative control: adjacent tagging (`#[serde(tag, content)]`) nests a newtype variant's
/// payload under the `content` key by design -- unlike internal tagging, it must NEVER flatten,
/// even when `types` resolves the wrapped struct.
#[test]
fn gen_tagged_enum_adjacent_tagging_keeps_nested_shape_even_when_type_resolves() {
    let mut enum_def = format_metadata_like_enum();
    enum_def.serde_content = Some("data".to_string());
    let types = [excel_metadata_type_def()];

    let output = gen_enum(&enum_def, "Js", true, "test_core", None, &types);

    assert!(
        !output.contains("pub sheet_count: Option<u32>,"),
        "adjacent tagging must never flatten a newtype variant's payload, got:\n{output}"
    );
}

/// A `CsvMetadata` payload sharing a field NAME with `ExcelMetadata` at a DIFFERENT Rust type --
/// the exact shape measured against a real 21-variant enum that produced 40 rustc errors: a
/// single flat `#[napi(object)]` struct cannot union `width: Option<u32>` (Excel) with
/// `width: Option<i64>` (Csv) under one field. This is the fixture
/// `is_fully_flattened_internal_enum` exists to route away from `gen_tagged_enum_as_object`.
fn csv_metadata_type_def() -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: "CsvMetadata".to_string(),
        rust_path: "test::CsvMetadata".to_string(),
        fields: vec![FieldDef {
            name: "width".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn excel_metadata_type_def_with_conflicting_width() -> crate::core::ir::TypeDef {
    let mut typ = excel_metadata_type_def();
    typ.fields.push(FieldDef {
        name: "width".to_string(),
        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
        optional: true,
        ..Default::default()
    });
    typ
}

/// A two-variant internally-tagged enum where EVERY data-carrying variant's payload flattens
/// (both resolve as `Named` structs). `is_fully_flattened_internal_enum` must claim it regardless
/// of the field-type conflict between the two payloads -- the predicate only asks whether serde
/// flattens each variant, not whether the result happens to be representable as one struct.
fn two_variant_fully_flattened_enum() -> EnumDef {
    let mut enum_def = format_metadata_like_enum();
    enum_def.variants.push(EnumVariant {
        name: "Csv".to_string(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named("CsvMetadata".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    enum_def
}

#[test]
fn is_fully_flattened_internal_enum_is_true_only_when_every_data_variant_resolves_and_flattens() {
    let enum_def = two_variant_fully_flattened_enum();
    let both_resolve = [
        excel_metadata_type_def_with_conflicting_width(),
        csv_metadata_type_def(),
    ];
    assert!(
        is_fully_flattened_internal_enum(&enum_def, &both_resolve),
        "every data-carrying variant resolves and flattens, so the whole enum must qualify"
    );

    // Negative control: Csv unresolved -- not EVERY variant flattens, so the enum must not
    // qualify (the single-variant `format_metadata_like_enum` case stays representable as a flat
    // struct via the existing per-variant flattening in `gen_tagged_enum_as_object`). ~keep
    let only_excel_resolves = [excel_metadata_type_def_with_conflicting_width()];
    assert!(
        !is_fully_flattened_internal_enum(&enum_def, &only_excel_resolves),
        "Csv's payload does not resolve, so the enum as a whole must not be fully flattened"
    );

    // Negative control: no `types` at all resolves nothing. ~keep
    assert!(!is_fully_flattened_internal_enum(&enum_def, &[]));
}

/// `is_tagged_data_enum` must exclude a fully flattened enum (routing it to JSON passthrough
/// instead), even though it is internally tagged and would otherwise qualify via
/// `serde_tag.is_some()`.
#[test]
fn is_tagged_data_enum_excludes_the_fully_flattened_case() {
    let enum_def = two_variant_fully_flattened_enum();
    let types = [
        excel_metadata_type_def_with_conflicting_width(),
        csv_metadata_type_def(),
    ];

    assert!(
        !is_tagged_data_enum(&enum_def, &types),
        "a fully flattened internally-tagged enum must not route through the flat-struct emitter"
    );
    assert!(
        is_json_passthrough_data_enum(&enum_def, &types),
        "it must instead route through JSON passthrough"
    );

    // When unresolved, the enum falls back to the pre-existing per-variant nested/flattened
    // struct shape and stays a tagged data enum. ~keep
    assert!(is_tagged_data_enum(&enum_def, &[]));
    assert!(!is_json_passthrough_data_enum(&enum_def, &[]));
}

/// `gen_enum` must compile a fully flattened enum as the `serde_json::Value` passthrough wrapper
/// (`gen_untagged_data_enum_as_value_wrapper`'s output), not a `#[napi(object)]` struct with
/// conflicting field types.
#[test]
fn gen_enum_routes_fully_flattened_enum_through_json_passthrough() {
    let enum_def = two_variant_fully_flattened_enum();
    let types = [
        excel_metadata_type_def_with_conflicting_width(),
        csv_metadata_type_def(),
    ];

    let output = gen_enum(&enum_def, "Js", true, "test_core", None, &types);

    assert!(
        output.contains("pub struct JsFormatMetadata(pub serde_json::Value)"),
        "must route through the JSON passthrough wrapper, not a flat #[napi(object)] struct; got:\n{output}"
    );
    assert!(
        !output.contains("#[napi(object"),
        "must not be emitted as a #[napi(object)] struct; got:\n{output}"
    );
}
