//! Whole-producer regressions for compatible NAPI container and serde repairs.

mod fixtures;
mod probe;

use super::NapiBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::BTreeSet;

fn config() -> ResolvedCrateConfig {
    let source = "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n";
    toml::from_str::<NewAlefConfig>(source)
        .expect("fixture configuration")
        .resolve()
        .expect("resolved configuration")
        .remove(0)
}

fn rust_bindings(api: &ApiSurface) -> String {
    let files = NapiBackend
        .generate_bindings(api, &config())
        .expect("complete NAPI producer");
    let matches: Vec<_> = files.iter().filter(|file| file.path.ends_with("lib.rs")).collect();
    assert_eq!(matches.len(), 1, "one complete Rust binding module must be rendered");
    matches[0].content.clone()
}

fn run_core(source: &str, driver: &str) -> (ApiSurface, String) {
    let api = probe::extract(source);
    let generated = rust_bindings(&api);
    let source = probe::conversion_source(&generated, source, driver);
    let output = probe::run(&source);
    (api, output)
}

#[test]
fn output_only_adjacent_enum_without_default_compiles_its_emitted_conversions() {
    let api = probe::extract(fixtures::NO_DEFAULT);
    assert_eq!(api.enums.len(), 1);
    assert!(
        !api.enums[0].has_default,
        "the core enum must not accidentally derive Default"
    );
    assert_eq!(api.functions.len(), 1);
    assert!(
        api.functions[0].params.is_empty(),
        "this must exercise the output-only route"
    );
    let generated = rust_bindings(&api);
    let driver = r#"fn main() {
        let original = test_lib::Choice::Right("kept".into());
        let binding: JsChoice = original.clone().into();
        let restored: test_lib::Choice = binding.try_into().unwrap();
        assert_eq!(restored, original);
    }"#;
    probe::run(&probe::conversion_source(&generated, fixtures::NO_DEFAULT, driver));
}

fn declared_keys(files: &[GeneratedFile]) -> (String, BTreeSet<String>) {
    let matches: Vec<_> = files.iter().filter(|file| file.path.ends_with("index.d.ts")).collect();
    assert_eq!(matches.len(), 1, "the actual public DTS overlay must be measured");
    let declaration = &matches[0].content;
    let properties = regex::Regex::new(r#"(?:^|[\s{;])(?:"([^"]+)"|'([^']+)'|([A-Za-z_$][A-Za-z0-9_$]*))\??\s*:"#)
        .expect("property signature regex");
    let keys = properties
        .captures_iter(declaration)
        .map(|capture| {
            capture
                .get(1)
                .or_else(|| capture.get(2))
                .or_else(|| capture.get(3))
                .expect("one property spelling")
                .as_str()
                .to_string()
        })
        .collect();
    (declaration.clone(), keys)
}

fn actual_json_keys(value: &serde_json::Value, keys: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                keys.insert(key.clone());
                actual_json_keys(value, keys);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                actual_json_keys(value, keys);
            }
        }
        _ => {}
    }
}

#[test]
fn adjacent_inline_and_named_payload_declarations_match_actual_runtime_keys() {
    let (api, output) = run_core(fixtures::NAMING, fixtures::NAMING_MAIN);
    let files = NapiBackend
        .generate_type_stubs(&api, &config())
        .expect("complete DTS producer");
    let (declaration, declared) = declared_keys(&files);
    let values: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).expect("actual runtime JSON"))
        .collect();
    assert_eq!(
        values.len(),
        3,
        "inline, ordinary named, and rename_all named payloads must run"
    );
    let mut actual = BTreeSet::new();
    for value in &values {
        actual_json_keys(value, &mut actual);
        let tag = value
            .as_object()
            .expect("adjacent object")
            .values()
            .find_map(serde_json::Value::as_str)
            .expect("variant tag");
        assert!(
            declaration.contains(&format!("\"{tag}\"")) || declaration.contains(&format!("'{tag}'")),
            "variant serde spelling must be preserved: {tag}\n{declaration}"
        );
    }
    assert!(
        actual.len() >= 5 && declared.len() >= 5,
        "both runtime and declaration must contain actual properties"
    );
    let missing: Vec<_> = actual.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "runtime properties absent from actual DTS: {missing:?}\n{declaration}"
    );
}

fn assert_extracted_optional(api: &ApiSurface, nested: bool) {
    let packet = api
        .types
        .iter()
        .find(|item| item.name == "Packet")
        .expect("Packet extracted");
    assert_eq!(packet.fields.len(), 1);
    let field = &packet.fields[0];
    assert!(field.optional, "the real extractor unwraps the outer Option");
    let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    let vector = TypeRef::Vec(Box::new(map));
    let expected = if nested {
        TypeRef::Optional(Box::new(vector))
    } else {
        vector
    };
    assert_eq!(
        field.ty, expected,
        "the actual source composition must survive extraction"
    );
}

#[test]
fn ordinary_optional_json_map_vector_round_trips_after_actual_extraction() {
    let api = probe::extract(fixtures::ORDINARY_OPTIONAL);
    assert_extracted_optional(&api, false);
    let driver = [fixtures::JSON_MAPS, fixtures::ORDINARY_MAIN].join("\n");
    run_core(fixtures::ORDINARY_OPTIONAL, &driver);
}

#[test]
fn nested_optional_json_map_vector_keeps_configured_values_in_both_directions() {
    let api = probe::extract(fixtures::NESTED_OPTIONAL);
    assert_extracted_optional(&api, true);
    let driver = [fixtures::JSON_MAPS, fixtures::NESTED_MAIN].join("\n");
    run_core(fixtures::NESTED_OPTIONAL, &driver);
}

