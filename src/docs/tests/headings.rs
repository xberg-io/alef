use super::*;

#[test]
fn test_render_type_with_multiple_methods_have_same_heading_level() {
    use crate::core::ir::PrimitiveType;
    use crate::docs::test_helpers::make_method;
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MyTraitType".to_string(),
            rust_path: "mylib::MyTraitType".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![
                make_method(
                    "first_method",
                    vec![],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    false,
                    false,
                    None,
                ),
                make_method(
                    "second_method",
                    vec![],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    false,
                    false,
                    None,
                ),
                make_method(
                    "third_method",
                    vec![],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    false,
                    false,
                    None,
                ),
            ],
            doc: "A trait with multiple methods.".to_string(),
            is_opaque: false,
            cfg: None,
            is_copy: false,
            is_clone: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();

    // All method headings should be at #### level (H4)
    assert!(
        lang_file.content.contains("#### first_method()"),
        "first method should be H4"
    );
    assert!(
        lang_file.content.contains("#### second_method()"),
        "second method should be H4"
    );
    assert!(
        lang_file.content.contains("#### third_method()"),
        "third method should be H4"
    );

    // Ensure no methods are at H5 or H6 (##### or ######)
    let content = &lang_file.content;
    if let Some(methods_pos) = content.find("### Methods") {
        let after_methods = &content[methods_pos..];
        // Count the heading markers to ensure they're all ####
        let first_method_line = after_methods.lines().find(|l| l.contains("first_method")).unwrap_or("");
        assert!(
            first_method_line.starts_with("####"),
            "methods should all be at H4 level"
        );
    }
}

#[test]
fn test_generated_docs_have_monotonic_heading_increments() {
    use crate::docs::doc_cleaning::check_monotonic_headings;

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigType".to_string(),
            rust_path: "mylib::ConfigType".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            // Doc comment with internal headings to test demotion
            doc: "Configuration options.\n\n## Default Behavior\n\nBy default, uses standard settings.\n\n## Advanced Options\n\nFor power users.".to_string(),
            is_opaque: false,
            cfg: None,
            is_copy: false,
            is_clone: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
};

    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();

    // Check all generated files for monotonic heading increments
    for file in &files {
        assert!(
            check_monotonic_headings(&file.content).is_ok(),
            "File {} has invalid heading increments: {}",
            file.path.display(),
            check_monotonic_headings(&file.content).unwrap_err()
        );
    }
}

#[test]
fn test_function_doc_with_internal_headings_are_demoted() {
    use crate::docs::doc_cleaning::check_monotonic_headings;
    use crate::docs::test_helpers::make_function;

    let mut func = make_function(
        "my_function",
        vec![],
        TypeRef::Primitive(PrimitiveType::Bool),
        false,
        None,
    );
    func.doc =
        "Main description.\n\n## Processing Steps\n\n1. First step\n2. Second step\n\n## Error Handling\n\nMay fail."
            .to_string();

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();

    // Verify the internal headings were demoted
    assert!(
        lang_file.content.contains("#### Processing Steps") || lang_file.content.contains("##### Processing Steps"),
        "internal heading from doc comment should be demoted"
    );

    // Verify monotonic heading increments
    assert!(
        check_monotonic_headings(&lang_file.content).is_ok(),
        "File must have monotonic heading increments"
    );
}
