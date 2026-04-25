use super::*;
use crate::languages::generate_pre_commit_config;
use alef_core::config::*;
use std::path::PathBuf;

fn test_config() -> AlefConfig {
    AlefConfig {
        alef: Default::default(),
        crate_config: CrateConfig {
            name: "my-lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![Language::Python, Language::Node],
        exclude: ExcludeConfig::default(),
        include: IncludeConfig::default(),
        output: OutputConfig::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        go: None,
        java: None,
        csharp: None,
        r: None,
        scaffold: Some(ScaffoldConfig {
            description: Some("Test library".to_string()),
            license: Some("MIT".to_string()),
            repository: Some("https://github.com/test/my-lib".to_string()),
            homepage: None,
            authors: vec!["Alice".to_string()],
            keywords: vec!["test".to_string()],
        }),
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: CustomModulesConfig::default(),
        custom_registrations: CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
    }
}

fn test_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

/// Filter out project-level scaffold files (like .pre-commit-config.yaml)
/// to isolate language-specific scaffold tests.
fn language_files(files: &[GeneratedFile]) -> Vec<&GeneratedFile> {
    files
        .iter()
        .filter(|f| {
            let p = f.path.to_string_lossy();
            !p.ends_with(".pre-commit-config.yaml")
                && !p.ends_with(".typos.toml")
                && !p.ends_with("rust-toolchain.toml")
                && !p.ends_with(".cargo/config.toml")
        })
        .collect()
}

#[test]
fn test_scaffold_python() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    // scaffold_python: pyproject.toml + py.typed; scaffold_python_cargo: Cargo.toml
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].path, PathBuf::from("packages/python/pyproject.toml"));
    assert!(files[0].content.contains("maturin"));
    assert!(files[0].content.contains("my-lib"));
    assert_eq!(files[1].path, PathBuf::from("packages/python/my_lib/py.typed"));
    assert_eq!(files[2].path, PathBuf::from("crates/my-lib-py/Cargo.toml"));
    assert!(files[2].content.contains("pyo3"));
}

#[test]
fn test_scaffold_node() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    // scaffold_node: pkg package.json + crate package.json + index.d.ts + tsconfig.json + .oxfmtrc.json + .oxlintrc.json; scaffold_node_cargo: Cargo.toml
    assert_eq!(files.len(), 7);
    assert_eq!(files[0].path, PathBuf::from("packages/node/package.json"));
    assert!(files[0].content.contains("napi"));
    assert!(files[0].content.contains("oxfmt"));
    assert_eq!(files[1].path, PathBuf::from("crates/my-lib-node/package.json"));
    assert_eq!(files[2].path, PathBuf::from("packages/node/src/index.d.ts"));
    assert_eq!(files[3].path, PathBuf::from("packages/node/tsconfig.json"));
    assert_eq!(files[4].path, PathBuf::from("packages/node/.oxfmtrc.json"));
    assert_eq!(files[5].path, PathBuf::from("packages/node/.oxlintrc.json"));
    assert_eq!(files[6].path, PathBuf::from("crates/my-lib-node/Cargo.toml"));
    assert!(files[6].content.contains("napi-derive"));
}

#[test]
fn test_scaffold_multiple() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let files = language_files(&all_files);
    // Python: 3 files (pyproject.toml + py.typed + Cargo.toml); Node: 7 files (2 package.json + index.d.ts + tsconfig.json + .oxfmtrc.json + .oxlintrc.json + Cargo.toml)
    assert_eq!(files.len(), 10);
}

#[test]
fn test_scaffold_python_production_features() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let content = &files[0].content;
    assert!(content.contains("[project.urls]"));
    assert!(content.contains("repository ="));
    // Linter config (ruff) is included in the generated pyproject.toml
    assert!(content.contains("[tool.ruff]"));
}

