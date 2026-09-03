//! Regression for alef#309 instance 1 (the node e2e "Missing field `type`" defect): a
//! `$mock_url`-templated array or single-object `json_object` argument routed the value through
//! `json_to_js_camel`/blind key-casing instead of the same typed builder (`ts_builder_expression`
//! and its per-element callers in `args.rs`) the non-templated path already uses. A fixture's bare
//! `"markdown"` for a data-carrying enum field (`OutputFormat`, internally tagged under napi's
//! default `"type"` key) must still render as `{ type: "markdown" }` once the same value also
//! needs the JSON.stringify/replaceAll/JSON.parse dance for an unrelated `$mock_url` placeholder,
//! exactly as it does when no placeholder is present.
//!
//! Split out of `args.rs` (approaching the 1,000-line split threshold) as its own concept,
//! matching `call_arity_tests.rs` and `json_object_field_agreement_tests.rs` next door. ~keep

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
            // A data-carrying variant with no explicit `#[serde(tag = ...)]` at the enum
            // level, exactly like the real `OutputFormat::Custom(String)` -- this is what makes
            // `is_tagged_data_enum` true (internally-tagged, default "type" key) even though
            // `serde_tag` is `None`.
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

fn file_extraction_config_type_def() -> TypeDef {
    TypeDef {
        name: "FileExtractionConfig".into(),
        fields: vec![FieldDef {
            name: "output_format".into(),
            ty: TypeRef::Named("OutputFormat".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn extract_input_with_config_type_def() -> TypeDef {
    TypeDef {
        name: "ExtractInput".into(),
        fields: vec![
            FieldDef {
                name: "uri".into(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "config".into(),
                ty: TypeRef::Named("FileExtractionConfig".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn fixture() -> Fixture {
    Fixture {
        id: "api_extract_batch_uri_with_config".to_string(),
        description: "Tests batch URI extraction with per-input config".to_string(),
        ..Default::default()
    }
}

#[test]
fn node_mock_url_array_element_still_tags_nested_enum_field() {
    let enums = [output_format_enum_def()];
    let type_defs = [file_extraction_config_type_def(), extract_input_with_config_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({
        "inputs": [{ "uri": "$mock_url/pdf/fake.pdf", "config": { "output_format": "markdown" } }]
    });
    let args = [ArgMapping {
        name: "inputs".into(),
        field: "input.inputs".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("ExtractInput".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
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
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    let json_stringify_line = setup_lines
        .iter()
        .find(|line| line.contains("JSON.stringify"))
        .expect("a $mock_url-templated array argument must build via JSON.stringify/replaceAll/JSON.parse");
    assert!(
        json_stringify_line.contains(r#"outputFormat: { type: "markdown" } as OutputFormat"#),
        "tagged-data enum field lost its `type` discriminator in the mock_url-templated array \
         path: {json_stringify_line}"
    );
    assert!(
        !json_stringify_line.contains(r#"outputFormat: "markdown""#),
        "regression: enum field rendered as a bare wire string instead of the tagged object \
         the napi binding requires: {json_stringify_line}"
    );
}

/// The single-object half of the same defect. `build_args_and_setup` has two `json_object`
/// branches -- one for an array-valued argument, one for a single object -- and BOTH had their
/// own copy of the `$mock_url` short-circuit that dumped the value through `json_to_js_camel`
/// before the typed builder could run. Fixing only the array branch would leave every consumer
/// whose templated argument is a plain options object still emitting a bare wire string for a
/// tagged-data enum field, with no test able to tell.
#[test]
fn node_mock_url_single_object_still_tags_nested_enum_field() {
    let enums = [output_format_enum_def()];
    let type_defs = [file_extraction_config_type_def(), extract_input_with_config_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({
        "request": { "uri": "$mock_url/pdf/fake.pdf", "config": { "output_format": "markdown" } }
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

    let (setup_lines, _call_args) = build_args_and_setup(
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
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    let json_stringify_line = setup_lines
        .iter()
        .find(|line| line.contains("JSON.stringify"))
        .expect("a $mock_url-templated object argument must build via JSON.stringify/replaceAll/JSON.parse");
    assert!(
        json_stringify_line.contains(r#"outputFormat: { type: "markdown" } as OutputFormat"#),
        "tagged-data enum field lost its `type` discriminator in the mock_url-templated \
         single-object path: {json_stringify_line}"
    );
    assert!(
        !json_stringify_line.contains(r#"outputFormat: "markdown""#),
        "regression: enum field rendered as a bare wire string instead of the tagged object \
         the napi binding requires: {json_stringify_line}"
    );
}
