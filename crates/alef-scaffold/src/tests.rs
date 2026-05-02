use super::*;
use crate::languages::generate_pre_commit_config;
use alef_core::config::{
    Language, NewAlefConfig, PythonConfig, ResolvedCrateConfig, ScaffoldCargoTargets, ScaffoldConfig,
};
use std::path::{Path, PathBuf};

fn test_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    )
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
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
    // scaffold_node: pkg package.json + crate package.json + src/index.d.ts + index.d.ts + tsconfig.json + .oxfmtrc.json + .oxlintrc.json; scaffold_node_cargo: Cargo.toml
    assert_eq!(files.len(), 8);
    assert_eq!(files[0].path, PathBuf::from("packages/node/package.json"));
    assert!(files[0].content.contains("napi"));
    assert!(files[0].content.contains("oxfmt"));
    assert_eq!(files[1].path, PathBuf::from("crates/my-lib-node/package.json"));
    assert_eq!(files[2].path, PathBuf::from("packages/node/src/index.d.ts"));
    assert_eq!(files[3].path, PathBuf::from("packages/node/index.d.ts"));
    assert_eq!(files[4].path, PathBuf::from("packages/node/tsconfig.json"));
    assert_eq!(files[5].path, PathBuf::from("packages/node/.oxfmtrc.json"));
    assert_eq!(files[6].path, PathBuf::from("packages/node/.oxlintrc.json"));
    assert_eq!(files[7].path, PathBuf::from("crates/my-lib-node/Cargo.toml"));
    assert!(files[7].content.contains("napi-derive"));
}

#[test]
fn test_scaffold_multiple() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let files = language_files(&all_files);
    // Python: 3 files (pyproject.toml + py.typed + Cargo.toml); Node: 8 files (2 package.json + src/index.d.ts + index.d.ts + tsconfig.json + .oxfmtrc.json + .oxlintrc.json + Cargo.toml)
    assert_eq!(files.len(), 11);
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
fn test_scaffold_ffi_deps_are_pinned() {
    // Audit: FFI Cargo.toml ships sensible, current dependency pins.
    // Bumping cbindgen requires re-generating headers; treat this test as a
    // canary — if it fails, audit cbindgen's changelog before adjusting.
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;
    assert!(
        cargo_toml.contains("cbindgen = \"0.29\""),
        "cbindgen should be pinned to a specific minor for reproducible headers"
    );
    assert!(cargo_toml.contains("serde_json = \"1\""));
    assert!(cargo_toml.contains("tokio = "));
    assert!(cargo_toml.contains("[dev-dependencies]"));
    assert!(cargo_toml.contains("tempfile = \"3\""));
}

#[test]
fn test_scaffold_ffi_merges_extra_dependencies() {
    // Multi-crate workspaces (e.g. mylib's mylib-core/-http/-extra) emit FFI
    // bindings that reference qualified crate paths. The scaffold must merge
    // [crate.extra_dependencies] from alef.toml so the generated cdylib can
    // resolve those imports.
    let mut config = test_config();
    let mut deps: std::collections::HashMap<String, toml::Value> = Default::default();
    deps.insert(
        "my-lib-http".to_string(),
        toml::Value::try_from(toml::Table::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )]))
        .unwrap(),
    );
    deps.insert(
        "my-lib-graphql".to_string(),
        toml::Value::try_from(toml::Table::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-graphql".to_string()),
        )]))
        .unwrap(),
    );
    deps.insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies = deps;

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;
    assert!(
        cargo_toml.contains("my-lib-http = { path = \"../my-lib-http\" }"),
        "scaffold should emit my-lib-http path dep, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib-graphql = { path = \"../my-lib-graphql\" }"),
        "scaffold should emit my-lib-graphql path dep, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("anyhow = \"1.0\""),
        "scaffold should emit anyhow string dep, got:\n{cargo_toml}"
    );
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
        files[4].content.contains("config.ext_dir = 'native'"),
        "extconf.rb must set ext_dir = 'native' so rb_sys finds native/Cargo.toml"
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
    assert!(content.contains("alef-readme"));
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
    assert!(content.contains("alef-verify"));
    assert!(content.contains("alef-readme"));
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
    assert!(content.contains("alef-readme"));
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
    // alef-readme/alef-verify replace oxlint/oxfmt at the alef hook level
    assert!(content.contains("alef-readme"));
    assert!(content.contains("alef-verify"));
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

