use super::*;

#[test]
fn docs_snippets_merge_covers_validation_policy_and_paths() {
    let workspace: DocsSnippetsConfig = toml::from_str(
        r#"
dirs = ["docs/snippets"]
content_collections = { apiExamples = "docs/snippets/generated" }
inline_dirs = ["docs/guides"]
exclude = ["docs/reference"]
strict = true
deny_unclassified = true
allowed_side_effects = ["safe", "filesystem"]
cache_dir = ".cache/snippets"
report_output = "artifacts/snippets.json"
"#,
    )
    .unwrap();
    let krate: DocsSnippetsConfig = toml::from_str(
        r#"
inline_dirs = ["book"]
allowed_side_effects = ["safe"]
strict = true
deny_unclassified = true
"#,
    )
    .unwrap();

    let merged = DocsSnippetsConfig::merge(Some(&workspace), Some(&krate)).unwrap();
    assert_eq!(merged.dirs, vec![PathBuf::from("docs/snippets")]);
    assert_eq!(
        merged.content_collections,
        BTreeMap::from([("apiExamples".to_string(), PathBuf::from("docs/snippets/generated"))])
    );
    assert_eq!(merged.inline_dirs, vec![PathBuf::from("book")]);
    assert_eq!(merged.exclude, vec![PathBuf::from("docs/reference")]);
    assert!(merged.strict);
    assert!(merged.deny_unclassified);
    assert_eq!(merged.allowed_side_effects, vec!["safe"]);
    assert_eq!(merged.cache_dir(), PathBuf::from(".cache/snippets"));
    assert_eq!(merged.report_output, Some(PathBuf::from("artifacts/snippets.json")));
}

#[test]
fn docs_snippets_cache_dir_has_stable_default() {
    assert_eq!(
        DocsSnippetsConfig::default().cache_dir(),
        PathBuf::from(".alef/snippets")
    );
}

#[test]
fn string_or_vec_single_from_toml() {
    let toml_str = r#"format = "ruff format""#;
    #[derive(Deserialize)]
    struct T {
        format: StringOrVec,
    }
    let t: T = toml::from_str(toml_str).unwrap();
    assert_eq!(t.format.commands(), vec!["ruff format"]);
}

#[test]
fn string_or_vec_multiple_from_toml() {
    let toml_str = r#"format = ["cmd1", "cmd2", "cmd3"]"#;
    #[derive(Deserialize)]
    struct T {
        format: StringOrVec,
    }
    let t: T = toml::from_str(toml_str).unwrap();
    assert_eq!(t.format.commands(), vec!["cmd1", "cmd2", "cmd3"]);
}

#[test]
fn string_or_vec_empty_array_from_toml() {
    let toml_str = "format = []";
    #[derive(Deserialize)]
    struct T {
        format: StringOrVec,
    }
    let t: T = toml::from_str(toml_str).unwrap();
    assert!(matches!(t.format, StringOrVec::Multiple(_)));
    assert!(t.format.commands().is_empty());
}

#[test]
fn string_or_vec_single_element_array_from_toml() {
    let toml_str = r#"format = ["cmd"]"#;
    #[derive(Deserialize)]
    struct T {
        format: StringOrVec,
    }
    let t: T = toml::from_str(toml_str).unwrap();
    assert_eq!(t.format.commands(), vec!["cmd"]);
}

#[test]
fn test_config_backward_compat_string() {
    let toml_str = r#"command = "pytest""#;
    let cfg: TestConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
    assert!(cfg.e2e.is_none());
    assert!(cfg.coverage.is_none());
}

#[test]
fn test_config_array_command() {
    let toml_str = r#"command = ["cmd1", "cmd2"]"#;
    let cfg: TestConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.command.unwrap().commands(), vec!["cmd1", "cmd2"]);
}

#[test]
fn test_config_with_coverage() {
    let toml_str = r#"
command = "pytest"
coverage = "pytest --cov=. --cov-report=term-missing"
"#;
    let cfg: TestConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
    assert_eq!(
        cfg.coverage.unwrap().commands(),
        vec!["pytest --cov=. --cov-report=term-missing"]
    );
    assert!(cfg.e2e.is_none());
}

#[test]
fn test_config_all_optional() {
    let toml_str = "";
    let cfg: TestConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.command.is_none());
    assert!(cfg.e2e.is_none());
    assert!(cfg.coverage.is_none());
}

