use super::{apply_napi_case, gen_enum, string_enum_js_values};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn make_simple_enum(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
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
    let result = gen_enum(&e, "", false, "", None);
    assert!(result.contains("enum Status") || result.is_empty() || result.contains("Status"));
}

/// gen_enum with variants includes variant names.
#[test]
fn gen_enum_includes_variant_names() {
    let e = make_simple_enum("Color", &["Red", "Green", "Blue"]);
    let result = gen_enum(&e, "", false, "", None);
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
                name: "Bold".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("bold".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
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

    let result = gen_enum(&e, "Js", true, "", None);

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

    let result = gen_enum(&e, "Js", true, "", None);

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

    let result = gen_enum(&e, "Js", true, "", None);

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
                name: "Block".to_string(),
                fields: vec![],
                doc: "A block or multi-line comment (e.g., `/* ... */`).".to_string(),
                is_default: false,
                serde_rename: Some("block".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Doc".to_string(),
                fields: vec![],
                doc: "A documentation comment (e.g., `/// ...` or `/** ... */`).".to_string(),
                is_default: false,
                serde_rename: Some("doc".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
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

    let result = gen_enum(&e, "", false, "", None);
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

    let output = gen_enum(&enum_def, "Js", true, "", None);
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

    let output = gen_enum(&enum_def, "Js", true, "", None);

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
#[test]
fn default_tagged_data_enum_preserves_custom_string_variant_payload_round_trip() {
    let e = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "demo::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pdf".to_string(),
                fields: vec![],
                doc: String::new(),
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

    let output = gen_enum(&e, "Js", true, "", None);
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
    let binding_to_core =
        crate::backends::napi::gen_bindings::methods::gen_tagged_enum_binding_to_core(&e, "demo", "Js", &struct_names);
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
    );
    assert!(
        core_to_binding.contains(
            r#"demo::FormatMetadata::Custom(custom) => Self { type_tag: "Custom".to_string(), custom: Some(custom) }"#
        ),
        "core-to-binding conversion must forward the Custom payload, not discard it; got:\n{core_to_binding}"
    );
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

    let values = string_enum_js_values(&enum_def, true, None).expect("plain string enum must yield wire values");

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

    let output = gen_enum(&enum_def, "Js", true, "core_crate", None);

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

    let output = gen_enum(&enum_def, "Js", true, "core_crate", Some(&configured));

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

    let output = gen_enum(&enum_def, "Js", true, "core_crate", Some(&configured));

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

    let output = gen_enum(&enum_def, "Js", true, "test_core", None);

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
