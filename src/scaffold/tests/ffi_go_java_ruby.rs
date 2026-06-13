use super::*;

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
fn test_scaffold_ffi_injects_version_for_workspace_member_deps() {
    use std::fs;
    use tempfile::TempDir;

    // `cargo publish` rejects path-only deps: "all dependencies must have a
    // version requirement specified when publishing". Every internal workspace
    // dep the FFI/umbrella manifest pulls in (auto-detected from the public
    // surface via `[crate.extra_dependencies]`) must therefore carry the
    // resolved workspace version alongside its path, mirroring the core dep.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-core", "crates/my-lib-http"]

[workspace.package]
version = "4.2.0"
"#,
    )
    .unwrap();
    for member in ["my-lib-core", "my-lib-http"] {
        fs::create_dir_all(root.join(format!("crates/{member}/src"))).unwrap();
        fs::write(root.join(format!("crates/{member}/src/lib.rs")), "pub fn f() {}").unwrap();
        fs::write(
            root.join(format!("crates/{member}/Cargo.toml")),
            format!("[package]\nname = \"{member}\"\nversion.workspace = true\n"),
        )
        .unwrap();
    }

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    let mut deps: std::collections::HashMap<String, toml::Value> = Default::default();
    // Path-only internal workspace member deps (as auto-detected and emitted
    // into [crate.extra_dependencies]).
    for member in ["my-lib-core", "my-lib-http"] {
        deps.insert(
            member.to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "path".to_string(),
                toml::Value::String(format!("../{member}")),
            )])),
        );
    }
    // A genuinely external dep must stay untouched (no spurious version inject).
    deps.insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies = deps;

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    for member in ["my-lib-core", "my-lib-http"] {
        // Each internal member dep must carry the injected workspace version.
        assert!(
            cargo_toml.contains(&format!("{member} = {{ path = \"../{member}\", version = \"4.2.0\" }}")),
            "FFI manifest must version-inject internal workspace dep {member}; got:\n{cargo_toml}"
        );
    }
    // External dep unchanged.
    assert!(
        cargo_toml.contains("anyhow = \"1.0\""),
        "external dep must be emitted unchanged, got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_ffi_target_dep_overrides_emit_cfg_blocks() {
    // When FfiConfig.target_dep_overrides is configured, the core-crate
    // dependency moves out of the main [dependencies] table into per-cfg
    // [target.'cfg(...)'.dependencies] tables. This is the only shape that
    // satisfies targets whose feature set differs from the default, e.g.
    // x86_64-linux-android (no ONNX Runtime prebuilt) needs the
    // `android-target` feature instead of `full`.
    use crate::core::config::FfiTargetDepOverride;
    use crate::core::config::languages::FfiConfig;

    let mut config = test_config();
    config.features = vec!["full".to_string(), "ocr".to_string()];
    config.ffi = Some(FfiConfig {
        prefix: None,
        error_style: "last_error".to_string(),
        header_name: None,
        lib_name: None,
        visitor_callbacks: false,
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        rename_fields: Default::default(),
        plugin_error_constructor: None,
        target_dep_overrides: vec![FfiTargetDepOverride {
            cfg: "all(target_os = \"android\", target_arch = \"x86_64\")".to_string(),
            features: vec!["android-target".to_string()],
        }],
    });

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    // The default branch is wrapped in cfg(not(<override-cfg>)).
    assert!(
        cargo_toml.contains("[target.'cfg(not(all(target_os = \"android\", target_arch = \"x86_64\")))'.dependencies]"),
        "expected default-branch target table with cfg(not(...)), got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib = { path = \"../my-lib\", version = \"0.1.0\", features = [\"full\", \"ocr\"] }"),
        "default branch should keep the full feature set, got:\n{cargo_toml}"
    );

    // The override branch keeps the explicit cfg and a reduced feature set.
    assert!(
        cargo_toml.contains("[target.'cfg(all(target_os = \"android\", target_arch = \"x86_64\"))'.dependencies]"),
        "expected override target table, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib = { path = \"../my-lib\", version = \"0.1.0\", features = [\"android-target\"] }"),
        "override branch should emit android-target feature, got:\n{cargo_toml}"
    );

    // The main [dependencies] table still exists for ahash/serde_json/tokio but
    // no longer contains the core-crate line.
    assert!(cargo_toml.contains("[dependencies]\nahash = \"0.8\""));
    assert!(
        !cargo_toml.contains("\n[dependencies]\nmy-lib ="),
        "core-crate dep should have moved out of [dependencies], got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_go_production_format() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    // go.mod + .golangci.yml + .lib/.gitkeep
    assert_eq!(files.len(), 3);
    let content = &files[0].content;
    assert!(content.contains("go 1.26"));
    assert!(!content.contains("require ("));
}

