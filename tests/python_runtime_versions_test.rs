use alef::core::config::{Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef};
use alef::scaffold::scaffold;
use semver::VersionReq;

fn dependency_minor(manifest: &toml::Value, dependency: &str) -> (u64, u64) {
    let requirement = manifest["dependencies"][dependency]["version"]
        .as_str()
        .expect("generated Python runtime dependency has a version");
    let parsed = VersionReq::parse(requirement).expect("generated requirement is valid semver");
    assert_eq!(
        parsed.comparators.len(),
        1,
        "expected one comparator for {dependency}: {requirement}"
    );
    let comparator = &parsed.comparators[0];
    (
        comparator.major,
        comparator.minor.expect("runtime requirement specifies a minor version"),
    )
}

#[test]
fn should_generate_matching_pyo3_and_async_runtime_minor_versions() {
    let api = ApiSurface {
        crate_name: "sample-lib".into(),
        version: "0.1.0".into(),
        functions: vec![FunctionDef {
            name: "fetch".into(),
            rust_path: "sample_lib::fetch".into(),
            is_async: true,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".into(),
        languages: vec![Language::Python],
        ..ResolvedCrateConfig::default()
    };
    let files = scaffold(&api, &config, &[Language::Python]).expect("Python scaffold succeeds");
    let manifests: Vec<_> = files.iter().filter(|file| file.path.ends_with("Cargo.toml")).collect();
    assert_eq!(manifests.len(), 1, "expected exactly one Python binding Cargo manifest");
    let manifest: toml::Value = toml::from_str(&manifests[0].content).expect("generated Cargo manifest parses");
    assert_eq!(
        dependency_minor(&manifest, "pyo3"),
        dependency_minor(&manifest, "pyo3-async-runtimes"),
        "PyO3 and its async runtime must share a minor version to avoid conflicting native links\n{}",
        manifests[0].content,
    );

    let sync_api = ApiSurface {
        functions: vec![],
        ..api
    };
    let files = scaffold(&sync_api, &config, &[Language::Python]).expect("Python scaffold succeeds");
    let manifest = files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .expect("Python binding Cargo manifest");
    let manifest: toml::Value = toml::from_str(&manifest.content).expect("generated Cargo manifest parses");
    assert!(manifest["dependencies"].get("pyo3-async-runtimes").is_none());
}
