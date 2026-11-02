use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::{Command, Output};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::enums::{GoEnumRepresentation, gen_enum_type, go_enum_representation};
use super::structs::gen_struct_type;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::test_support::toolchain;

#[path = "flatten_runtime_consumed.rs"]
mod consumed;
#[path = "flatten_runtime_optional_structs.rs"]
mod optional_structs;
#[path = "flatten_runtime_schema.rs"]
mod schema;

use schema::CollisionRecord as WireCollision;

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireChoice {
    Live { name: String },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireAdjacent {
    Live(String),
}

#[derive(Deserialize, Serialize)]
struct WireParent<T> {
    marker: String,
    #[serde(flatten)]
    payload: T,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular: Option<WireChoice>,
}

#[derive(Deserialize, Serialize)]
struct WireChild {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct WireRemainder {
    #[serde(flatten)]
    child: WireChild,
    #[serde(flatten)]
    extras: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular: Option<WireChoice>,
}

fn wire_case<T: DeserializeOwned + Serialize>(input: Value) -> Value {
    let core: T = serde_json::from_value(input.clone()).expect("the fixture must be accepted by core Serde");
    json!({"input": input, "expected": serde_json::to_value(core).expect("serialize core Serde oracle")})
}

fn oracle_cases() -> BTreeMap<String, Value> {
    let mut cases = BTreeMap::new();
    for mixed in [false, true] {
        add_parent_cases(&mut cases, mixed);
        add_remainder_cases(&mut cases, mixed);
    }
    add_collision_cases(&mut cases);
    assert_eq!(
        cases.len(),
        12,
        "every native case needs an independent core Serde oracle"
    );
    cases
}

fn add_parent_cases(cases: &mut BTreeMap<String, Value>, mixed: bool) {
    let suffix = if mixed { "mixed" } else { "only" };
    let regular = json!({"kind": "live", "name": "regular"});
    let mut input = json!({"marker": "m", "kind": "live", "name": "payload"});
    if mixed {
        input["regular"] = regular.clone();
    }
    cases.insert(format!("sealed_{suffix}"), wire_case::<WireParent<WireChoice>>(input));
    for present in [false, true] {
        let mut input = json!({"marker": "m"});
        if mixed {
            input["regular"] = regular.clone();
        }
        if present {
            input["kind"] = json!("live");
            input["value"] = json!("payload");
        }
        let state = if present { "present" } else { "absent" };
        cases.insert(
            format!("optional_{suffix}_{state}"),
            wire_case::<WireParent<Option<WireAdjacent>>>(input),
        );
    }
}

fn add_remainder_cases(cases: &mut BTreeMap<String, Value>, mixed: bool) {
    let suffix = if mixed { "mixed" } else { "only" };
    for nonempty in [false, true] {
        let mut input = json!({"tags": [], "extra": "kept"});
        if mixed {
            input["regular"] = json!({"kind": "live", "name": "regular"});
        }
        if nonempty {
            input["tags"] = json!(["owned"]);
        }
        let state = if nonempty { "nonempty" } else { "empty" };
        cases.insert(format!("remainder_{suffix}_{state}"), wire_case::<WireRemainder>(input));
    }
}

fn add_collision_cases(cases: &mut BTreeMap<String, Value>) {
    for omitted in [false, true] {
        let tags = if omitted { vec![] } else { vec!["owned".to_string()] };
        let mut extras = BTreeMap::from([("extra".to_string(), "kept".to_string())]);
        if omitted {
            extras.insert("tags".to_string(), "from-extra".to_string());
        }
        let input = json!({"tags": &tags, "extras": &extras});
        let expected = serde_json::to_value(WireCollision { tags, extras }).expect("serialize collision oracle");
        if omitted {
            assert_eq!(
                expected["tags"], "from-extra",
                "Serde permits the skipped member's name in extras"
            );
        }
        let suffix = if omitted { "omitted" } else { "nonempty" };
        cases.insert(
            format!("collision_{suffix}"),
            json!({"input": input, "expected": expected}),
        );
    }
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        ..FieldDef::default()
    }
}

fn flattened(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        serde_flatten: true,
        ..field(name, ty)
    }
}

