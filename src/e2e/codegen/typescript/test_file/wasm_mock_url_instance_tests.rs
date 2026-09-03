//! Regression for the wasm half of alef#309 instance 1: a `$mock_url`-templated `json_object`
//! argument on wasm used to round-trip its typed-builder output through
//! `JSON.stringify(...).replaceAll(...)` / `JSON.parse(...) as T`, which discards the real class
//! instance `ts_builder_expression` already constructed. wasm-bindgen's generated `_assertClass`
//! rejects the resulting plain object at runtime with "expected instance of WasmExtractInput".
//! `mock_url_splice::splice_mock_url_into_builder_code` fixes this by interpolating the runtime
//! URL directly into the builder's own source instead of re-parsing it. Node keeps the
//! stringify/parse path unchanged -- see `mock_url_tagged_enum_tests.rs` -- since napi's types are
//! structural and a plain object already satisfies them.

use super::args::build_args_and_setup;
use crate::core::ir::{EnumDef, FieldDef, TypeDef, TypeRef};
use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;

fn extract_input_type_def() -> TypeDef {
    TypeDef {
        name: "ExtractInput".into(),
        fields: vec![FieldDef {
            name: "uri".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
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
fn wasm_mock_url_single_object_builds_real_instance_not_json_parse() {
    let type_defs = [extract_input_type_def()];
    let enums: [EnumDef; 0] = [];
    let fixture = fixture();
    let input = serde_json::json!({ "request": { "uri": "$mock_url/pdf/fake.pdf" } });
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
        "wasm",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "Wasm",
        &config,
        false,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    let base_url_line = setup_lines
        .iter()
        .find(|line| line.contains("MockBaseUrl"))
        .expect("a wasm mock-url object argument must still declare the runtime base-url binding");
    assert_eq!(
        base_url_line,
        "const requestMockBaseUrl = process.env.MOCK_SERVER_EXTRACT_URI ?? \
         `${process.env.MOCK_SERVER_URL}/fixtures/extract_uri`;"
    );
    assert!(
        !setup_lines.iter().any(|line| line.contains("JSON.parse")),
        "wasm must never JSON.parse the builder's own construction -- that discards the real \
         class instance and fails wasm-bindgen's instanceof guard: {setup_lines:?}"
    );
    assert_eq!(
        call_args,
        "(() => { const _u0 = WasmExtractInput.default(); _u0.uri = `${requestMockBaseUrl}/pdf/fake.pdf`; \
         return _u0; })()",
        "wasm must construct a real WasmExtractInput instance with the mock url spliced in as a \
         template interpolation: {call_args}"
    );
}