#[test]
fn test_scaffold_csharp_csproj_at_package_root() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    // csproj at package root + .editorconfig + Directory.Build.props
    let csproj = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".csproj"))
        .expect("C# scaffold must produce a .csproj file");
    // Must be at packages/csharp/<Namespace>.csproj (package root), NOT inside the source subdir
    assert_eq!(
        csproj.path,
        PathBuf::from("packages/csharp/MyLib/MyLib.csproj"),
        "csproj must be in the namespace subdirectory so runtimes/** glob aligns with FFI staging"
    );
    assert!(
        csproj.content.contains("Microsoft.NET.Sdk"),
        "csproj must use Microsoft.NET.Sdk"
    );
    assert!(
        csproj.content.contains("net10.0"),
        "csproj must target net10.0 by default"
    );
    assert!(
        csproj.content.contains("<RootNamespace>MyLib</RootNamespace>"),
        "csproj must set RootNamespace to the PascalCase project name"
    );
    assert!(
        csproj.content.contains("<Nullable>enable</Nullable>"),
        "csproj must enable nullable reference types"
    );
    assert!(
        !csproj.generated_header,
        "csproj must be scaffold-once (generated_header = false)"
    );
}

#[test]
fn test_render_csharp_csproj_runtimes_glob_is_relative() {
    // Regression: the runtimes glob must NOT have a "../" prefix.
    // The csproj lives at packages/csharp/<Namespace>/<Namespace>.csproj, so
    // `runtimes/**` resolves to packages/csharp/<Namespace>/runtimes/ — the exact
    // directory where alef-publish stages the FFI shared libraries.
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.2.3");
    assert!(
        content.contains(r#"Include="runtimes/**""#),
        "runtimes glob must be relative (no ../ prefix): {content}"
    );
    assert!(
        !content.contains(r#"Include="../runtimes"#),
        "runtimes glob must NOT have ../: {content}"
    );
    assert!(
        content.contains(r#"Include="../../LICENSE""#),
        "LICENSE path must be ../../LICENSE to reach workspace root: {content}"
    );
    assert!(
        content.contains("<Version>1.2.3</Version>"),
        "version must be substituted: {content}"
    );
}

fn config_with_extra_deps() -> ResolvedCrateConfig {
    let mut config = test_config();
    config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies.insert(
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
fn test_scaffold_elixir_cargo_lib_name_no_path() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    // [lib] must NOT have a path pointing to a non-existent -elixir crate.
    // Cargo defaults to src/lib.rs, which is where the generated lib.rs lives.
    assert!(
        !cargo_toml.content.contains("-elixir/src/lib.rs"),
        "Elixir Cargo.toml [lib] must NOT point to a non-existent -elixir crate; content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("name = \"my_lib_nif\""),
        "Elixir Cargo.toml [lib] must set name to {{app_name}}_nif; content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_language_level_extra_deps_override_crate_level() {
    let mut config = test_config();
    // Crate-level dep with version "1.0"
    config
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
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
    });
    let rendered = render_extra_deps(&config, Language::Python);
    // Python-level "2.0" should win over crate-level "1.0"
    assert!(rendered.contains("shared-dep = \"2.0\""), "got: {rendered}");
    assert!(
        !rendered.contains("1.0"),
        "crate-level version should be overridden, got: {rendered}"
    );
}

#[test]
fn test_scaffold_elixir_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let api = test_api(); // all sync — no async functions or methods
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only API must not include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("async-trait"),
        "sync-only API without trait bridges must not include async-trait; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Ruby];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only Ruby API must not include tokio; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_java_checkstyle_suppressions_use_config_location() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let xml = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    assert!(
        xml.content
            .contains(r#"value="packages/java/checkstyle-suppressions.xml""#),
        "checkstyle suppressions path must work from repo root and Maven; content:\n{}",
        xml.content
    );
    let properties = files
        .iter()
        .find(|f| f.path.ends_with("checkstyle.properties"))
        .unwrap();
    assert!(
        properties.content.is_empty(),
        "checkstyle properties should stay empty; content:\n{}",
        properties.content
    );
}

#[test]
fn test_scaffold_php_cs_fixer_handles_missing_tests_dir() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let fixer = files.iter().find(|f| f.path.ends_with("php-cs-fixer.php")).unwrap();
    assert!(
        fixer.content.contains("declare(strict_types=1);"),
        "php-cs-fixer config should be fixer-clean; content:\n{}",
        fixer.content
    );
    assert!(
        fixer.content.contains("is_dir(__DIR__ . '/tests')"),
        "php-cs-fixer config must not require a tests directory; content:\n{}",
        fixer.content
    );
}