#[test]
fn test_scaffold_node_production_features() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let content = &files[0].content;
    assert!(content.contains("\"scripts\""));
    assert!(content.contains("\"build\""));
    assert!(content.contains("\"files\""));
    assert!(content.contains("\"devDependencies\""));
    assert!(content.contains("@napi-rs/cli"));
    assert!(content.contains("\"triples\""));
}

#[test]
fn test_scaffold_ffi_with_core_import() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 2);
    let cargo_toml = &files[0].content;
    assert!(cargo_toml.contains("serde"));
    assert!(cargo_toml.contains("serde_json"));
    // Should have core_import as dependency
    assert!(cargo_toml.contains("my-lib ="));
    // Should generate cmake config
    let cmake = &files[1].content;
    assert!(cmake.contains("find_package"));
    assert!(cmake.contains("my-lib-ffi::my-lib-ffi"));
}

#[test]
fn test_scaffold_go_production_format() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    // go.mod + .golangci.yml
    assert_eq!(files.len(), 2);
    let content = &files[0].content;
    assert!(content.contains("go 1.26"));
    assert!(!content.contains("require ("));
}

#[test]
fn test_scaffold_java_production_features() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    // pom.xml + checkstyle.xml + checkstyle.properties + checkstyle-suppressions.xml
    // + eclipse-formatter.xml + versions-rules.xml + pmd-ruleset.xml
    assert_eq!(files.len(), 7);
    let content = &files[0].content;
    assert!(content.contains("<properties>"));
    assert!(content.contains("<project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>"));
    assert!(content.contains("<dependencies>"));
    assert!(content.contains("<build>"));
    assert!(content.contains("maven-compiler-plugin"));
    assert!(content.contains("maven-surefire-plugin"));
    assert!(content.contains("--enable-native-access=ALL-UNNAMED"));
    assert!(content.contains("-Djava.library.path=${project.basedir}/../../target/release"));
}

#[test]
fn test_scaffold_ruby_production_features() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    // scaffold_ruby: gemspec, rubocop, Rakefile, lib/*.rb, extconf.rb, Gemfile, Steepfile = 7 files
    // scaffold_ruby_cargo: Cargo.toml = 1 file
    assert_eq!(files.len(), 8);
    let content = &files[0].content;
    assert!(content.contains("spec.required_ruby_version"));
    assert!(content.contains("spec.extensions"));
    assert!(content.contains("spec.metadata['keywords']"));
    assert!(content.contains("frozen_string_literal: true"));
    assert!(content.contains("spec.metadata['rubygems_mfa_required'] = 'true'"));
    // Check for .rubocop.yml generation
    assert_eq!(files[1].path, PathBuf::from("packages/ruby/.rubocop.yml"));
    // Check for Rakefile generation
    assert_eq!(files[2].path, PathBuf::from("packages/ruby/Rakefile"));
    assert!(files[2].content.contains("Rake::ExtensionTask"));
    assert!(files[2].content.contains("my_lib_rb"));
    // Check for lib entry point generation
    assert_eq!(files[3].path, PathBuf::from("packages/ruby/lib/my_lib.rb"));
    assert!(files[3].content.contains("require 'my_lib_rb'"));
    // Check for extconf.rb generation
    assert_eq!(files[4].path, PathBuf::from("packages/ruby/ext/my_lib_rb/extconf.rb"));
    assert!(files[4].content.contains("create_rust_makefile"));
    assert!(files[4].content.contains("rb_sys/mkmf"));
    assert!(
        files[4].content.contains("cargo_manifest"),
        "extconf.rb must set cargo_manifest so rb_sys finds native/Cargo.toml"
    );
    assert!(
        files[4].content.contains("native/Cargo.toml"),
        "extconf.rb cargo_manifest must point to native/Cargo.toml"
    );
    // files[5] is Gemfile; files[6] is Steepfile; files[7] is the Cargo.toml from scaffold_ruby_cargo
    assert_eq!(files[5].path, PathBuf::from("packages/ruby/Gemfile"));
    assert_eq!(files[6].path, PathBuf::from("packages/ruby/Steepfile"));
    // Check for Cargo.toml generation
    assert_eq!(
        files[7].path,
        PathBuf::from("packages/ruby/ext/my_lib_rb/native/Cargo.toml")
    );
    assert!(files[7].content.contains("magnus"));
    assert!(
        files[7].content.contains("path = \"../src/lib.rs\""),
        "Ruby Cargo.toml [lib] must set path to the binding source crate"
    );
}

