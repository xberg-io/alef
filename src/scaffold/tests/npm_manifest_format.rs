use super::*;

fn assert_expanded_json_collections(file: &GeneratedFile) {
    let document: serde_json::Value = serde_json::from_str(&file.content).expect("npm manifest must remain valid JSON");
    assert!(
        document["name"].is_string(),
        "{} must be an npm manifest",
        file.path.display()
    );
    for line in file.content.lines() {
        assert!(
            !line.contains("\": [") || line.trim_end().ends_with('['),
            "{} must put array entries on separate lines: {line}",
            file.path.display()
        );
        assert!(
            !line.contains("\": { "),
            "{} must expand inline objects: {line}",
            file.path.display()
        );
    }
}

#[test]
fn should_expand_all_nine_node_manifests_with_and_without_publish_metadata() {
    let mut rich = test_config();
    rich.scaffold.as_mut().unwrap().authors = vec!["Primary".into(), "Contributor".into(), "Other".into()];
    for config in [minimal_config_from_toml(""), rich] {
        let files = scaffold(&test_api(), &config, &[Language::Node]).unwrap();
        let manifests: Vec<_> = files
            .iter()
            .filter(|file| file.path.ends_with("package.json"))
            .collect();
        assert_eq!(
            manifests.len(),
            9,
            "root package and all eight platform packages must be checked"
        );
        for file in manifests {
            assert_expanded_json_collections(file);
            if config.scaffold.is_some() {
                let document: serde_json::Value = serde_json::from_str(&file.content).unwrap();
                assert_eq!(document["contributors"], serde_json::json!(["Contributor", "Other"]));
                assert_eq!(document["keywords"], serde_json::json!(["test"]));
            }
        }
    }
}

#[test]
fn should_expand_the_node_files_array_when_service_exports_are_enabled() {
    let mut api = test_api();
    api.services.push(crate::core::ir::ServiceDef {
        name: "App".into(),
        rust_path: "my_lib::App".into(),
        constructor: crate::core::ir::MethodDef {
            name: "new".into(),
            ..Default::default()
        },
        configurators: vec![],
        registrations: vec![],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    });
    let files = scaffold(&api, &test_config(), &[Language::Node]).unwrap();
    let root = files
        .iter()
        .find(|file| file.path == Path::new("crates/my-lib-node/package.json"))
        .unwrap();
    assert_expanded_json_collections(root);
    let document: serde_json::Value = serde_json::from_str(&root.content).unwrap();
    assert_eq!(document["files"].as_array().unwrap().len(), 7);
    assert_eq!(document["exports"]["./service"]["require"], "./service.cjs");
}

#[test]
fn should_expand_wasm_files_for_single_and_multiple_targets() {
    for config in [test_config(), test_config_from_toml("[crates.wasm]\ntargets=['web']\n")] {
        let files = scaffold(&test_api(), &config, &[Language::Wasm]).unwrap();
        let manifests: Vec<_> = files
            .iter()
            .filter(|file| file.path.ends_with("package.json"))
            .collect();
        assert_eq!(manifests.len(), 1, "the WASM root manifest must be checked");
        assert_expanded_json_collections(manifests[0]);
        let document: serde_json::Value = serde_json::from_str(&manifests[0].content).unwrap();
        let count = if config.wasm_targets().len() == 1 { 2 } else { 4 };
        assert_eq!(document["files"].as_array().unwrap().len(), count);
    }
}

#[test]
#[should_panic(expected = "must put array entries on separate lines")]
fn should_reject_a_valid_manifest_with_a_compact_array() {
    assert_expanded_json_collections(&GeneratedFile {
        path: "package.json".into(),
        content: r#"{"name": "sample", "files": ["index.js"]}"#.into(),
        generated_header: false,
    });
}

#[test]
#[should_panic(expected = "must expand inline objects")]
fn should_reject_a_valid_manifest_with_a_compact_object() {
    assert_expanded_json_collections(&GeneratedFile {
        path: "package.json".into(),
        content: r#"{"name": "sample", "engines": { "node": ">=18" }}"#.into(),
        generated_header: false,
    });
}