fn defaulted_tags() -> FieldDef {
    FieldDef {
        default: Some("Default::default()".into()),
        serde_skip_serializing_if: true,
        ..field("tags", TypeRef::Vec(Box::new(TypeRef::String)))
    }
}

fn structure(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.into(),
        fields,
        has_serde: true,
        ..TypeDef::default()
    }
}

fn with_regular(mut fields: Vec<FieldDef>, mixed: bool) -> Vec<FieldDef> {
    if mixed {
        fields.push(field("regular", TypeRef::Named("RegularChoice".into())));
    }
    fields
}

fn fixture_types() -> Vec<TypeDef> {
    let mut types = vec![structure("DefaultChild", vec![defaulted_tags()])];
    types.extend(extracted_schema());
    types.extend(optional_structs::parent_types());
    types.extend(consumed::parent_types());
    for mixed in [false, true] {
        types.extend(parent_types(mixed));
    }
    types
}

fn extracted_schema() -> Vec<TypeDef> {
    let directory = tempfile::tempdir().expect("create public extraction fixture");
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, include_str!("flatten_runtime_schema.rs")).expect("write the same core Serde definitions");
    let api = crate::extract::extractor::extract(&[source.as_path()], "codec_fixture", "0.1.0", None)
        .expect("extract actual public fixture schema");
    assert_eq!(api.types.len(), 7);
    let collision = api
        .types
        .iter()
        .find(|typ| typ.name == "CollisionRecord")
        .expect("extracted collision type");
    let tags = collision
        .fields
        .iter()
        .find(|field| field.name == "tags")
        .expect("extracted tags field");
    assert!(
        tags.default.is_none(),
        "skip_serializing_if must not invent a serde default"
    );
    assert!(tags.serde_skip_serializing_if);
    assert!(!collision.serde_container_default);
    api.types
}

fn parent_types(mixed: bool) -> Vec<TypeDef> {
    let suffix = if mixed { "Mixed" } else { "Only" };
    let fields = vec![
        field("marker", TypeRef::String),
        flattened("payload", TypeRef::Named("SealedChoice".into())),
    ];
    let sealed = structure(&format!("Sealed{suffix}"), with_regular(fields, mixed));
    let mut optional = flattened(
        "payload",
        TypeRef::Optional(Box::new(TypeRef::Named("AdjacentChoice".into()))),
    );
    optional.optional = true;
    let fields = vec![field("marker", TypeRef::String), optional];
    let optional = structure(&format!("Optional{suffix}"), with_regular(fields, mixed));
    let extras = flattened(
        "extras",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    );
    let fields = vec![flattened("child", TypeRef::Named("DefaultChild".into())), extras];
    let remainder = structure(&format!("Remainder{suffix}"), with_regular(fields, mixed));
    vec![sealed, optional, remainder]
}

fn fixture_enums() -> Vec<EnumDef> {
    ["SealedChoice", "RegularChoice", "AdjacentChoice"]
        .into_iter()
        .map(fixture_enum)
        .collect()
}

