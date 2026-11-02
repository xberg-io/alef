use crate::backends::wasm::WasmBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{Language, NewAlefConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::validation::ValidatedApiSurface;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy)]
pub(super) enum Case {
    Maps,
    NamedComposition,
}

pub(super) fn emit(root: &Path, source: &str, case: Case) -> Vec<(PathBuf, Vec<u8>)> {
    write_inputs(root, source);
    let source_path = root.join("crates/map-core/src/lib.rs");
    let api = crate::extract::extractor::extract(&[source_path.as_path()], "map-core", "0.1.0", Some(root)).unwrap();
    let expected_functions: &[&str] = match case {
        Case::Maps => {
            assert_map_shapes(&api);
            &[
                "records",
                "optionalrecords",
                "batches",
                "optionalbatches",
                "hashmap",
                "ping",
            ]
        }
        Case::NamedComposition => {
            assert_named_composition(&api);
            &["combine"]
        }
    };
    ValidatedApiSurface::new(&api, &[]).expect("unsuppressed public extraction validation");
    let config_text = std::fs::read_to_string(root.join("alef.toml")).unwrap();
    let raw: NewAlefConfig = toml::from_str(&config_text).unwrap();
    let mut config = raw.resolve().unwrap().remove(0);
    config.workspace_root = Some(root.to_path_buf());
    assert!(config.components.is_empty());
    let scaffold = crate::scaffold::scaffold(&api, &config, &[Language::Wasm]).unwrap();
    assert!(scaffold.len() >= 2);
    for file in &scaffold {
        write(root, file);
    }
    let generated = WasmBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(generated.len(), 2, "complete backend Rust and manifest required");
    for file in &generated {
        write(root, file);
    }
    assert_emitted_functions(root, expected_functions);
    protected_files(root)
}

fn assert_named_composition(api: &ApiSurface) {
    assert_eq!((api.types.len(), api.enums.len(), api.functions.len()), (1, 0, 1));
    let payload = &api.types[0];
    assert_eq!(payload.name, "Payload");
    assert!(
        !payload.is_opaque,
        "nonopaque named input must select named-let argument conversion"
    );
    assert_eq!(payload.fields.len(), 1);
    assert_eq!(payload.fields[0].ty, TypeRef::String);
    let function = &api.functions[0];
    assert_eq!(function.name, "combine");
    assert_eq!(function.params.len(), 2);
    assert_eq!(function.params[0].ty, TypeRef::Named("Payload".into()));
    assert_eq!(
        function.params[1].ty,
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json))
    );
    assert!(function.params[1].map_is_btree);
    assert!(!function.params[1].optional);
}

fn assert_map_shapes(api: &ApiSurface) {
    assert_eq!((api.types.len(), api.enums.len(), api.functions.len()), (0, 0, 6));
    assert_eq!(
        api.functions
            .iter()
            .map(|function| function.params.len())
            .sum::<usize>(),
        5
    );
    let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    for (name, vector, optional, btree) in [
        ("records", false, false, true),
        ("optionalrecords", false, true, true),
        ("batches", true, false, false),
        ("optionalbatches", true, true, false),
        ("hashmap", false, false, false),
    ] {
        let function = api.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.params.len(), 1);
        let parameter = &function.params[0];
        let expected = if vector {
            TypeRef::Vec(Box::new(map.clone()))
        } else {
            map.clone()
        };
        assert_eq!(parameter.ty, expected, "actual extracted container: {name}");
        assert_eq!(
            (parameter.optional, parameter.map_is_btree),
            (optional, btree),
            "{name}"
        );
    }
}

fn write_inputs(root: &Path, source: &str) {
    let core = root.join("crates/map-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(core.join("src/lib.rs"), source).unwrap();
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
    assert_ne!(
        file.path,
        Path::new("Cargo.toml"),
        "outer workspace is authored fixture input"
    );
    let path = root.join(&file.path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, file.content.as_bytes()).unwrap();
}

fn assert_emitted_functions(root: &Path, expected: &[&str]) {
    let source = std::fs::read_to_string(root.join("crates/map-core-wasm/src/lib.rs")).unwrap();
    let parsed = syn::parse_file(&source).expect("complete emitted WASM Rust must parse");
    let actual: Vec<_> = parsed
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if expected.iter().any(|name| function.sig.ident == *name) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "all actual public wrappers must be emitted"
    );
    for &name in expected {
        assert_eq!(
            actual.iter().filter(|actual| actual.as_str() == name).count(),
            1,
            "{name}"
        );
    }
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