#[test]
fn test_pre_commit_config_python_node() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Python, Language::Node]);
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    // Common hooks always present
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    assert!(content.contains("trailing-whitespace"));
    assert!(content.contains("cargo-deny"));
    // Python-specific TOML formatting
    assert!(content.contains("pyproject-fmt"));
    // Alef unified hooks replace per-language hooks
    assert!(content.contains("alef-fmt"));
    assert!(content.contains("alef-lint"));
    assert!(content.contains("alef-verify"));
    assert!(content.contains("alef-sync-versions"));
    // No per-language hooks
    assert!(!content.contains("ruff-pre-commit"));
    assert!(!content.contains("oxlint"));
    assert!(!content.contains("php-lint"));
    assert!(!content.contains("golangci-lint"));
    assert!(!content.contains("mix-credo"));
}

#[test]
fn test_pre_commit_config_ffi_only() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Ffi]);
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    // Common + Rust hooks
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    // Alef unified hooks present
    assert!(content.contains("alef-fmt"));
    assert!(content.contains("alef-lint"));
    // No per-language hooks
    assert!(!content.contains("clang-format"));
    assert!(!content.contains("ruff"));
    assert!(!content.contains("biome"));
}

#[test]
fn test_pre_commit_config_clippy_excludes() {
    let config = test_config();
    let files = generate_pre_commit_config(
        &config,
        &[Language::Python, Language::Node, Language::Php, Language::Wasm],
    );
    let content = &files[0].content;
    assert!(content.contains("--exclude=my-lib-py"));
    assert!(content.contains("--exclude=my-lib-node"));
    assert!(content.contains("--exclude=my-lib-php"));
    // Wasm is NOT excluded — rust-toolchain.toml provides the target
    assert!(!content.contains("--exclude=my-lib-wasm"));
    // Ruby not in languages, should not be excluded
    assert!(!content.contains("--exclude=my-lib-rb"));
}

#[test]
fn test_pre_commit_config_all_languages() {
    let config = test_config();
    let files = generate_pre_commit_config(
        &config,
        &[
            Language::Python,
            Language::Node,
            Language::Ruby,
            Language::Php,
            Language::Ffi,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
        ],
    );
    let content = &files[0].content;
    // Common hooks always present
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    assert!(content.contains("trailing-whitespace"));
    assert!(content.contains("typos"));
    // Python-specific TOML formatting
    assert!(content.contains("pyproject-fmt"));
    // Alef unified hooks replace all per-language hooks
    assert!(content.contains("alef-fmt"));
    assert!(content.contains("alef-lint"));
    assert!(content.contains("alef-verify"));
    assert!(content.contains("alef-sync-versions"));
    // Clippy excludes for all binding crates
    assert!(content.contains("--exclude=my-lib-py"));
    assert!(content.contains("--exclude=my-lib-node"));
    assert!(content.contains("--exclude=my-lib-rb"));
    assert!(content.contains("--exclude=my-lib-php"));
    assert!(content.contains("--exclude=my-lib-r"));
    // No per-language hooks
    assert!(!content.contains("ruff"));
    assert!(!content.contains("oxlint"));
    assert!(!content.contains("clang-format"));
    assert!(!content.contains("golangci-lint"));
    assert!(!content.contains("cpd"));
    assert!(!content.contains("dotnet-format"));
    assert!(!content.contains("mix-credo"));
    assert!(!content.contains("rubocop"));
    assert!(!content.contains("php-lint"));
    assert!(!content.contains("r-lintr"));
}