#[test]
fn enum_keyed_maps_round_trip_serde_names_values_and_optional_containers() {
    let api = probe::extract(fixtures::ENUM_MAPS);
    assert_eq!(api.enums.len(), 1);
    let role = &api.enums[0];
    assert_eq!(role.serde_rename_all.as_deref(), Some("kebab-case"));
    assert!(
        role.variants
            .iter()
            .any(|variant| variant.serde_rename.as_deref() == Some("backup-source"))
    );
    let files = NapiBackend
        .generate_type_stubs(&api, &config())
        .expect("complete DTS producer");
    let (declaration, _) = declared_keys(&files);
    assert!(
        declaration.contains("records: Record<string, Entry>"),
        "map key declarations must match the actual String-key boundary: {declaration}"
    );
    assert!(
        declaration.contains("groups: Array<Record<string, Entry>>"),
        "nested map key declarations must match the actual String-key boundary: {declaration}"
    );
    run_core(fixtures::ENUM_MAPS, fixtures::ENUM_MAPS_MAIN);
}

#[test]
fn malformed_adjacent_enum_input_returns_an_error_without_panicking() {
    let driver = r#"fn main() {
        let outcome = std::panic::catch_unwind(|| {
            test_lib::Choice::try_from(JsChoice(serde_json::json!({"invalid": true})))
        });
        assert!(outcome.is_ok(), "malformed enum input must return an error rather than panic");
        assert!(outcome.unwrap().is_err(), "malformed enum input must not silently default");
    }"#;
    run_core(fixtures::NAMING, driver);
}

#[test]
fn invalid_enum_map_key_returns_an_error_without_panicking() {
    let driver = r#"fn main() {
        let core = test_lib::Envelope {
            records: Default::default(), raw: Default::default(), maybe: None, groups: Vec::new(),
        };
        let mut binding: JsEnvelope = core.into();
        binding.records.insert("not-a-role".into(), JsEntry { text: "must-not-default".into() });
        let outcome = std::panic::catch_unwind(|| test_lib::Envelope::try_from(binding));
        assert!(outcome.is_ok(), "invalid map key must return an error rather than panic");
        assert!(outcome.unwrap().is_err(), "invalid map key must not silently default");
    }"#;
    run_core(fixtures::ENUM_MAPS, driver);
}

#[test]
fn fallible_enum_inputs_preserve_arc_field_and_vector_ownership() {
    let source = r#"
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum Choice { Left(String), Right(String) }
#[derive(Clone, Debug, PartialEq)]
pub struct Owner {
    pub value: std::sync::Arc<Choice>,
    pub values: Vec<std::sync::Arc<Choice>>,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            value: std::sync::Arc::new(Choice::Left("default scalar".into())),
            values: vec![std::sync::Arc::new(Choice::Right("default vector".into()))],
        }
    }
}
pub fn echo(owner: Owner) -> Owner { owner }
"#;
    let api = probe::extract(source);
    let owner = api
        .types
        .iter()
        .find(|typ| typ.name == "Owner")
        .expect("actual Owner extraction");
    assert!(owner.has_default, "the real Default-owner builder must run");
    assert_eq!(owner.fields[0].core_wrapper, crate::core::ir::CoreWrapper::Arc);
    assert_eq!(
        owner.fields[1].vec_inner_core_wrapper,
        crate::core::ir::CoreWrapper::Arc
    );
    let driver = r#"fn main() {
        let original = test_lib::Owner {
            value: std::sync::Arc::new(test_lib::Choice::Left("one".into())),
            values: vec![std::sync::Arc::new(test_lib::Choice::Right("two".into()))],
        };
        let binding: JsOwner = original.clone().into();
        let restored = test_lib::Owner::try_from(binding).unwrap();
        assert_eq!(restored, original);
        let omitted = JsOwner { value: None, values: None };
        assert_eq!(test_lib::Owner::try_from(omitted).unwrap(), test_lib::Owner::default());
    }"#;
    run_core(source, driver);
}

#[test]
fn infallible_callback_rejects_a_fallible_return_without_a_default() {
    let mut api = probe::extract(fixtures::ENUM_MAPS);
    let mut contract = crate::core::ir::TypeDef {
        name: "Provider".into(),
        rust_path: "test_lib::Provider".into(),
        is_trait: true,
        methods: vec![crate::core::ir::MethodDef {
            name: "provide".into(),
            return_type: TypeRef::Named("Envelope".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        }],
        ..Default::default()
    };
    api.types.push(contract.clone());
    let bridge: crate::core::config::TraitBridgeConfig =
        toml::from_str("trait_name = 'Provider'").expect("bridge fixture");
    let result = crate::backends::napi::trait_bridge::gen_trait_bridge(
        &contract,
        &bridge,
        "test_lib",
        "TestError",
        "TestError::Message { message: {msg} }",
        &api,
    );
    let error = result
        .err()
        .expect("an infallible callback cannot report failed input conversion without a fallback");
    assert!(error.to_string().contains("Provider.provide") && error.to_string().contains("Envelope"));
    contract.methods[0].error_type = Some("TestError".into());
    let supported = crate::backends::napi::trait_bridge::gen_trait_bridge(
        &contract,
        &bridge,
        "test_lib",
        "TestError",
        "TestError::Message { message: {msg} }",
        &api,
    )
    .expect("the same callback can propagate a typed error when its contract is fallible");
    assert!(supported.code.contains("test_lib::Envelope::try_from(__decoded)"));
}
