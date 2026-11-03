//! Core source fixtures parsed by the real extractor and compiled by dispatched probes.

pub(super) const NO_DEFAULT: &str = r#"
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum Choice { Left(String), Right(u32) }
pub fn output_only() -> Choice { Choice::Left("kept".into()) }
"#;

pub(super) const NAMING: &str = r#"
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Payload { pub retry_count: u32 }
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct RenamedPayload { pub merge_limit: u32 }
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action_kind", content = "payload_data", rename_all = "kebab-case")]
pub enum Choice {
    #[default] Empty,
    InlineChoice { file_path: String },
    NamedPayload(Payload),
    #[serde(rename = "precise-variant")]
    Renamed(RenamedPayload),
}
pub fn output_only() -> Choice { Choice::Empty }
"#;

pub(super) const NAMING_MAIN: &str = r#"
fn main() {
    let samples = [
        test_lib::Choice::InlineChoice { file_path: "kept".into() },
        test_lib::Choice::NamedPayload(test_lib::Payload { retry_count: 7 }),
        test_lib::Choice::Renamed(test_lib::RenamedPayload { merge_limit: 9 }),
    ];
    for original in samples {
        let binding: JsChoice = original.clone().into();
        println!("{}", serde_json::to_string(&binding.0).unwrap());
        let restored: test_lib::Choice = binding.try_into().unwrap();
        assert_eq!(restored, original);
    }

}
"#;

pub(super) const ORDINARY_OPTIONAL: &str = r#"
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub fallbacks: Option<Vec<std::collections::BTreeMap<String, serde_json::Value>>>,
}
pub fn echo(packet: Packet) -> Packet { packet }
"#;

pub(super) const NESTED_OPTIONAL: &str = r#"
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub fallbacks: Option<Option<Vec<std::collections::BTreeMap<String, serde_json::Value>>>>,
}
pub fn echo(packet: Packet) -> Packet { packet }
"#;

pub(super) const JSON_MAPS: &str = r#"
fn maps() -> Vec<std::collections::BTreeMap<String, serde_json::Value>> {
    vec![std::collections::BTreeMap::from([
        ("object".into(), serde_json::json!({"nested": [1, true]})),
        ("number".into(), serde_json::json!(23)),
        ("null".into(), serde_json::Value::Null),
    ])]
}
"#;

pub(super) const ORDINARY_MAIN: &str = r#"
fn main() {
    for value in [None, Some(Vec::new()), Some(maps())] {
        let original = test_lib::Packet { fallbacks: value };
        let binding: JsPacket = original.clone().into();
        let restored: test_lib::Packet = binding.try_into().unwrap();
        assert_eq!(restored, original);
    }
}
"#;

pub(super) const NESTED_MAIN: &str = r#"
fn main() {
    let absent = test_lib::Packet { fallbacks: None };
    let explicit_null = test_lib::Packet { fallbacks: Some(None) };
    assert_eq!(serde_json::to_value(&absent).unwrap(), serde_json::to_value(&explicit_null).unwrap());
    for value in [None, Some(None), Some(Some(Vec::new())), Some(Some(maps()))] {
        let original = test_lib::Packet { fallbacks: value };
        let binding: JsPacket = original.clone().into();
        let restored: test_lib::Packet = binding.try_into().unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), serde_json::to_value(&original).unwrap());
    }
}
"#;

pub(super) const ENUM_MAPS: &str = r#"
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role { PrimaryOrigin, #[serde(rename = "backup-source")] FallbackOrigin }
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry { pub text: String }
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    pub records: std::collections::BTreeMap<Role, Entry>,
    pub raw: std::collections::BTreeMap<Role, serde_json::Value>,
    pub maybe: Option<std::collections::BTreeMap<Role, Entry>>,
    pub groups: Vec<std::collections::BTreeMap<Role, Entry>>,
}
pub fn echo(envelope: Envelope) -> Envelope { envelope }
"#;

pub(super) const ENUM_MAPS_MAIN: &str = r#"
fn main() {
    let original = test_lib::Envelope {
        records: std::collections::BTreeMap::from([
            (test_lib::Role::PrimaryOrigin, test_lib::Entry { text: "kept".into() }),
            (test_lib::Role::FallbackOrigin, test_lib::Entry { text: "backup".into() }),
        ]),
        raw: std::collections::BTreeMap::from([
            (test_lib::Role::PrimaryOrigin, serde_json::json!({"nested": [1, null]})),
        ]),
        maybe: Some(std::collections::BTreeMap::from([
            (test_lib::Role::FallbackOrigin, test_lib::Entry { text: "optional".into() }),
        ])),
        groups: vec![std::collections::BTreeMap::from([
            (test_lib::Role::PrimaryOrigin, test_lib::Entry { text: "nested".into() }),
        ])],
    };
    let binding: JsEnvelope = original.clone().into();
    let mut keys: Vec<String> = binding.records.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, vec!["backup-source".to_string(), "primary-origin".to_string()]);
    assert_eq!(binding.raw["primary-origin"], serde_json::json!({"nested": [1, null]}));
    assert_eq!(binding.maybe.as_ref().unwrap()["backup-source"].text, "optional");
    assert_eq!(binding.groups[0]["primary-origin"].text, "nested");
    let restored: test_lib::Envelope = binding.try_into().unwrap();
    assert_eq!(restored, original);
    let absent = test_lib::Envelope { records: Default::default(), raw: Default::default(), maybe: None, groups: Vec::new() };
    let binding: JsEnvelope = absent.clone().into();
    let restored: test_lib::Envelope = binding.try_into().unwrap();
    assert_eq!(restored, absent);
}
"#;
