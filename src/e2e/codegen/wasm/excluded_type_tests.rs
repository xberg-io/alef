//! A WASM snippet may only name types the WASM binding exports.
//!
//! The snippet's imports come from the crate IR, and `[crates.wasm] exclude_types` is the
//! difference between the IR and the package's export list. A snippet that constructs an excluded
//! type imports a symbol the package does not have, which the snippet validator reports as an
//! unexported member — so the fixture is refused (a recorded coverage gap) exactly as it already is
//! when the *function* is not exported. ~keep

use super::snippet::render;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

fn options_type() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "SampleOptions".to_string(),
        fields: vec![FieldDef {
            name: "heading_style".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        has_default: true,
        ..TypeDef::default()
    }]
}

fn convert_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".into(),
        rust_path: "sample::convert".into(),
        params: vec![
            ParamDef {
                name: "html".into(),
                ty: TypeRef::String,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".into(),
                ty: TypeRef::Named("SampleOptions".into()),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::String,
        error_type: Some("SampleError".into()),
        ..FunctionDef::default()
    }]
}

fn arg(name: &str, field: &str, arg_type: &str, element_type: Option<&str>) -> ArgMapping {
    ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: arg_type.into(),
        optional: false,
        owned: false,
        element_type: element_type.map(str::to_string),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn convert_config() -> E2eConfig {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "convert".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.args = vec![
        arg("html", "input.html", "string", None),
        arg("options", "input.options", "json_object", Some("SampleOptions")),
    ];
    e2e_config.call.overrides.insert(
        "wasm".into(),
        crate::core::config::e2e::CallOverride {
            options_type: Some("SampleOptions".into()),
            ..Default::default()
        },
    );
    e2e_config
}

fn convert_fixture() -> Fixture {
    Fixture {
        id: "convert_html".into(),
        description: "Convert HTML".into(),
        input: serde_json::json!({"html": "<p>x</p>", "options": {"heading_style": "atx"}}),
        ..Fixture::default()
    }
}

fn crate_config(exclude_types: &str) -> ResolvedCrateConfig {
    let mut config = ResolvedCrateConfig::default();
    config.wasm = Some(toml::from_str::<crate::core::config::WasmConfig>(exclude_types).expect("wasm config parses"));
    config
}

fn render_convert_snippet(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    render(
        &convert_fixture(),
        &convert_config(),
        config,
        &options_type(),
        &[],
        &convert_functions(),
    )
}

#[test]
fn a_snippet_naming_an_excluded_type_is_refused_rather_than_published() {
    let error = render_convert_snippet(&crate_config("exclude_types = [\"SampleOptions\"]"))
        .expect_err("an excluded options type has no exported WASM symbol to import");
    let message = error.to_string();
    assert!(
        message.contains("SampleOptions") && message.contains("exclude_types"),
        "the refusal must name the type and the config key that removed it: {message}"
    );
}

/// Negative control: the same fixture renders when nothing is excluded, so this is a refusal on the
/// exclusion and not on the fixture shape.
#[test]
fn the_same_snippet_renders_when_the_type_is_exported() {
    let body = render_convert_snippet(&crate_config("exclude_types = []")).expect("snippet renders");
    assert!(
        body.contains("WasmSampleOptions"),
        "the exported type is constructed under its prefixed WASM name:\n{body}"
    );
}

/// Negative control for the identifier match: an exclusion whose name is a prefix of the type the
/// snippet really names must not refuse it.
#[test]
fn an_exclusion_that_is_only_a_substring_does_not_refuse() {
    let body = render_convert_snippet(&crate_config("exclude_types = [\"Sample\"]")).expect("snippet renders");
    assert!(
        body.contains("WasmSampleOptions"),
        "`Sample` is not `SampleOptions`, so the snippet must still render:\n{body}"
    );
}

#[test]
fn a_snippet_with_a_cfg_disabled_root_type_is_refused() {
    let mut types = options_type();
    types[0].cfg = Some(r#"feature = "sample-options""#.to_string());
    let error = render(
        &convert_fixture(),
        &convert_config(),
        &ResolvedCrateConfig::default(),
        &types,
        &[],
        &convert_functions(),
    )
    .expect_err("a cfg-disabled options type has no exported WASM symbol");
    assert!(error.to_string().contains("SampleOptions"), "{error}");
}

#[test]
fn a_snippet_with_a_dropped_dependency_root_type_is_refused() {
    let mut types = options_type();
    types[0].rust_path = "sample_options::SampleOptions".to_string();
    let config = crate_config("exclude_extra_dependencies = [\"sample-options\"]");
    let error = render(
        &convert_fixture(),
        &convert_config(),
        &config,
        &types,
        &[],
        &convert_functions(),
    )
    .expect_err("a dropped-dependency options type has no exported WASM symbol");
    assert!(error.to_string().contains("SampleOptions"), "{error}");
}

#[test]
fn a_zero_arg_snippet_returning_an_excluded_type_is_refused() {
    let fixture = Fixture {
        id: "hidden_status".into(),
        description: "Hidden status".into(),
        ..Default::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "hidden_status".into();
    let functions = [FunctionDef {
        name: "hidden_status".into(),
        rust_path: "sample::hidden_status".into(),
        return_type: TypeRef::Named("SampleOptions".into()),
        ..Default::default()
    }];

    let error = render(
        &fixture,
        &e2e,
        &crate_config("exclude_types = [\"SampleOptions\"]"),
        &options_type(),
        &[],
        &functions,
    )
    .expect_err("the WASM backend omits a function returning an excluded type");
    assert!(error.to_string().contains("does not export"), "{error}");
}

#[test]
fn a_snippet_function_accepting_an_excluded_type_is_refused() {
    let fixture = Fixture {
        id: "consume_hidden".into(),
        description: "Consume hidden".into(),
        input: serde_json::json!({ "value": "x" }),
        ..Default::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "consume_hidden".into();
    e2e.call.args = vec![arg("value", "value", "string", None)];
    let functions = [FunctionDef {
        name: "consume_hidden".into(),
        rust_path: "sample::consume_hidden".into(),
        params: vec![ParamDef {
            name: "value".into(),
            ty: TypeRef::Named("SampleOptions".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let error = render(
        &fixture,
        &e2e,
        &crate_config("exclude_types = [\"SampleOptions\"]"),
        &options_type(),
        &[],
        &functions,
    )
    .expect_err("the WASM backend omits a function accepting an excluded type");
    assert!(error.to_string().contains("does not export"), "{error}");
}
