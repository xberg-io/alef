//! Scaffold coverage for crates that declare components and no binding languages.

use super::*;
use crate::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig};
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

/// A crate with `[[crates.components]]` and no binding languages scaffolds only the
/// component producer crates: no per-language package or binding crate is planned.
#[test]
fn components_only_crate_with_no_languages_scaffolds_only_producer_crates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("alef.toml");
    std::fs::write(&config_path, "").expect("write config");
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "1.2.3".into(),
        types: vec![TypeDef {
            name: "Extractor".into(),
            rust_path: "demo::Extractor".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "extract".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Bytes,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("Error".into()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "demo".into(),
        languages: Vec::new(),
        component_contracts: vec![ComponentContractConfig {
            name: "extractor".into(),
            trait_path: "demo::Extractor".into(),
            interface_version: 1,
        }],
        components: vec![ComponentConfig {
            producer_language: Default::default(),
            name: "pdf".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "extractor".into(),
                implementation: "demo::PdfExtractor".into(),
            }],
            features: vec!["pdf".into()],
            default_features: false,
            targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
            bundled_on: Vec::new(),
            source_manifest: None,
            producer_dir: None,
            extra_dependencies: Default::default(),
        }],
        ..ResolvedCrateConfig::default()
    };

    let files = scaffold_with_extensions(&api, &config, &[], &config_path, &[]).expect("components-only scaffold");

    let producer_dir = Path::new("crates/demo-pdf-component");
    let producer_files: Vec<_> = files
        .iter()
        .filter(|file| file.path.starts_with(producer_dir))
        .collect();
    assert!(
        producer_files.iter().any(|file| file.path.ends_with("Cargo.toml")),
        "producer manifest missing from {:?}",
        files.iter().map(|file| &file.path).collect::<Vec<_>>()
    );
    assert!(producer_files.iter().any(|file| file.path.ends_with("src/lib.rs")));
    let others: Vec<_> = files
        .iter()
        .filter(|file| !file.path.starts_with(producer_dir))
        .map(|file| file.path.display().to_string())
        .collect();
    assert!(
        others
            .iter()
            .all(|path| !path.starts_with("packages/") && !path.contains("-py")),
        "unexpected binding output for a components-only crate: {others:?}"
    );
}
