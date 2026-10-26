use super::test_case::render_test_case;
use crate::e2e::collection_payload_test_support as support;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;
use std::collections::{BTreeSet, HashMap};

fn full_test(assertions: Vec<crate::e2e::fixture::Assertion>) -> String {
    let config = crate::core::config::ResolvedCrateConfig {
        untagged_union_text_types: vec!["Body".into()],
        ..Default::default()
    };
    full_test_with(support::api(), config, assertions)
}

fn full_test_with(
    api: crate::core::ir::ApiSurface,
    config: crate::core::config::ResolvedCrateConfig,
    assertions: Vec<crate::e2e::fixture::Assertion>,
) -> String {
    assert_eq!(support::wire_value()["content"].as_array().unwrap().len(), 2);
    let fixture = Fixture {
        id: "parts_reply".into(),
        assertions,
        ..Default::default()
    };
    let e2e = E2eConfig {
        call: CallConfig {
            function: "reply".into(),
            result_var: "result".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut output = String::new();
    render_test_case(
        &mut output,
        &fixture,
        None,
        None,
        &e2e,
        "wasm",
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &api.types,
        &api.enums,
        &api.functions,
        "",
        &config,
        &mut BTreeSet::new(),
        &[],
    );
    output
}

#[test]
fn configured_wasm_text_projection_is_wired_through_full_test_case() {
    let output = full_test(support::assertions());
    assert_eq!(
        output
            .lines()
            .filter(|line| line.trim_start().starts_with("// skipped:"))
            .filter(|line| line.contains("untagged_union_text_types text projection"))
            .count(),
        3,
        "{output}"
    );
    assert!(!output.contains(".length"), "{output}");
    assert!(!output.contains("expect(result"), "{output}");
    assert!(output.contains("it.skip("), "{output}");
    assert!(
        output.contains("parts_reply:  (alef rendered no runnable expectation"),
        "{output}"
    );
    assert!(
        support::resolver(true)
            .text_projected_crossing("content.parts")
            .is_some()
    );
    assert_projected_test_compiles(&output);
}

#[test]
fn wasm_binding_producer_distinguishes_structural_and_configured_text_surfaces() {
    use crate::core::backend::Backend;
    let api = support::api();
    for text in [false, true] {
        let config = crate::core::config::ResolvedCrateConfig {
            name: "sample_core".into(),
            untagged_union_text_types: if text { vec!["Body".into()] } else { Vec::new() },
            ..Default::default()
        };
        let files = crate::backends::wasm::WasmBackend
            .generate_bindings(&api, &config)
            .unwrap();
        let source = files
            .iter()
            .map(|file| file.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text {
            assert!(source.contains("Option<String>"), "{source}");
            assert!(!source.contains("export type Body ="), "{source}");
        } else {
            assert!(source.contains("serde_wasm_bindgen::to_value"), "{source}");
            assert!(source.contains("JsValue"), "{source}");
        }
    }
}

fn assert_projected_test_compiles(output: &str) {
    let directory = tempfile::tempdir().unwrap();
    let Ok(tsc) = which::which_in("tsc", std::env::var_os("PATH"), directory.path()) else {
        assert!(std::env::var_os("ALEF_REQUIRE_TSC").is_none(), "tsc required");
        return;
    };
    let source = crate::e2e::template_env::render(
        "typescript/projected_test_probe.jinja",
        crate::alef_context! { test => output, expected_checks => usize::from(!output.contains("it.skip(")) },
    );
    let path = directory.path().join("probe.ts");
    std::fs::write(&path, source).unwrap();
    let result = std::process::Command::new(tsc)
        .current_dir(directory.path())
        .args([
            "--strict",
            "--noUnusedLocals",
            "--noUnusedParameters",
            "--module",
            "commonjs",
            "--target",
            "ES2022",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_node_probe(directory.path());
}

#[test]
fn mixed_projected_fixture_keeps_real_sibling_assertion() {
    let mut assertions = support::assertions();
    assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "equals".into(),
        field: Some("summary".into()),
        value: Some(serde_json::json!("report")),
        ..Default::default()
    });
    let output = full_test(assertions);
    assert_eq!(
        output
            .lines()
            .filter(|line| line.trim_start().starts_with("// skipped:"))
            .filter(|line| line.contains("untagged_union_text_types text projection"))
            .count(),
        3,
        "{output}"
    );
    assert!(!output.contains("it.skip("), "{output}");
    assert!(output.contains("expect(result.summary)"), "{output}");
    assert!(
        output.contains("console.warn(\"alef missing assertion coverage"),
        "{output}"
    );
    assert_projected_test_compiles(&output);
}

fn assert_node_probe(directory: &std::path::Path) {
    let Ok(node) = which::which_in("node", std::env::var_os("PATH"), directory) else {
        assert!(std::env::var_os("ALEF_REQUIRE_NODE").is_none(), "Node required");
        return;
    };
    let runtime = std::process::Command::new(node)
        .current_dir(directory)
        .arg(directory.join("probe.js"))
        .output()
        .unwrap();
    assert!(runtime.status.success(), "{}", String::from_utf8_lossy(&runtime.stderr));
}

fn serializable_api() -> crate::core::ir::ApiSurface {
    let mut api = support::api();
    for definition in &mut api.enums {
        definition.has_serde = true;
    }
    api
}

fn text_projection_config() -> crate::core::config::ResolvedCrateConfig {
    crate::core::config::ResolvedCrateConfig {
        untagged_union_text_types: vec!["Body".into()],
        ..Default::default()
    }
}

fn content_field(api: &mut crate::core::ir::ApiSurface) -> &mut crate::core::ir::FieldDef {
    api.types
        .iter_mut()
        .find(|definition| definition.name == "Reply")
        .and_then(|definition| definition.fields.iter_mut().find(|field| field.name == "content"))
        .expect("Reply.content exists")
}

fn assert_no_value_getter(output: &str) {
    assert!(!output.contains("contentValue"), "{output}");
    assert_eq!(
        output
            .lines()
            .filter(|line| line.trim_start().starts_with("// skipped:"))
            .filter(|line| line.contains("untagged_union_text_types text projection"))
            .count(),
        3,
        "{output}"
    );
}

/// Control for the three tests below: a serializable, ungated union field does get the getter.
#[test]
fn serializable_text_projection_asserts_through_the_value_getter() {
    let output = full_test_with(serializable_api(), text_projection_config(), support::assertions());
    assert!(output.contains("result.contentValue"), "{output}");
    assert!(!output.contains("// skipped:"), "{output}");
}

/// The backend emits no `contentValue` getter for a field behind a cfg gate.
#[test]
fn cfg_gated_text_projection_field_gets_no_value_getter_assertion() {
    let mut api = serializable_api();
    content_field(&mut api).cfg = Some("feature = \"extra\"".into());
    assert_no_value_getter(&full_test_with(api, text_projection_config(), support::assertions()));
}

/// The backend emits no `contentValue` getter for a sanitized field.
#[test]
fn sanitized_text_projection_field_gets_no_value_getter_assertion() {
    let mut api = serializable_api();
    content_field(&mut api).sanitized = true;
    assert_no_value_getter(&full_test_with(api, text_projection_config(), support::assertions()));
}

/// The backend emits no `contentValue` getter when the union type is excluded from the binding.
#[test]
fn excluded_text_projection_type_gets_no_value_getter_assertion() {
    let config = crate::core::config::ResolvedCrateConfig {
        wasm: Some(toml::from_str("exclude_types = [\"Body\"]").expect("WASM config parses")),
        ..text_projection_config()
    };
    assert_no_value_getter(&full_test_with(serializable_api(), config, support::assertions()));
}

fn wasm_config_excluding_reply_fields(fields: &str) -> crate::core::config::ResolvedCrateConfig {
    crate::core::config::ResolvedCrateConfig {
        wasm: Some(toml::from_str(&format!("[exclude_fields]\nReply = [{fields}]")).expect("WASM config parses")),
        ..text_projection_config()
    }
}

/// A sibling named `content_value` that the binding strips frees the name, so the backend emits
/// the `contentValue` getter and the assertion goes through it.
#[test]
fn stripped_sibling_value_field_keeps_the_value_getter_assertion() {
    let mut api = serializable_api();
    let reply = api
        .types
        .iter_mut()
        .find(|definition| definition.name == "Reply")
        .expect("Reply exists");
    let mut sibling = reply
        .fields
        .iter()
        .find(|field| field.name == "summary")
        .expect("Reply.summary exists")
        .clone();
    sibling.name = "content_value".into();
    reply.fields.push(sibling);
    let output = full_test_with(
        api,
        wasm_config_excluding_reply_fields("\"content_value\""),
        support::assertions(),
    );
    assert!(output.contains("result.contentValue"), "{output}");
    assert!(!output.contains("// skipped:"), "{output}");
}

/// The backend emits no `contentValue` getter when `exclude_fields` strips `content` itself.
#[test]
fn stripped_text_projection_field_gets_no_value_getter_assertion() {
    let output = full_test_with(
        serializable_api(),
        wasm_config_excluding_reply_fields("\"content\""),
        support::assertions(),
    );
    assert!(!output.contains("contentValue"), "{output}");
}
