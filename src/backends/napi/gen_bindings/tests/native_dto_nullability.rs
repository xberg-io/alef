use super::super::NapiBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::validation::ValidatedApiSurface;

const SOURCE: &str = r#"
use std::collections::BTreeMap;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Packet { pub fallbacks: Option<Vec<BTreeMap<String, serde_json::Value>>> }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct NestedPacket {
    pub fallbacks: Option<Option<Vec<BTreeMap<String, serde_json::Value>>>>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct JsonPayload { pub value: serde_json::Value }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WirePayload { pub maybe_value: Option<String> }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum WireChoice { Empty, Inline { raw_value: Option<String> }, Named(WirePayload) }

pub fn echo_packet(value: Packet) -> Packet { value }
pub fn echo_nested(value: NestedPacket) -> NestedPacket { value }
pub fn echo_json(value: JsonPayload) -> JsonPayload { value }
pub fn echo_wire(value: WireChoice) -> WireChoice { value }
pub fn echo_optional(value: Option<String>) -> Option<String> { value }
"#;

fn assert_extracted_optional_shapes(api: &ApiSurface) {
    assert_eq!((api.types.len(), api.enums.len(), api.functions.len()), (4, 1, 5));
    let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    let vector = TypeRef::Vec(Box::new(map));
    for (name, expected) in [
        ("Packet", vector.clone()),
        ("NestedPacket", TypeRef::Optional(Box::new(vector))),
    ] {
        let typ = api.types.iter().find(|typ| typ.name == name).unwrap();
        assert!(!typ.is_opaque);
        assert_eq!(typ.fields.len(), 1);
        assert!(typ.fields[0].optional);
        assert_eq!(typ.fields[0].ty, expected);
    }
    assert!(super::super::enums::is_adjacent_content_collision_enum(&api.enums[0]));
}

fn fixture_dts() -> String {
    let directory = tempfile::tempdir().expect("public extractor native DTO nullability fixture");
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let api = crate::extract::extractor::extract(&[source.as_path()], "nullability_core", "0.1.0", None)
        .expect("actual extraction of native DTO and Serde wire contracts");
    assert_extracted_optional_shapes(&api);
    ValidatedApiSurface::new(&api, &[]).expect("unsuppressed fixture validation");
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['node']\n[[crates]]\nname='nullability_core'\nsources=['lib.rs']\n")
            .unwrap();
    let config = raw.resolve().unwrap().remove(0);
    let outputs = NapiBackend.generate_type_stubs(&api, &config).unwrap();
    assert_eq!(outputs.len(), 3, "the complete public declaration pipeline must run");
    let declarations: Vec<_> = outputs
        .iter()
        .filter(|output| output.path.to_string_lossy().ends_with("index.d.ts"))
        .collect();
    assert_eq!(declarations.len(), 1, "exactly one public declaration file");
    declarations[0].content.clone()
}

fn interface_body<'a>(dts: &'a str, name: &str) -> &'a str {
    let marker = format!("export interface {name} {{\n");
    assert_eq!(
        dts.matches(marker.as_str()).count(),
        1,
        "one native DTO interface: {name}"
    );
    dts.split_once(marker.as_str())
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0
        .trim()
}

#[test]
fn native_optional_array_fields_do_not_advertise_explicit_null() {
    let dts = fixture_dts();
    for name in ["Packet", "NestedPacket"] {
        assert_eq!(
            interface_body(&dts, name),
            "readonly fallbacks?: Array<Record<string, JsonValue>>",
            "native optional DTO field must match the observed absent/array contract: {name}"
        );
    }
}

#[test]
fn json_values_and_function_options_keep_their_nullable_contracts() {
    let dts = fixture_dts();
    assert_eq!(interface_body(&dts, "JsonPayload"), "readonly value: JsonValue");
    let json_value = concat!(
        "export type JsonValue = string | number | boolean | null | ",
        "JsonValue[] | { [key: string]: JsonValue };"
    );
    assert!(dts.lines().any(|line| line == json_value));
    let optional_function = concat!(
        "export declare function echoOptional(value?: string | undefined | null)",
        ": string | null;"
    );
    assert!(dts.lines().any(|line| line == optional_function));
}

#[test]
fn serde_raw_enum_wire_aliases_keep_nullable_optional_members() {
    let dts = fixture_dts();
    assert!(
        dts.lines()
            .any(|line| line == "export type __AlefWireWirePayload = { maybe_value?: (string | null) };")
    );
    assert!(
        dts.lines()
            .any(|line| line.trim() == "| { kind: \"inline\"; payload: { raw_value?: (string | null) } }")
    );
}
