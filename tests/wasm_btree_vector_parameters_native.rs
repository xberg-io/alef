use alef::backends::wasm::WasmBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{Language, NewAlefConfig};
use alef::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use alef::core::validation::ValidatedApiSurface;
use std::path::{Component, Path, PathBuf};

#[path = "support/mise_task.rs"]
pub(crate) mod mise_task;
mod test_support {
    pub(crate) use crate::mise_task;
}
#[path = "../src/backends/wasm/gen_bindings/functions/params_native_tests/native.rs"]
mod native;

const CORE: &str = r#"
use std::collections::{BTreeMap, HashMap};

#[derive(Clone)]
pub struct Payload { pub marker: String }
#[derive(Clone)]
pub struct Packet {
    pub flags: Vec<bool>,
    pub rows: Vec<BTreeMap<String, serde_json::Value>>,
    pub maybe_rows: Option<Vec<BTreeMap<String, serde_json::Value>>>,
}
pub fn echo_packet(packet: Packet) -> Packet { packet }

pub fn vectors(payload: Payload, records: Vec<BTreeMap<String, serde_json::Value>>) -> u32 {
    (payload.marker.len() + records.len()) as u32
}
pub fn optionalvectors(payload: Payload, records: Option<Vec<BTreeMap<String, serde_json::Value>>>) -> u32 {
    (payload.marker.len() + records.map(|rows| rows.len()).unwrap_or(0)) as u32
}
pub fn nestedroot(payload: Payload, records: BTreeMap<String, BTreeMap<String, serde_json::Value>>) -> u32 {
    (payload.marker.len() + records.len()) as u32
}
pub fn optionalroot(payload: Payload, records: Option<BTreeMap<String, serde_json::Value>>) -> u32 {
    (payload.marker.len() + records.map(|rows| rows.len()).unwrap_or(0)) as u32
}
pub fn root(payload: Payload, records: BTreeMap<String, serde_json::Value>) -> u32 {
    (payload.marker.len() + records.len()) as u32
}
pub fn hashroot(payload: Payload, records: HashMap<String, serde_json::Value>) -> u32 {
    (payload.marker.len() + records.len()) as u32
}
pub fn optionalhashroot(payload: Payload, records: Option<HashMap<String, serde_json::Value>>) -> u32 {
    (payload.marker.len() + records.map(|rows| rows.len()).unwrap_or(0)) as u32
}
"#;

#[test]
#[ignore = "requires installed Rust wasm32 target and cached offline WASM dependencies"]
fn named_dto_beside_vector_maps_compiles_for_wasm32() {
    let directory = tempfile::tempdir().expect("isolated named DTO/vector map fixture");
    let root = directory.path();
    write_inputs(root);
    let protected = emit(root);
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
        "complete canonical named DTO/vector map wasm32 bindings failed\nstdout:\n{}\nstderr:\n{}\nGENERATED RUST:\n{}",
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

fn emit(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let source = root.join("crates/map-core/src/lib.rs");
    let api = alef::extract::extractor::extract(&[source.as_path()], "map-core", "0.1.0", Some(root)).unwrap();
    assert_shapes(&api);
    ValidatedApiSurface::new(&api, &[]).expect("unsuppressed public extraction validation");
    let text = std::fs::read_to_string(root.join("alef.toml")).unwrap();
    let raw: NewAlefConfig = toml::from_str(&text).unwrap();
    let mut config = raw.resolve().unwrap().remove(0);
    config.workspace_root = Some(root.to_path_buf());
    assert!(config.components.is_empty());
    let scaffold = alef::scaffold::scaffold(&api, &config, &[Language::Wasm]).unwrap();
    assert!(scaffold.len() >= 2);
    for file in &scaffold {
        write(root, file);
    }
    let generated = WasmBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(generated.len(), 2, "complete backend Rust and manifest required");
    for file in &generated {
        write(root, file);
    }
    assert_wrappers(root);
    protected_files(root)
}

fn assert_shapes(api: &ApiSurface) {
    assert_eq!((api.types.len(), api.enums.len(), api.functions.len()), (2, 0, 8));
    let payload = api.types.iter().find(|typ| typ.name == "Payload").unwrap();
    assert_eq!(payload.name, "Payload");
    assert!(!payload.is_opaque);
    assert_eq!(payload.fields.len(), 1);
    assert_eq!(payload.fields[0].ty, TypeRef::String);
    assert_eq!(
        api.functions
            .iter()
            .map(|function| function.params.len())
            .sum::<usize>(),
        15
    );
    let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    let maps = TypeRef::Vec(Box::new(map.clone()));
    let packet = api.types.iter().find(|typ| typ.name == "Packet").unwrap();
    assert!(!packet.is_opaque);
    assert_eq!(packet.fields.len(), 3);
    assert_eq!(
        packet.fields[0].ty,
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::Bool)))
    );
    assert_eq!(packet.fields[1].ty, maps);
    assert_eq!(packet.fields[2].ty, maps);
    assert!(!packet.fields[1].optional);
    assert!(packet.fields[2].optional);
    let nested = TypeRef::Map(Box::new(TypeRef::String), Box::new(map.clone()));
    for (name, shape, optional, btree) in [
        ("vectors", &maps, false, false),
        ("optionalvectors", &maps, true, false),
        ("nestedroot", &nested, false, true),
        ("optionalroot", &map, true, true),
        ("root", &map, false, true),
        ("hashroot", &map, false, false),
        ("optionalhashroot", &map, true, false),
    ] {
        let function = api.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.params.len(), 2);
        assert_eq!(function.params[0].ty, TypeRef::Named("Payload".into()));
        assert_eq!(&function.params[1].ty, shape, "{name}");
        assert_eq!(function.params[1].optional, optional, "{name}");
        assert_eq!(function.params[1].map_is_btree, btree, "{name}");
    }
}