// --- Oxc toolchain tests ---

#[test]
fn test_node_scaffold_no_biome_references() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    for f in &files {
        assert!(
            !f.content.contains("biome"),
            "File {} should not reference biome: found in content",
            f.path.display()
        );
        assert!(
            !f.path.to_string_lossy().contains("biome"),
            "File path should not contain biome: {}",
            f.path.display()
        );
    }
}

#[test]
fn test_node_scaffold_oxfmt_config_content() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let oxfmtrc = files.iter().find(|f| f.path.ends_with(".oxfmtrc.json")).unwrap();
    assert!(oxfmtrc.content.contains("\"printWidth\": 120"));
    assert!(oxfmtrc.content.contains("\"useTabs\": true"));
    assert!(oxfmtrc.content.contains("\"tabWidth\": 4"));
    assert!(oxfmtrc.content.contains("\"singleQuote\": false"));
    assert!(oxfmtrc.content.contains("\"trailingComma\": \"all\""));
    assert!(oxfmtrc.content.contains("\"sortImports\": true"));
}

#[test]
fn test_node_scaffold_oxlint_config_content() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let oxlintrc = files.iter().find(|f| f.path.ends_with(".oxlintrc.json")).unwrap();
    assert!(oxlintrc.content.contains("\"correctness\": \"error\""));
    assert!(oxlintrc.content.contains("\"suspicious\": \"warn\""));
    assert!(oxlintrc.content.contains("\"style\": \"off\""));
    assert!(oxlintrc.content.contains("\"typescript\""));
    assert!(oxlintrc.content.contains("overrides"));
    assert!(oxlintrc.content.contains("**/*.test.ts"));
}

#[test]
fn test_node_package_json_uses_oxc() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let pkg = &files[0];
    assert!(pkg.content.contains("\"oxfmt\""));
    assert!(pkg.content.contains("\"oxlint\""));
    assert!(pkg.content.contains("\"format\": \"oxfmt\""));
    assert!(pkg.content.contains("\"lint\": \"oxlint\""));
    assert!(pkg.content.contains("\"lint:fix\": \"oxlint --fix\""));
    assert!(!pkg.content.contains("biome"));
}

#[test]
fn test_precommit_no_biome_with_node() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Node]);
    let content = &files[0].content;
    assert!(!content.contains("biome-format"));
    assert!(!content.contains("biome-lint"));
    assert!(!content.contains("biomejs"));
    // alef-fmt/alef-lint replace oxlint/oxfmt
    assert!(content.contains("alef-fmt"));
    assert!(content.contains("alef-lint"));
    assert!(!content.contains("oxlint"));
}

// --- Java checkstyle tests ---

#[test]
fn test_java_checkstyle_no_cosmetic_checks() {
    let mut config = test_config();
    config.languages = vec![Language::Java];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let checkstyle = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    // Should NOT have cosmetic whitespace checks (Spotless handles formatting)
    assert!(!checkstyle.content.contains("WhitespaceAfter"));
    assert!(!checkstyle.content.contains("WhitespaceAround"));
    assert!(!checkstyle.content.contains("GenericWhitespace"));
    assert!(!checkstyle.content.contains("EmptyBlock"));
    assert!(!checkstyle.content.contains("NeedBraces"));
    assert!(!checkstyle.content.contains("MagicNumber"));
    assert!(!checkstyle.content.contains("JavadocPackage"));
    // Should still have correctness checks
    assert!(checkstyle.content.contains("EqualsHashCode"));
    assert!(checkstyle.content.contains("UnusedImports"));
    assert!(checkstyle.content.contains("MethodLength"));
    assert!(checkstyle.content.contains("LineLength"));
    assert!(checkstyle.content.contains("\"120\""));
}