fn fixture_enum(name: &str) -> EnumDef {
    let adjacent = name == "AdjacentChoice";
    let enumeration = EnumDef {
        name: name.into(),
        serde_tag: Some("kind".into()),
        serde_content: adjacent.then(|| "value".into()),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![EnumVariant {
            name: "Live".into(),
            is_tuple: adjacent,
            fields: vec![field(if adjacent { "_0" } else { "name" }, TypeRef::String)],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let expected = if adjacent {
        GoEnumRepresentation::AdjacentTaggedStruct
    } else {
        GoEnumRepresentation::DataInterface
    };
    assert_eq!(go_enum_representation(&enumeration), expected);
    enumeration
}

fn generated_codec() -> String {
    let types = fixture_types();
    let mut structs: HashSet<&str> = types.iter().map(|typ| typ.name.as_str()).collect();
    structs.insert("AdjacentChoice");
    let data_enums = HashSet::from(["SealedChoice", "RegularChoice"]);
    let mut source = String::from("package codec\n\nimport (\"encoding/json\"; \"fmt\")\n\n");
    for enumeration in fixture_enums() {
        source.push_str(&gen_enum_type(&enumeration, &[]));
    }
    for typ in &types {
        source.push_str(&gen_struct_type(
            typ,
            &HashSet::new(),
            &HashSet::new(),
            &data_enums,
            (&structs, &types),
            &[],
        ));
    }
    source
}

fn go_command(go: &Path, directory: &Path, arguments: &[&str]) -> Output {
    Command::new(go)
        .args(arguments)
        .current_dir(directory)
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GOWORK", "off")
        .output()
        .expect("execute the available native Go toolchain")
}

fn write_fixture(directory: &Path) {
    std::fs::write(directory.join("go.mod"), "module example.com/codec\n\ngo 1.24\n").expect("write local Go module");
    std::fs::write(directory.join("codec.go"), generated_codec()).expect("write actual generated codec");
    std::fs::write(
        directory.join("codec_test.go"),
        include_str!("flatten_runtime_test.go.fixture"),
    )
    .expect("write runtime contracts");
    std::fs::write(
        directory.join("oracle.json"),
        serde_json::to_vec(&oracle_cases()).unwrap(),
    )
    .expect("write core Serde oracle");
    std::fs::write(
        directory.join("optional_struct_oracle.json"),
        serde_json::to_vec(&optional_structs::oracle_cases()).unwrap(),
    )
    .expect("write optional-struct core Serde oracle");
    std::fs::write(
        directory.join("consumed_keys_oracle.json"),
        serde_json::to_vec(&consumed::oracle_cases()).unwrap(),
    )
    .expect("write consumed-key core Serde oracle");
}

fn assert_compiles(go: &Path, directory: &Path) {
    let compilation = go_command(go, directory, &["test", "-run", "^$", "./..."]);
    assert!(
        compilation.status.success(),
        "compile-only precondition failed:\n{}\n{}",
        String::from_utf8_lossy(&compilation.stdout),
        String::from_utf8_lossy(&compilation.stderr)
    );
}

#[allow(
    clippy::print_stderr,
    reason = "Expose actual native codec case counts in captured verification evidence"
)]
fn run_contract(test_name: &str, expected_cases: usize) {
    let Some(go) = toolchain::GO.open() else {
        return;
    };
    let directory = tempfile::tempdir().expect("create isolated generated codec fixture");
    let directory = directory.path();
    write_fixture(directory);
    assert_compiles(&go, directory);
    let output = go_command(
        &go,
        directory,
        &["test", "-v", "-count=1", "-run", &format!("^{test_name}$"), "./..."],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!("{stdout}");
    let count = stdout
        .lines()
        .filter(|line| line.starts_with("=== RUN") && line.contains(&format!("{test_name}/")))
        .count();
    assert_eq!(
        count, expected_cases,
        "the named Go runtime cases must actually execute: {stdout}"
    );
    assert!(
        output.status.success(),
        "generated Go violated the core Serde oracle:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn flattened_sealed_interfaces_use_the_generated_dispatcher() {
    run_contract("TestSealedFlatten", 2);
}

#[test]
fn absent_optional_flattened_enums_preserve_serde_none() {
    run_contract("TestOptionalFlatten", 4);
}

#[test]
fn omitted_named_member_allows_serde_flattened_map_serialization() {
    run_contract("TestCollisionSerialization", 2);
}

#[test]
fn flattened_children_consume_defaulted_keys_before_remaining_maps() {
    run_contract("TestRemainderFlatten", 4);
}