#[test]
fn test_scaffold_go_uses_inert_module_when_unconfigured() {
    let config = minimal_config_from_toml(
        r#"
[crates.go]
module_major = 5
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    let go_mod = files
        .iter()
        .find(|f| f.path == Path::new("packages/go/v5/go.mod"))
        .expect("go.mod must be emitted");

    assert!(
        go_mod.content.starts_with("module example.invalid/my-lib\n"),
        "unconfigured Go scaffold must use inert example.invalid fallback, got:\n{}",
        go_mod.content
    );
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
fn test_scaffold_java_scm_uses_configured_non_github_host() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://gitlab.example.com/acme/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files
        .iter()
        .find(|f| f.path == Path::new("packages/java/pom.xml"))
        .expect("pom.xml must be emitted");

    assert!(pom.content.contains("scm:git:git://gitlab.example.com/acme/my-lib.git"));
    assert!(
        pom.content
            .contains("scm:git:ssh://git@gitlab.example.com/acme/my-lib.git")
    );
    assert!(!pom.content.contains("github.com/acme/my-lib"));
}

#[test]
fn test_scaffold_ruby_production_features() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    // scaffold_ruby: gemspec, rubocop, Rakefile, extconf.rb, Gemfile, Steepfile = 6 files.
    // The `lib/<gem>.rb` entry point is emitted by the magnus backend (gen_bindings),
    // not the scaffold — it requires `<gem>/native` and `<gem>/version`.
    // scaffold_ruby_cargo: Cargo.toml = 1 file
    assert_eq!(files.len(), 7);
    let content = &files[0].content;
    assert!(content.contains("spec.required_ruby_version"));
    assert!(content.contains("spec.extensions"));
    assert!(content.contains("README*"));
    assert!(content.contains("LICENSE*"));
    assert!(content.contains("lib/**/*"));
    assert!(content.contains("ext/**/*"));
    assert!(content.contains("sig/**/*"));
    assert!(content.contains("spec.metadata[\"keywords\"]"));
    assert!(content.contains("frozen_string_literal: true"));
    assert!(content.contains("spec.metadata[\"rubygems_mfa_required\"] = \"true\""));
    // Check for .rubocop.yml generation
    assert_eq!(files[1].path, PathBuf::from("packages/ruby/.rubocop.yml"));
    // Check for Rakefile generation
    assert_eq!(files[2].path, PathBuf::from("packages/ruby/Rakefile"));
    assert!(files[2].content.contains("RbSys::ExtensionTask"));
    assert!(files[2].content.contains("my_lib_rb"));
    assert!(files[2].content.contains("require \"rb_sys/extensiontask\""));
    assert!(files[2].content.contains("MANIFEST_PATH"));
    assert!(files[2].content.contains("--manifest-path"));
    assert!(files[2].content.contains("task compile: \"compile:ruby\""));
    // Check for extconf.rb generation
    assert_eq!(
        files[3].path,
        PathBuf::from("packages/ruby/ext/my_lib_rb/native/extconf.rb")
    );
    assert!(files[3].content.contains("create_rust_makefile"));
    assert!(files[3].content.contains("rb_sys/mkmf"));
    assert!(
        files[3].content.contains("config.ext_dir = \".\""),
        "extconf.rb must set ext_dir = \".\" so rb_sys finds the sibling Cargo.toml"
    );
    // files[4] is Gemfile; files[5] is Steepfile; files[6] is the Cargo.toml from scaffold_ruby_cargo
    assert_eq!(files[4].path, PathBuf::from("packages/ruby/Gemfile"));
    assert_eq!(files[5].path, PathBuf::from("packages/ruby/Steepfile"));
    // Check for Cargo.toml generation
    assert_eq!(
        files[6].path,
        PathBuf::from("packages/ruby/ext/my_lib_rb/native/Cargo.toml")
    );
    assert!(files[6].content.contains("magnus"));
    assert!(
        files[6].content.contains("path = \"../src/lib.rs\""),
        "Ruby Cargo.toml [lib] must set path to the binding source crate"
    );
}