// --- Go golangci v2 format tests ---

#[test]
fn test_go_golangci_v2_format() {
    let mut config = test_config();
    config.languages = vec![Language::Go];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    let golangci = files.iter().find(|f| f.path.ends_with(".golangci.yml")).unwrap();
    assert!(golangci.content.contains("version: \"2\""));
    assert!(golangci.content.contains("default: none"));
    assert!(golangci.content.contains("settings:"));
    // Should NOT use old v1 format
    assert!(!golangci.content.contains("linters-settings:"));
    // Should have detailed config
    assert!(golangci.content.contains("errcheck"));
    assert!(golangci.content.contains("govet"));
    assert!(golangci.content.contains("misspell"));
    assert!(golangci.content.contains("locale: US"));
    assert!(golangci.content.contains("exclusions:"));
}

fn config_with_extra_deps() -> AlefConfig {
    let mut config = test_config();
    config
        .crate_config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.crate_config.extra_dependencies.insert(
        "tracing".to_string(),
        toml::Value::Table({
            let mut t = toml::map::Map::new();
            t.insert("version".to_string(), toml::Value::String("0.1".to_string()));
            t.insert(
                "features".to_string(),
                toml::Value::Array(vec![toml::Value::String("log".to_string())]),
            );
            t
        }),
    );
    config
}

#[test]
fn test_render_extra_deps_empty() {
    let config = test_config();
    assert_eq!(render_extra_deps(&config, Language::Python), "");
}

#[test]
fn test_render_extra_deps_string_version() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("anyhow = \"1.0\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_table_value() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("tracing = "), "got: {rendered}");
    assert!(rendered.contains("\"log\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_sorted() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    let anyhow_pos = rendered.find("anyhow").expect("anyhow missing");
    let tracing_pos = rendered.find("tracing").expect("tracing missing");
    assert!(anyhow_pos < tracing_pos, "deps should be sorted alphabetically");
}

#[test]
fn test_scaffold_python_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("tracing"),
        "content: {}",
        cargo_toml.content
    );
    // Extra deps should appear in [dependencies] section, before [features]
    let deps_pos = cargo_toml.content.find("[dependencies]").unwrap();
    let features_pos = cargo_toml.content.find("[features]").unwrap();
    let anyhow_pos = cargo_toml.content.find("anyhow").unwrap();
    assert!(anyhow_pos > deps_pos && anyhow_pos < features_pos);
}

#[test]
fn test_scaffold_node_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_php_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_lib_path() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml
            .content
            .contains("path = \"../../../../crates/my-lib-elixir/src/lib.rs\""),
        "Elixir Cargo.toml [lib] must set path to the binding source crate; content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("name = \"my_lib_nif\""),
        "Elixir Cargo.toml [lib] must set name; content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_language_level_extra_deps_override_crate_level() {
    let mut config = test_config();
    // Crate-level dep with version "1.0"
    config
        .crate_config
        .extra_dependencies
        .insert("shared-dep".to_string(), toml::Value::String("1.0".to_string()));
    // Python-level override with a different version; inject via extra_deps_for_language
    // by inserting directly into a Python extra_dependencies map.
    let mut python_extra: std::collections::HashMap<String, toml::Value> = std::collections::HashMap::new();
    python_extra.insert("shared-dep".to_string(), toml::Value::String("2.0".to_string()));
    config.python = Some(PythonConfig {
        module_name: None,
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: std::collections::HashMap::new(),
        release_gil: false,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: python_extra,
        scaffold_output: None,
        rename_fields: Default::default(),
    });
    let rendered = render_extra_deps(&config, Language::Python);
    // Python-level "2.0" should win over crate-level "1.0"
    assert!(rendered.contains("shared-dep = \"2.0\""), "got: {rendered}");
    assert!(
        !rendered.contains("1.0"),
        "crate-level version should be overridden, got: {rendered}"
    );
}