#[test]
fn test_scaffold_dart() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let files = language_files(&all_files);
    // pubspec.yaml + analysis_options.yaml + .gitignore + test + BUILDING.md + .editorconfig + README.md + example + dart.yml
    assert_eq!(files.len(), 9, "Expected 9 files for Dart scaffold");

    let pubspec = &files[0];
    assert_eq!(pubspec.path, PathBuf::from("packages/dart/pubspec.yaml"));
    assert!(pubspec.content.contains("name: my_lib"), "got: {}", pubspec.content);
    assert!(pubspec.content.contains("version: 0.1.0"), "got: {}", pubspec.content);
    assert!(
        pubspec.content.contains("flutter_rust_bridge:"),
        "got: {}",
        pubspec.content
    );
    assert!(pubspec.content.contains("sdk: '"), "got: {}", pubspec.content);
    assert!(pubspec.content.contains("test:"), "got: {}", pubspec.content);
    assert!(pubspec.content.contains("lints:"), "got: {}", pubspec.content);

    let analysis_options = &files[1];
    assert_eq!(
        analysis_options.path,
        PathBuf::from("packages/dart/analysis_options.yaml")
    );
    assert!(
        analysis_options.content.contains("package:lints/recommended.yaml"),
        "got: {}",
        analysis_options.content
    );
    assert!(
        analysis_options.content.contains("linter:"),
        "analysis_options.yaml should include linter rules; got: {}",
        analysis_options.content
    );

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/dart/.gitignore"));
    assert!(gitignore.content.contains(".dart_tool/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains("build/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains("pubspec.lock"), "got: {}", gitignore.content);

    let test_file = &files[3];
    assert_eq!(test_file.path, PathBuf::from("packages/dart/test/my_lib_test.dart"));
    assert!(
        test_file.content.contains("import 'package:test/test.dart'"),
        "got: {}",
        test_file.content
    );
    assert!(
        test_file.content.contains("test('placeholder'"),
        "got: {}",
        test_file.content
    );
    assert!(
        test_file.content.contains("expect(1 + 1, equals(2))"),
        "got: {}",
        test_file.content
    );

    let building_md = &files[4];
    assert_eq!(building_md.path, PathBuf::from("packages/dart/BUILDING.md"));
    assert!(
        building_md
            .content
            .contains("cargo install flutter_rust_bridge_codegen"),
        "got: {}",
        building_md.content
    );
    assert!(
        building_md.content.contains("flutter_rust_bridge_codegen generate"),
        "got: {}",
        building_md.content
    );
    assert!(
        building_md.content.contains("dart test"),
        "got: {}",
        building_md.content
    );

    // Check for new production files
    assert_eq!(files[5].path, PathBuf::from("packages/dart/.editorconfig"));
    assert!(files[5].content.contains("*.dart"));

    assert_eq!(files[6].path, PathBuf::from("packages/dart/README.md"));
    assert!(files[6].content.contains("dart pub get"));

    assert_eq!(
        files[7].path,
        PathBuf::from("packages/dart/example/my_lib_example.dart")
    );
    assert!(files[7].content.contains("void main"));

    assert_eq!(files[8].path, PathBuf::from(".github/workflows/dart.yml"));
    assert!(files[8].content.contains("dart-lang/setup-dart"));
}

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_function() {
    use alef_core::ir::{FunctionDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.functions.push(FunctionDef {
        name: "do_work".to_string(),
        rust_path: "my_lib::do_work".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async function API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

/// Trait bridge module names must use PascalCase for hyphenated crate names.
///
/// When the consumer crate name contains hyphens (e.g., `html-to-markdown`), the
/// Elixir trait bridge module name must be `HtmlToMarkdownHtmlVisitorBridge`, not
/// `Html_to_markdownHtmlVisitorBridge` (which is what `capitalize_first` produces).
#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_hyphenated_crate() {
    use alef_core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "html-to-markdown".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(alef_core::config::ElixirConfig {
        app_name: Some("html_to_markdown".to_string()),
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("html_visitor_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file
            .content
            .contains("defmodule HtmlToMarkdownHtmlVisitorBridge do"),
        "trait bridge module name must be PascalCase for hyphenated crate names; got:\n{}",
        bridge_file.content
    );
    assert!(
        !bridge_file.content.contains("Html_to_markdown"),
        "trait bridge module name must not contain capitalize_first artifact 'Html_to_markdown'; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_multi_word_crate() {
    use alef_core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "tree-sitter-language-pack".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(alef_core::config::ElixirConfig {
        app_name: Some("tree_sitter_language_pack".to_string()),
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Parser".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("parser_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file
            .content
            .contains("defmodule TreeSitterLanguagePackParserBridge do"),
        "trait bridge module name must be full PascalCase; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_method() {
    use alef_core::ir::{MethodDef, TypeDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.types.push(TypeDef {
        name: "Worker".to_string(),
        rust_path: "my_lib::Worker".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "run".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async method API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_swift() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);
    // Original 7 + new: .editorconfig + .swiftformat + README.md + Examples/Demo/main.swift + swift.yml = 12
    assert_eq!(
        files.len(),
        12,
        "Expected 12 files for Swift scaffold (original 7 + 5 new)"
    );

    let package_swift = &files[0];
    assert_eq!(package_swift.path, PathBuf::from("packages/swift/Package.swift"));
    // Module name derives to PascalCase of "my-lib" → "MyLib"
    assert!(
        package_swift.content.contains("name: \"MyLib\""),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".macOS(.v13)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".iOS(.v16)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("swift-tools-version: 6.0"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Sources/MyLib"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Tests/MyLibTests"),
        "got: {}",
        package_swift.content
    );
    // Must declare RustBridge and RustBridgeC targets
    assert!(
        package_swift.content.contains("\"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("\"RustBridgeC\""),
        "Package.swift must declare RustBridgeC target; got: {}",
        package_swift.content
    );
    // RustBridge target must exist (unsafeFlags removed to allow use as a dependency)
    assert!(
        package_swift.content.contains("name: \"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );

    let gitignore = &files[1];
    assert_eq!(gitignore.path, PathBuf::from("packages/swift/.gitignore"));
    assert!(gitignore.content.contains(".build/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains(".swiftpm/"), "got: {}", gitignore.content);

    // RustBridgeC placeholder header (pure C target)
    let header = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridgeC/RustBridgeC.h"))
        .unwrap();
    assert!(
        header.content.contains("#ifndef RUST_BRIDGE_C_H"),
        "got: {}",
        header.content
    );

    // module.modulemap in RustBridge (kept as documentation comment)
    let modulemap = files.iter().find(|f| f.path.ends_with("module.modulemap")).unwrap();
    assert!(!modulemap.content.is_empty(), "module.modulemap must not be empty");

    // RustBridge placeholder Swift source
    let rust_bridge_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridge/RustBridge.swift"))
        .unwrap();
    assert!(
        !rust_bridge_swift.content.is_empty(),
        "RustBridge.swift must not be empty"
    );

    // BUILDING.md documents the cargo-then-copy workflow
    let building = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/BUILDING.md"))
        .unwrap();
    assert!(
        building.content.contains("cargo build"),
        "BUILDING.md must mention cargo build; got: {}",
        building.content
    );
    assert!(
        building.content.contains("Sources/RustBridgeC"),
        "BUILDING.md must mention RustBridgeC copy destination; got: {}",
        building.content
    );
    // Check for new production files
    let readme = files.iter().find(|f| f.path == Path::new("packages/swift/README.md"));
    assert!(readme.is_some(), "README.md should be generated");
    assert!(
        readme.unwrap().content.contains("swift build"),
        "README.md must document build process"
    );
    let editorconfig = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.editorconfig"));
    assert!(editorconfig.is_some(), ".editorconfig should be generated");
    let swiftformat = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.swiftformat"));
    assert!(swiftformat.is_some(), ".swiftformat should be generated");
    let demo = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Examples/Demo/main.swift"));
    assert!(demo.is_some(), "Demo example should be generated");
    let workflow = files
        .iter()
        .find(|f| f.path == Path::new(".github/workflows/swift.yml"));
    assert!(workflow.is_some(), "GitHub workflow should be generated");
}

#[test]
fn test_scaffold_kotlin() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    // build.gradle.kts, settings.gradle.kts, .gitignore, .editorconfig, gradle.properties, README.md, Sample.kt, kotlin.yml
    assert_eq!(files.len(), 8, "Expected 8 files for Kotlin scaffold");
    assert_eq!(files[0].path, PathBuf::from("packages/kotlin/build.gradle.kts"));
    assert!(files[0].content.contains("kotlin(\"jvm\")"));
    assert!(files[0].content.contains("org.jlleitschuh.gradle.ktlint"));
    assert_eq!(files[1].path, PathBuf::from("packages/kotlin/settings.gradle.kts"));
    assert_eq!(files[2].path, PathBuf::from("packages/kotlin/.gitignore"));
    assert_eq!(files[3].path, PathBuf::from("packages/kotlin/.editorconfig"));
    assert!(files[3].content.contains("*.kt"));
    assert_eq!(files[4].path, PathBuf::from("packages/kotlin/gradle.properties"));
    assert!(files[4].content.contains("org.gradle.parallel=true"));
    assert_eq!(files[5].path, PathBuf::from("packages/kotlin/README.md"));
    assert!(files[5].content.contains("my_lib"));
    assert!(files[5].content.contains("gradle build"));
    assert_eq!(
        files[6].path,
        PathBuf::from("packages/kotlin/src/main/kotlin/sample/Sample.kt")
    );
    assert!(files[6].content.contains("object"));
    assert_eq!(files[7].path, PathBuf::from(".github/workflows/kotlin.yml"));
    assert!(files[7].content.contains("gradle build"));
}

#[test]
fn test_scaffold_gleam() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Gleam]).unwrap();
    let files = language_files(&all_files);
    // gleam.toml + manifest.toml + .gitignore + test + .editorconfig + README.md + example + gleam.yml
    assert_eq!(files.len(), 8, "Expected 8 files for Gleam scaffold");

    let gleam_toml = &files[0];
    assert_eq!(gleam_toml.path, PathBuf::from("packages/gleam/gleam.toml"));
    assert!(
        gleam_toml.content.contains("description"),
        "gleam.toml should include description"
    );
    assert!(
        gleam_toml.content.contains("licences = [\"MIT\"]"),
        "gleam.toml should include licences"
    );

    let manifest = &files[1];
    assert_eq!(manifest.path, PathBuf::from("packages/gleam/manifest.toml"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/gleam/.gitignore"));
    assert!(gitignore.content.contains("build/"));

    assert!(files[3].path.to_string_lossy().ends_with("_test.gleam"));

    let editorconfig = &files[4];
    assert_eq!(editorconfig.path, PathBuf::from("packages/gleam/.editorconfig"));
    assert!(editorconfig.content.contains("*.gleam"));

    let readme = &files[5];
    assert_eq!(readme.path, PathBuf::from("packages/gleam/README.md"));
    assert!(readme.content.contains("gleam build"));

    assert!(files[6].path.to_string_lossy().ends_with("_example.gleam"));
    assert!(files[6].content.contains("Nil"));

    let workflow = &files[7];
    assert_eq!(workflow.path, PathBuf::from(".github/workflows/gleam.yml"));
    assert!(workflow.content.contains("erlef/setup-beam"));
}

#[test]
fn test_scaffold_zig() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    // build.zig + build.zig.zon + .gitignore + .editorconfig + README.md + example.zig + main.zig + zig.yml
    assert_eq!(files.len(), 8, "Expected 8 files for Zig scaffold");

    let build_zig = &files[0];
    assert_eq!(build_zig.path, PathBuf::from("packages/zig/build.zig"));
    assert!(build_zig.content.contains("addModule"));

    let build_zig_zon = &files[1];
    assert_eq!(build_zig_zon.path, PathBuf::from("packages/zig/build.zig.zon"));
    assert!(build_zig_zon.content.contains(".fingerprint"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/zig/.gitignore"));
    assert!(gitignore.content.contains("zig-cache/"));

    let editorconfig = &files[3];
    assert_eq!(editorconfig.path, PathBuf::from("packages/zig/.editorconfig"));
    assert!(editorconfig.content.contains("*.zig"));

    let readme = &files[4];
    assert_eq!(readme.path, PathBuf::from("packages/zig/README.md"));
    assert!(readme.content.contains("zig build"));

    let example = &files[5];
    assert_eq!(example.path, PathBuf::from("packages/zig/examples/example.zig"));
    assert!(example.content.contains("pub fn main"));

    let main = &files[6];
    assert_eq!(main.path, PathBuf::from("packages/zig/src/main.zig"));
    assert!(main.content.contains("test"));
    assert!(main.content.contains("pub fn add"));

    let workflow = &files[7];
    assert_eq!(workflow.path, PathBuf::from(".github/workflows/zig.yml"));
    assert!(workflow.content.contains("mlugg/setup-zig"));
}

// ---------------------------------------------------------------------------
// `[scaffold.cargo]` workspace `.cargo/config.toml` rendering tests.
// ---------------------------------------------------------------------------

fn cargo_only_config(cargo: ScaffoldCargo) -> ResolvedCrateConfig {
    let mut cfg = test_config();
    cfg.scaffold = Some(ScaffoldConfig {
        description: Some("Test library".to_string()),
        license: Some("MIT".to_string()),
        repository: Some("https://github.com/test/my-lib".to_string()),
        homepage: None,
        authors: vec!["Alice".to_string()],
        keywords: vec!["test".to_string()],
        cargo: Some(cargo),
    });
    cfg
}

#[test]
fn cargo_config_default_renders_canonical_six_target_template() {
    let rendered = render_cargo_config(&ScaffoldCargo::default());

    // Header marker so finalize_hashes will stamp it.
    assert!(rendered.starts_with("# This file is auto-generated by alef. DO NOT EDIT.\n"));
    assert!(rendered.contains("# Re-generate with: alef scaffold\n"));

    // Canonical sections, in fixed order.
    assert!(rendered.contains("[build]\nincremental = true"));
    assert!(rendered.contains("[net]\ngit-fetch-with-cli = true"));
    assert!(rendered.contains("[registries.crates-io]\nprotocol = \"sparse\""));

    // All six target families present by default.
    assert!(rendered.contains("[target.'cfg(target_os = \"macos\")']"));
    assert!(rendered.contains("link-arg=-Wl,-undefined,dynamic_lookup"));
    assert!(rendered.contains("[target.x86_64-pc-windows-msvc]"));
    assert!(rendered.contains("[target.i686-pc-windows-msvc]"));
    assert!(rendered.contains("[target.aarch64-unknown-linux-gnu]"));
    assert!(rendered.contains("[target.x86_64-unknown-linux-musl]"));
    assert!(rendered.contains("[target.wasm32-unknown-unknown]"));
    assert!(rendered.contains("getrandom_backend=\\\"wasm_js\\\""));

    // No [env] block when none declared.
    assert!(!rendered.contains("\n[env]\n"));
}

#[test]
fn cargo_config_re_render_is_byte_identical() {
    let cargo = ScaffoldCargo::default();
    let first = render_cargo_config(&cargo);
    let second = render_cargo_config(&cargo);
    assert_eq!(first, second);
}

#[test]
fn cargo_config_disabling_individual_targets_omits_their_blocks() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets {
            i686_pc_windows_msvc: false,
            x86_64_unknown_linux_musl: false,
            ..ScaffoldCargoTargets::default()
        },
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(!rendered.contains("[target.i686-pc-windows-msvc]"));
    assert!(!rendered.contains("[target.x86_64-unknown-linux-musl]"));
    // Other targets remain.
    assert!(rendered.contains("[target.x86_64-pc-windows-msvc]"));
    assert!(rendered.contains("[target.aarch64-unknown-linux-gnu]"));
    assert!(rendered.contains("[target.'cfg(target_os = \"macos\")']"));
}

#[test]
fn cargo_config_disabling_macos_omits_dynamic_lookup() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets {
            macos_dynamic_lookup: false,
            ..ScaffoldCargoTargets::default()
        },
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(!rendered.contains("dynamic_lookup"));
    assert!(!rendered.contains("cfg(target_os = \"macos\")"));
}

#[test]
fn cargo_config_env_plain_string_renders_into_env_block() {
    let mut env = std::collections::HashMap::new();
    env.insert("MY_VAR".to_string(), ScaffoldCargoEnvValue::Plain("hello".to_string()));
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        env,
    };
    let rendered = render_cargo_config(&cargo);
    assert!(rendered.contains("\n[env]\n"));
    assert!(rendered.contains("MY_VAR = \"hello\"\n"));
}

#[test]
fn cargo_config_env_structured_value_renders_with_relative() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "RUBY".to_string(),
        ScaffoldCargoEnvValue::Structured {
            value: "scripts/preferred-ruby.sh".to_string(),
            relative: true,
        },
    );
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        env,
    };
    let rendered = render_cargo_config(&cargo);
    assert!(rendered.contains("[env]\n"));
    assert!(rendered.contains("RUBY = { value = \"scripts/preferred-ruby.sh\", relative = true }\n"));
}

#[test]
fn cargo_config_env_keys_are_sorted_for_determinism() {
    let mut env = std::collections::HashMap::new();
    env.insert("ZED".to_string(), ScaffoldCargoEnvValue::Plain("z".to_string()));
    env.insert("ALPHA".to_string(), ScaffoldCargoEnvValue::Plain("a".to_string()));
    env.insert("MID".to_string(), ScaffoldCargoEnvValue::Plain("m".to_string()));
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        env,
    };
    let rendered = render_cargo_config(&cargo);
    let env_section = rendered.split("[env]\n").nth(1).expect("env section present");
    let alpha_pos = env_section.find("ALPHA").expect("ALPHA present");
    let mid_pos = env_section.find("MID").expect("MID present");
    let zed_pos = env_section.find("ZED").expect("ZED present");
    assert!(alpha_pos < mid_pos);
    assert!(mid_pos < zed_pos);
}

#[test]
fn cargo_config_env_string_with_quotes_is_escaped() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "QUOTED".to_string(),
        ScaffoldCargoEnvValue::Plain(r#"a"b\c"#.to_string()),
    );
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        env,
    };
    let rendered = render_cargo_config(&cargo);
    // Backslashes doubled, quotes escaped.
    assert!(rendered.contains("QUOTED = \"a\\\"b\\\\c\"\n"));
}

#[test]
fn scaffold_emits_cargo_config_when_scaffold_cargo_is_set() {
    let config = cargo_only_config(ScaffoldCargo::default());
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let cargo_file = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .expect(".cargo/config.toml should be emitted when [scaffold.cargo] is set");
    assert!(
        cargo_file.generated_header,
        "generated_header must be true so verify walks it"
    );
    assert!(cargo_file.content.contains("auto-generated by alef"));
    assert!(cargo_file.content.contains("dynamic_lookup"));
    assert!(cargo_file.content.contains("[target.x86_64-pc-windows-msvc]"));
}

#[test]
fn scaffold_skips_cargo_config_in_legacy_mode_when_file_exists() {
    // No `[scaffold.cargo]` opt-in. Existing-file check is filesystem-bound, so
    // we only assert that scaffold() does not panic and produces no `.cargo/config.toml`
    // entry when the legacy create-if-missing branch detects the file already exists.
    // (The existing tests filter `.cargo/config.toml` out via `language_files()`,
    // implicitly relying on this branch never producing a hash-headered file.)
    let config = test_config(); // scaffold.cargo is None
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let cargo_files: Vec<_> = all_files
        .iter()
        .filter(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .collect();
    // Legacy branch is gated on Wasm + !exists. We're not requesting Wasm here,
    // so no .cargo/config.toml should appear regardless of filesystem state.
    assert!(
        cargo_files.is_empty(),
        "legacy branch should not emit .cargo/config.toml without Wasm",
    );
}

#[test]
fn scaffold_emits_cargo_config_with_env_block_for_h2m_style_ruby_path() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "RUBY".to_string(),
        ScaffoldCargoEnvValue::Structured {
            value: "scripts/preferred-ruby.sh".to_string(),
            relative: true,
        },
    );
    let config = cargo_only_config(ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        env,
    });
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let cargo_file = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .expect(".cargo/config.toml should be emitted");
    assert!(cargo_file.content.contains("[env]\n"));
    assert!(
        cargo_file
            .content
            .contains("RUBY = { value = \"scripts/preferred-ruby.sh\", relative = true }")
    );
}
