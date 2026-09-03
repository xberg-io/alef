//! Exact-string companion to `mock_url_tagged_enum_tests.rs`: pins the precise
//! `JSON.stringify(...)` line node emits for a `$mock_url`-templated single-object argument whose
//! nested field is a data-carrying enum (`OutputFormat`). A `contains` assertion alone cannot
//! distinguish "tagged object nested correctly" from "tagged object present somewhere in a
//! differently-shaped line" -- this asserts the whole generated line.

use super::args::build_args_and_setup;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;

fn output_format_enum_def() -> EnumDef {
    EnumDef {
        name: "OutputFormat".into(),
        serde_rename_all: Some("lowercase".into()),
        variants: vec![
            EnumVariant {
                name: "Markdown".into(),
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn extract_input_type_def() -> TypeDef {
    TypeDef {
        name: "ExtractInput".into(),
        fields: vec![
            FieldDef {
                name: "uri".into(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "output_format".into(),
                ty: TypeRef::Named("OutputFormat".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn fixture() -> Fixture {
    Fixture {
        id: "extract_uri".to_string(),
        description: "Extract from a URI".to_string(),
        ..Default::default()
    }
}

#[test]
fn node_mock_url_object_tags_enum_field_exactly() {
    let enums = [output_format_enum_def()];
    let type_defs = [extract_input_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({
        "request": { "uri": "$mock_url/pdf/fake.pdf", "output_format": "markdown" }
    });
    let args = [ArgMapping {
        name: "request".into(),
        field: "input.request".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        Some("ExtractInput"),
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        false,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    // ~keep node now splices the runtime URL into the builder source instead of round-tripping
    // through JSON, so the whole typed expression is the call argument and the only setup line is
    // the base-url binding. Pinned exactly, because this test exists to catch the enum field
    // silently degrading to a bare wire string.
    assert!(
        setup_lines.iter().all(|line| !line.contains("JSON.stringify")),
        "the lossy JSON round trip must be gone: {setup_lines:?}"
    );
    assert_eq!(
        call_args,
        "{ outputFormat: { type: \"markdown\" } as OutputFormat, \
         uri: `${requestMockBaseUrl}/pdf/fake.pdf` } as ExtractInput",
        "the enum field must render as a tagged object, not a bare wire string"
    );
}
