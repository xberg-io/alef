use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use std::path::Path;

const CORE: &str = r#"
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Record { pub value: u32 }
#[derive(Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Kind { First, Second }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum Selection { Named { value: String } }
pub struct ConfigHandle;
"#;

fn generated(root: &Path) -> String {
    let core = root.join("sample-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    let source = core.join("src/lib.rs");
    std::fs::write(&source, CORE).unwrap();
    std::fs::write(core.join("Cargo.toml"), "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nserde={version='1',features=['derive']}\n").unwrap();
    let mut api = alef::extract::extractor::extract(&[&source], "sample-core", "0.1.0", None).unwrap();
    api.types.retain(|typ| typ.name != "ConfigHandle");
    assert_eq!(api.types.len(), 1);
    assert_eq!(api.enums.len(), 2);
    let parsed: NewAlefConfig = toml::from_str("[workspace]\nlanguages=['python']\n[workspace.opaque_types]\nConfigHandle='sample_core::ConfigHandle'\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\n").unwrap();
    let config = parsed.resolve().unwrap().remove(0);
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let libraries: Vec<_> = files.iter().filter(|file| file.path.ends_with("lib.rs")).collect();
    assert_eq!(libraries.len(), 1);
    libraries[0].content.clone()
}

#[test]
fn all_cloneable_class_shapes_opt_into_extraction_and_preserve_options() {
    let directory = tempfile::tempdir().unwrap();
    let source = generated(directory.path());
    let file = syn::parse_file(&source).unwrap();
    let mut headers = std::collections::BTreeMap::new();
    for item in file.items {
        let (name, attrs) = match item {
            syn::Item::Struct(item) => (item.ident.to_string(), item.attrs),
            syn::Item::Enum(item) => (item.ident.to_string(), item.attrs),
            _ => continue,
        };
        if let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("pyclass")) {
            headers.insert(name, quote::quote!(#attr).to_string());
        }
    }
    assert_eq!(headers.len(), 4, "{headers:?}");
    for (name, options) in [
        ("Record", vec!["frozen", "from_py_object"]),
        ("Kind", vec!["eq", "eq_int", "from_py_object"]),
        ("Selection", vec!["frozen", "from_py_object"]),
        ("ConfigHandle", vec!["unsendable", "from_py_object"]),
    ] {
        for option in options {
            assert!(
                headers[name].contains(option),
                "{name} lacks {option}: {}",
                headers[name]
            );
        }
    }
}

#[test]
fn generated_cloneable_classes_compile_by_value_with_pyo3_029() {
    compile_generated("0.29");
}

#[test]
fn generated_cloneable_classes_compile_by_value_with_pyo3_030() {
    compile_generated("0.30");
}

fn compile_generated(version: &str) {
    let directory = tempfile::tempdir().unwrap();
    let mut source = generated(directory.path());
    source.push_str("\n#[pyfunction]\nfn consume_generated(_record: Record, _kind: Kind, _selection: Selection, _handle: ConfigHandle) {}\n");
    let binding = directory.path().join("sample-py");
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
    std::fs::write(binding.join("Cargo.toml"), format!("[package]\nname='sample-py'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nsample-core={{path='../sample-core'}}\npyo3={{version='{version}',features=['abi3-py310']}}\nserde={{version='1',features=['derive']}}\nserde_json='1'\n")).unwrap();
    let output = std::process::Command::new("cargo")
        .args(["check", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", directory.path().join("target"))
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "PyO3 {version}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: toml::Value = toml::from_str(&std::fs::read_to_string(binding.join("Cargo.lock")).unwrap()).unwrap();
    let resolved: Vec<_> = lock["package"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"].as_str() == Some("pyo3"))
        .collect();
    assert_eq!(resolved.len(), 1);
    assert!(
        resolved[0]["version"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{version}."))
    );
}
