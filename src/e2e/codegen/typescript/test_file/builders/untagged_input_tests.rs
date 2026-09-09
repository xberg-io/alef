use super::*;
use crate::backends::napi::NapiBackend;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, EnumVariant, FieldDef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        ..Default::default()
    }
}

fn api() -> ApiSurface {
    let mut kind = field("choice_type", TypeRef::Named("Mode".into()));
    kind.serde_rename = Some("type".into());
    let mut config = field("config", TypeRef::Json);
    config.serde_flatten = true;
    let mut tool_type = field("tool_type", TypeRef::String);
    tool_type.serde_rename = Some("type".into());
    ApiSurface {
        crate_name: "sample".into(),
        types: vec![
            TypeDef {
                name: "Specific".into(),
                has_serde: true,
                fields: vec![kind, field("postal_code", TypeRef::String)],
                ..Default::default()
            },
            TypeDef {
                name: "Tool".into(),
                has_serde: true,
                fields: vec![tool_type, config],
                ..Default::default()
            },
        ],
        enums: vec![
            EnumDef {
                name: "Mode".into(),
                has_serde: true,
                serde_rename_all: Some("lowercase".into()),
                variants: vec![EnumVariant {
                    name: "Required".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumDef {
                name: "Choice".into(),
                has_serde: true,
                serde_untagged: true,
                variants: ["Mode", "Specific"]
                    .map(|name| EnumVariant {
                        name: name.into(),
                        is_tuple: true,
                        fields: vec![field("_0", TypeRef::Named(name.into()))],
                        ..Default::default()
                    })
                    .into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn declarations() -> String {
    NapiBackend
        .generate_type_stubs(&api(), &Default::default())
        .expect("NAPI declarations")
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn untagged_input_declarations_do_not_reuse_direct_binding_shapes() {
    let dts = declarations();
    let choice = dts.split("export type Choice =").nth(1).expect("Choice declaration");
    let choice = choice.split("\n\n").next().expect("union body");
    assert!(
        !choice.contains("| Specific\n"),
        "wire union must not reference camel-case binding: {dts}"
    );
    assert!(
        !choice.contains("| Mode\n"),
        "wire union must accept serde string literals: {dts}"
    );
    assert!(dts.contains("choiceType"), "direct binding keeps its host field: {dts}");
}

#[test]
fn node_untagged_object_payload_keeps_all_serde_wire_keys() {
    let api = api();
    let expr = node_typed_value_expression(
        &serde_json::json!({"type":"required", "postal_code":"12345"}),
        "Choice",
        &Default::default(),
        &api.types,
        &api.enums,
        &mut Default::default(),
    );
    assert_eq!(expr, r#"{ postal_code: "12345", type: "required" }"#);
}

#[test]
fn node_direct_flattened_json_is_nested_without_recasing_data() {
    let api = api();
    let expr = node_typed_value_expression(
        &serde_json::json!({"type":"function", "postal_code":"12345", "":"empty", "__proto__":{"marker":"own"}}),
        "Tool",
        &Default::default(),
        &api.types,
        &api.enums,
        &mut Default::default(),
    );
    assert!(expr.contains("config:"), "{expr}");
    assert!(expr.contains("postal_code"), "{expr}");
    assert!(expr.contains("toolType:"), "{expr}");
    assert!(!expr.contains("postalCode"), "{expr}");
}

#[test]
#[ignore = "requires real tsc and Node; executed explicitly in release verification"]
fn node_untagged_inputs_typecheck_against_actual_backend_declarations() {
    let dir = tempfile::tempdir().expect("fixture directory");
    std::fs::write(dir.path().join("binding.d.ts"), declarations()).expect("write declaration");
    let source = r#"import type { Choice, Specific } from './binding';
const mode: Choice = 'required';
const choice: Choice = { type: 'required', postal_code: '12345' };
const hostKey: keyof Specific = 'choiceType';
console.log(JSON.stringify([mode, choice, hostKey]));"#;
    std::fs::write(dir.path().join("input.ts"), source).expect("write input");
    let output = std::process::Command::new("tsc")
        .args(["--strict", "--target", "ES2022", "--module", "commonjs", "input.ts"])
        .current_dir(dir.path())
        .output()
        .expect("tsc");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new("node")
        .arg("input.js")
        .current_dir(dir.path())
        .output()
        .expect("Node");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(
        actual,
        serde_json::json!(["required", {"type":"required", "postal_code":"12345"}, "choiceType"])
    );
}

#[test]
fn top_level_node_builder_preserves_union_wire_shape() {
    let api = api();
    let value = serde_json::json!({"type":"required", "postal_code":"12345"});
    let expression = ts_builder_expression(
        value.as_object().expect("object"),
        "Choice",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &api.types,
        &api.enums,
        "",
        &[],
        &mut Default::default(),
    );
    assert!(expression.contains("postal_code:"), "{expression}");
    assert!(!expression.contains("postalCode:"), "{expression}");
}

#[test]
fn top_level_node_builder_nests_flattened_config() {
    let api = api();
    let value = serde_json::json!({"type":"function", "postal_code":"12345"});
    let expression = ts_builder_expression(
        value.as_object().expect("object"),
        "Tool",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &api.types,
        &api.enums,
        "",
        &[],
        &mut Default::default(),
    );
    assert!(expression.contains("config:"), "{expression}");
    assert!(expression.contains("postal_code:"), "{expression}");
}

#[test]
fn recursive_wire_aliases_preserve_null_and_avoid_public_name_collisions() {
    let mut api = api();
    let mut next = field("next_value", TypeRef::Named("Specific".into()));
    next.optional = true;
    api.types[0].fields.push(next);
    api.types[0]
        .fields
        .push(field("maybe_value", TypeRef::Optional(Box::new(TypeRef::String))));
    api.types.push(TypeDef {
        name: "__AlefWireSpecific".into(),
        fields: vec![field("sentinel", TypeRef::String)],
        ..Default::default()
    });
    let dts = NapiBackend
        .generate_type_stubs(&api, &Default::default())
        .expect("declarations")
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(dts.matches("export type __AlefWireSpecific_ =").count(), 1, "{dts}");
    assert!(dts.contains("next_value?: (__AlefWireSpecific_ | null)"), "{dts}");
    assert!(dts.contains("maybe_value?: (string | null)"), "{dts}");
    assert!(dts.len() < 10_000, "recursive declaration expansion must be bounded");
}

#[test]
fn untagged_duration_payloads_match_serde_and_keep_explicit_numeric_codecs() {
    let duration = std::time::Duration::new(3, 7);
    assert_eq!(
        serde_json::to_value(duration).expect("serde duration"),
        serde_json::json!({"secs":3, "nanos":7})
    );
    let mut api = api();
    api.types[0].fields.push(field("ttl", TypeRef::Duration));
    let mut millis = field("millis", TypeRef::Duration);
    millis.serde_with = Some("duration_ms".into());
    api.types[0].fields.push(millis);
    api.enums[1].variants.push(EnumVariant {
        name: "Timeout".into(),
        is_tuple: true,
        fields: vec![field("_0", TypeRef::Duration)],
        ..Default::default()
    });
    let dts = NapiBackend
        .generate_type_stubs(&api, &Default::default())
        .expect("declarations")
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(dts.contains("ttl: { secs: number; nanos: number }"), "{dts}");
    assert!(dts.contains("millis: number"), "{dts}");
    assert!(dts.contains("  | { secs: number; nanos: number }"), "{dts}");
}
