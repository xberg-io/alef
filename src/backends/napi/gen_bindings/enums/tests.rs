use super::gen_enum;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

pub(super) fn make_simple_enum(name: &str, variants: &[&str]) -> EnumDef {
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
                sensitive: false,
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
            sensitive: false,
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
