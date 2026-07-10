use super::*;

fn count_md_table_cells(row: &str) -> usize {
    let trimmed = row.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);

    let mut cells = 1usize;
    let mut prev_backslash = false;
    for ch in inner.chars() {
        if ch == '|' && !prev_backslash {
            cells += 1;
        }
        prev_backslash = ch == '\\' && !prev_backslash;
    }
    cells
}

/// Verify every markdown table in `content` has consistent cell counts
/// across the header, separator, and every data row.
///
/// This guards against MD056 (table-pipe-style / table-column-count)
/// violations: rows that emit more or fewer cells than the header.
fn assert_no_md056_violations(content: &str) {
    let mut header_cells: Option<usize> = None;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        let is_table_row = trimmed.starts_with('|');
        if !is_table_row {
            header_cells = None;
            continue;
        }
        let cells = count_md_table_cells(line);
        match header_cells {
            None => header_cells = Some(cells),
            Some(expected) => {
                assert_eq!(
                    cells,
                    expected,
                    "table row {} has {} cells, expected {} (MD056 violation):\n  {}",
                    idx + 1,
                    cells,
                    expected,
                    line,
                );
            }
        }
    }
}

#[test]
fn test_count_md_table_cells_treats_escaped_pipes_as_literal() {
    assert_eq!(count_md_table_cells("| a | b | c |"), 3);
    assert_eq!(count_md_table_cells("|---|---|---|"), 3);
    assert_eq!(count_md_table_cells("| `string \\| null` | `null` | desc |"), 3);
    assert_eq!(count_md_table_cells("| `string | null` | `null` | desc |"), 4);
}

#[test]
fn test_generate_docs_typescript_optional_field_emits_consistent_table_cells() {
    use crate::core::ir::{CoreWrapper, FieldDef};
    let api = ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "InitOptions".into(),
            rust_path: "mylib::InitOptions".into(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "cache_dir".into(),
                ty: TypeRef::String,
                optional: true,
                default: None,
                doc: "Override default cache directory.".into(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Init options.".into(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
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
    let files = generate_docs(
        &api,
        &config,
        &[Language::Node, Language::Python, Language::Elixir],
        "docs",
    )
    .unwrap();

    for file in &files {
        assert_no_md056_violations(&file.content);
    }

    let ts_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-typescript"))
        .unwrap();
    assert!(
        ts_file.content.contains("`string \\| null`"),
        "expected pipe inside union type to be escaped, got: {}",
        ts_file.content,
    );
}

#[test]
fn test_generate_docs_post_processing_wraps_bare_urls() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch".to_string(),
            rust_path: "mylib::fetch".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Fetches from https://example.com directly.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
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
    assert!(
        lang_file.content.contains("<https://example.com>"),
        "bare URL must be wrapped by post-processing: {}",
        lang_file.content
    );
}