#[test]
fn test_config_with_before_list() {
    let toml_str = r#"
before = ["cd packages/python && maturin develop", "echo ready"]
command = "pytest"
"#;
    let cfg: TestConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.precondition.is_none());
    assert_eq!(
        cfg.before.unwrap().commands(),
        vec!["cd packages/python && maturin develop", "echo ready"]
    );
    assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
}

#[test]
fn output_template_resolves_explicit_entry() {
    let tmpl = OutputTemplate {
        python: Some("crates/{crate}-py/src/".to_string()),
        ..Default::default()
    };
    assert_eq!(
        tmpl.resolve("sample_router", "python", true),
        PathBuf::from("crates/sample_router-py/src/")
    );
}

#[test]
fn output_template_substitutes_lang_and_crate() {
    let tmpl = OutputTemplate {
        go: Some("packages/{lang}/{crate}/".to_string()),
        ..Default::default()
    };
    assert_eq!(
        tmpl.resolve("sample_router-runtime", "go", true),
        PathBuf::from("packages/go/sample_router-runtime/")
    );
}

#[test]
fn output_template_falls_back_to_multi_crate_default() {
    let tmpl = OutputTemplate::default();
    assert_eq!(
        tmpl.resolve("sample_router-runtime", "python", true),
        PathBuf::from("packages/python/sample_router-runtime")
    );
}

/// Languages with a dedicated binding crate resolve to `crates/{crate}-<suffix>/src` on a
/// single-crate workspace with no configured output -- the same root the scaffolder
/// writes a manifest for and, for wasm, the same root `WasmBackend::generate_bindings`
/// writes its own `Cargo.toml` into. Languages with no such crate keep the historical
/// `packages/{lang}` default.
#[test]
fn output_template_falls_back_to_single_crate_binding_crate_default() {
    let tmpl = OutputTemplate::default();
    assert_eq!(
        tmpl.resolve("sample_router", "python", false),
        PathBuf::from("crates/sample_router-py/src")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "node", false),
        PathBuf::from("crates/sample_router-node/src")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "php", false),
        PathBuf::from("crates/sample_router-php/src")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "ffi", false),
        PathBuf::from("crates/sample_router-ffi/src")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "wasm", false),
        PathBuf::from("crates/sample_router-wasm/src")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "ruby", false),
        PathBuf::from("packages/ruby")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "elixir", false),
        PathBuf::from("packages/elixir")
    );
}

#[test]
fn output_template_falls_back_to_lang_dir_for_unknown_languages() {
    let tmpl = OutputTemplate::default();
    assert_eq!(tmpl.resolve("sample_router", "go", false), PathBuf::from("packages/go"));
    assert_eq!(
        tmpl.resolve("sample_router", "swift", false),
        PathBuf::from("packages/swift")
    );
}

#[test]
fn output_template_deserializes_from_toml() {
    let toml_str = r#"
python = "packages/python/{crate}/"
go     = "packages/go/{crate}/"
"#;
    let tmpl: OutputTemplate = toml::from_str(toml_str).unwrap();
    assert_eq!(tmpl.python.as_deref(), Some("packages/python/{crate}/"));
    assert_eq!(tmpl.go.as_deref(), Some("packages/go/{crate}/"));
    assert!(tmpl.node.is_none());
}

#[test]
#[should_panic(expected = "path separators are not allowed")]
fn resolve_rejects_crate_name_with_path_separator() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("../foo", "python", false);
}

#[test]
#[should_panic(expected = "path separators are not allowed")]
fn resolve_rejects_crate_name_with_backslash() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("..\\foo", "python", false);
}

#[test]
#[should_panic(expected = "NUL byte is not allowed")]
fn resolve_rejects_crate_name_with_nul_byte() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("foo\0bar", "python", false);
}

#[test]
#[should_panic(expected = "would escape the project root")]
fn resolve_rejects_template_that_produces_parent_dir() {
    let tmpl = OutputTemplate {
        python: Some("../../etc/{crate}".to_string()),
        ..Default::default()
    };
    tmpl.resolve("mylib", "python", false);
}

#[test]
fn resolve_accepts_normal_crate_name() {
    let tmpl = OutputTemplate::default();
    let path = tmpl.resolve("my-lib", "python", false);
    assert_eq!(path, PathBuf::from("crates/my-lib-py/src"));
}
