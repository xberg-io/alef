use super::*;

#[test]
fn test_generate_docs_empty_api() {
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };
    let config = make_test_config();

    let files = generate_docs(&api, &config, &[Language::Python], "docs").unwrap();
    // 1 lang + configuration.md + types.md + errors.md
    assert_eq!(files.len(), 4);
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("Python API Reference"));
    assert!(lang_file.content.contains("v0.1.0"));
}

#[test]
fn test_generate_docs_respects_language_excludes() {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.python]
exclude_functions = ["interact"]
exclude_types = ["InteractionResult"]

[crates.ffi]
exclude_functions = ["ffi_only"]
exclude_types = ["FfiHidden"]
"#,
    );
    let mut api = make_minimal_api("1.2.3");
    api.functions = vec![
        make_function("interact", vec![], TypeRef::Unit, false, None),
        make_function("scrape", vec![], TypeRef::Unit, false, None),
        make_function("ffi_only", vec![], TypeRef::Unit, false, None),
    ];
    api.types = vec![empty_type("InteractionResult"), empty_type("FfiHidden")];

    let files = generate_docs(&api, &config, &[Language::Python, Language::Go], "out").unwrap();
    let python = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    let go = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-go"))
        .unwrap();

    assert!(!python.content.contains("interact()"));
    assert!(python.content.contains("scrape()"));
    assert!(!python.content.contains("InteractionResult"));
    assert!(!go.content.contains("ffi_only()"));
    assert!(!go.content.contains("FfiHidden"));
    assert!(go.content.contains("Interact()"));
}

#[test]
fn test_generate_docs_produces_one_file_per_language_plus_three_shared() {
    let api = make_minimal_api("1.2.3");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python, Language::Node], "out").unwrap();
    // 2 language files + configuration.md + types.md + errors.md
    assert_eq!(files.len(), 5);
    let paths: Vec<&str> = files.iter().map(|f| f.path.to_str().unwrap()).collect();
    assert!(paths.iter().any(|p| p.contains("api-python")));
    assert!(paths.iter().any(|p| p.contains("api-typescript")));
    assert!(paths.iter().any(|p| p.contains("configuration")));
    assert!(paths.iter().any(|p| p.contains("types")));
    assert!(paths.iter().any(|p| p.contains("errors")));
}

#[test]
fn test_generate_docs_all_output_files_end_with_newline() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    for file in &files {
        assert!(
            file.content.ends_with('\n'),
            "file {:?} must end with trailing newline",
            file.path
        );
    }
}

#[test]
fn test_generate_docs_output_dir_prefix_in_all_paths() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "custom/output/dir").unwrap();
    for file in &files {
        assert!(
            file.path.to_str().unwrap().starts_with("custom/output/dir"),
            "all paths must be under output_dir: {:?}",
            file.path
        );
    }
}

#[test]
fn test_generate_docs_multiple_languages_produce_correct_slugs() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let langs = [
        Language::Python,
        Language::Node,
        Language::Go,
        Language::Java,
        Language::Ruby,
    ];
    let expected_slugs = ["api-python", "api-typescript", "api-go", "api-java", "api-ruby"];
    let files = generate_docs(&api, &config, &langs, "docs/api").unwrap();
    // 5 lang files + 3 shared
    assert_eq!(files.len(), 8);
    for slug in &expected_slugs {
        assert!(
            files.iter().any(|f| f.path.to_str().unwrap().contains(slug)),
            "expected file with slug {slug}"
        );
    }
}
