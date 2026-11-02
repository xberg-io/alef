#[path = "params_native_tests/fixture.rs"]
mod fixture;
#[path = "params_native_tests/native.rs"]
mod native;

const CORE: &str = r#"
use std::collections::{BTreeMap, HashMap};

pub fn records(value: BTreeMap<String, serde_json::Value>) -> BTreeMap<String, serde_json::Value> { value }
pub fn optionalrecords(value: Option<BTreeMap<String, serde_json::Value>>) -> Option<BTreeMap<String, serde_json::Value>> { value }
pub fn batches(value: Vec<BTreeMap<String, serde_json::Value>>) -> Vec<BTreeMap<String, serde_json::Value>> { value }
pub fn optionalbatches(value: Option<Vec<BTreeMap<String, serde_json::Value>>>) -> Option<Vec<BTreeMap<String, serde_json::Value>>> { value }
pub fn hashmap(value: HashMap<String, serde_json::Value>) -> HashMap<String, serde_json::Value> { value }
pub fn ping() -> u32 { 42 }
"#;

#[test]
#[ignore = "requires installed Rust wasm32 target and cached offline WASM dependencies"]
fn btree_map_parameter_shapes_compile_for_wasm32() {
    let directory = tempfile::tempdir().expect("isolated WASM map fixture");
    let root = directory.path();
    let protected = fixture::emit(root, CORE, fixture::Case::Maps);
    let output = native::build(root);
    for (path, before) in &protected {
        assert_eq!(
            std::fs::read(path).unwrap(),
            *before,
            "canonical/core bytes changed: {}",
            path.display()
        );
    }
    assert!(
        output.status.success(),
        "complete canonical wasm32 map bindings failed\nstdout:\n{}\nstderr:\n{}\nGENERATED RUST:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        std::fs::read_to_string(root.join("crates/map-core-wasm/src/lib.rs")).unwrap()
    );
    let module = std::fs::read(root.join("target/wasm32-unknown-unknown/debug/map_core_wasm.wasm")).unwrap();
    assert!(module.len() > 8);
    assert_eq!(
        &module[..8],
        &[0, 97, 115, 109, 1, 0, 0, 0],
        "actual WASM module required"
    );
}

const NAMED_COMPOSITION: &str = r#"
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Payload { pub marker: String }

pub fn combine(payload: Payload, records: BTreeMap<String, serde_json::Value>) -> u32 {
    (payload.marker.len() + records.len()) as u32
}
"#;

#[test]
#[ignore = "requires installed Rust wasm32 target and cached offline WASM dependencies"]
fn named_dto_beside_btree_map_compiles_for_wasm32() {
    let directory = tempfile::tempdir().expect("isolated named DTO and map fixture");
    let root = directory.path();
    let protected = fixture::emit(root, NAMED_COMPOSITION, fixture::Case::NamedComposition);
    let output = native::build(root);
    for (path, before) in &protected {
        assert_eq!(
            std::fs::read(path).unwrap(),
            *before,
            "canonical/core bytes changed: {}",
            path.display()
        );
    }
    assert!(
        output.status.success(),
        "complete canonical mixed DTO/map wasm32 bindings failed\nstdout:\n{}\nstderr:\n{}\nGENERATED RUST:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        std::fs::read_to_string(root.join("crates/map-core-wasm/src/lib.rs")).unwrap()
    );
    let module = std::fs::read(root.join("target/wasm32-unknown-unknown/debug/map_core_wasm.wasm")).unwrap();
    assert!(module.len() > 8);
    assert_eq!(
        &module[..8],
        &[0, 97, 115, 109, 1, 0, 0, 0],
        "actual WASM module required"
    );
}