fn assert_wrappers(root: &Path) {
    let source = std::fs::read_to_string(root.join("crates/map-core-wasm/src/lib.rs")).unwrap();
    let parsed = syn::parse_file(&source).expect("complete emitted WASM Rust must parse");
    for name in [
        "vectors",
        "optionalvectors",
        "nestedroot",
        "optionalroot",
        "root",
        "hashroot",
        "optionalhashroot",
        "echo_packet",
    ] {
        let count = parsed
            .items
            .iter()
            .filter(|item| match item {
                syn::Item::Fn(function) => function.sig.ident == name,
                _ => false,
            })
            .count();
        assert_eq!(count, 1, "actual emitted wrapper census: {name}");
    }
    assert_bool_vector_constructor(&parsed);
}

fn assert_bool_vector_constructor(parsed: &syn::File) {
    let constructors: Vec<_> = parsed
        .items
        .iter()
        .filter_map(|item| {
            let syn::Item::Impl(implementation) = item else {
                return None;
            };
            let syn::Type::Path(typ) = implementation.self_ty.as_ref() else {
                return None;
            };
            if !typ.path.is_ident("WasmPacket") || implementation.trait_.is_some() {
                return None;
            }
            implementation.items.iter().find_map(|item| match item {
                syn::ImplItem::Fn(function) if function.sig.ident == "new" => Some(&function.sig),
                _ => None,
            })
        })
        .collect();
    assert_eq!(
        constructors.len(),
        1,
        "the DTO must retain its real generated constructor"
    );
    assert_eq!(constructors[0].inputs.len(), 3);
    let syn::FnArg::Typed(flags) = &constructors[0].inputs[0] else {
        panic!("flags constructor input")
    };
    let syn::Type::Path(flags_type) = flags.ty.as_ref() else {
        panic!("mapped flags constructor input")
    };
    assert_eq!(flags_type.path.segments.last().unwrap().ident, "JsValue");
}

fn write_inputs(root: &Path) {
    let core = root.join("crates/map-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(core.join("src/lib.rs"), CORE).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname='map-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nserde_json='1'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver='3'\nmembers=['crates/map-core','crates/map-core-wasm']\n",
    )
    .unwrap();
    std::fs::write(root.join("alef.toml"), "[workspace]\nlanguages=['wasm']\n[[crates]]\nname='map-core'\nsources=['crates/map-core/src/lib.rs']\n[crates.wasm]\ntargets=['nodejs']\n").unwrap();
}

fn write(root: &Path, file: &GeneratedFile) {
    assert!(file.path.is_relative());
    assert!(!file.path.components().any(|part| part == Component::ParentDir));
    assert_ne!(file.path, Path::new("Cargo.toml"));
    let path = root.join(&file.path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, file.content.as_bytes()).unwrap();
}

fn protected_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let names = [
        "Cargo.toml",
        "alef.toml",
        ".cargo/config.toml",
        "crates/map-core/Cargo.toml",
        "crates/map-core/src/lib.rs",
        "crates/map-core-wasm/Cargo.toml",
        "crates/map-core-wasm/src/lib.rs",
    ];
    assert_eq!(names.len(), 7);
    names
        .into_iter()
        .map(|name| {
            let path = root.join(name);
            let bytes = std::fs::read(&path).unwrap();
            assert!(!bytes.is_empty(), "nonempty canonical/core file: {name}");
            (path, bytes)
        })
        .collect()
}