/// Regression: the generated gemspec must declare `sorbet-runtime` as a runtime
/// dependency so projects running `bundle install --without development` can load
/// the `native.rb` wrapper, which unconditionally `require 'sorbet-runtime'`.
/// Missing the dep caused `LoadError: cannot load such file -- sorbet-runtime`
/// in CI E2E runs.
#[test]
fn test_scaffold_ruby_gemspec_includes_sorbet_runtime_dependency() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    // files[0] is the gemspec
    let gemspec = &files[0].content;
    assert!(
        gemspec.contains("sorbet-runtime"),
        "gemspec must add sorbet-runtime as a runtime dependency; got:\n{gemspec}"
    );
    assert!(
        gemspec.contains("spec.add_dependency \"sorbet-runtime\""),
        "gemspec must use spec.add_dependency (not add_development_dependency) for sorbet-runtime; got:\n{gemspec}"
    );
    assert!(
        gemspec.contains("~> 0.5"),
        "sorbet-runtime dependency must carry a ~> 0.5 version constraint; got:\n{gemspec}"
    );
}

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
    // LineLength max is 200 to accommodate the alef-emitted DefaultClient FFM
    // call shims (single-line chains of arena allocation + marshalling that
    // don't reflow cleanly within shorter limits).
    assert!(checkstyle.content.contains("\"200\""));
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
    // The csproj lives at packages/csharp/<Namespace>/<Namespace>.csproj (3 levels deep),
    // so ../../../LICENSE correctly reaches the workspace root.
    assert!(
        content.contains(r#"Include="../../../LICENSE""#),
        "LICENSE path must be ../../../LICENSE to reach workspace root: {content}"
    );
    assert!(
        content.contains("<Version>1.2.3</Version>"),
        "version must be substituted: {content}"
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
        xml.content.contains(r#"value="checkstyle-suppressions.xml""#),
        "checkstyle suppressions path must be relative to project basedir; content:\n{}",
        xml.content
    );
    let properties = files
        .iter()
        .find(|f| f.path.ends_with("checkstyle.properties"))
        .unwrap();
    assert!(
        properties.content.is_empty(),
        "checkstyle properties must be empty (0 bytes) so end-of-file-fixer leaves it untouched on every regen; a lone trailing newline gets stripped back to empty; content:\n{}",
        properties.content
    );
}

#[test]
fn test_ruby_cargo_machete_rb_sys_only() {
    // Regression test: v0.22.25 fixed the mingw sysroot bug via a cargo-dep pin on rb-sys.
    // The NIF code now directly uses `tokio` and `async-trait` (not just transitively through
    // the core crate), so they must NOT be in the cargo-machete ignored list. Only `rb-sys`
    // should be ignored (it's pinned but used transitively through Magnus).
    use crate::core::ir::*;

    let config = test_config_from_toml(
        r#"
[crates.ruby]
gem_name = "test_lib"
"#,
    );

    // Minimal ApiSurface with no async/trait bridges to verify the baseline cargo-machete section
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let result = crate::scaffold::languages::scaffold_ruby_cargo(&api, &config);
    assert!(result.is_ok(), "scaffold_ruby_cargo should succeed");

    let files = result.unwrap();
    let cargo_toml_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Should generate Cargo.toml");

    let content = &cargo_toml_file.content;

    // Verify the cargo-machete section exists and contains only rb-sys
    assert!(
        content.contains("[package.metadata.cargo-machete]"),
        "Should contain [package.metadata.cargo-machete] section; got:\n{}",
        content
    );

    assert!(
        content.contains("ignored = [\"rb-sys\"]"),
        "Should ignore only rb-sys (pinned for mingw sysroot bug but used transitively through Magnus); got:\n{}",
        content
    );

    // Verify that the ignored list does NOT contain the conditional deps
    // (tokio, async-trait, futures, ahash are now directly used by NIF code and should not be ignored)
    let ignored_section = content
        .split("[package.metadata.cargo-machete]")
        .nth(1)
        .and_then(|s| s.split("[lib]").next())
        .unwrap_or("");

    assert!(
        !ignored_section.contains("\"tokio\""),
        "tokio should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"async-trait\""),
        "async-trait should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"futures\""),
        "futures should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"ahash\""),
        "ahash should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
}
