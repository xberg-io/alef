use super::*;

#[test]
fn test_scaffold_ffi_with_core_import() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 2);
    let cargo_toml = &files[0].content;
    assert!(cargo_toml.contains("serde = \"1\""));
    assert!(cargo_toml.contains("serde_json"));
    assert!(cargo_toml.contains("my-lib ="));
    let cmake = &files[1].content;
    assert!(cmake.contains("find_package"));
    assert!(cmake.contains("my-lib-ffi::my-lib-ffi"));
}

#[test]
fn test_scaffold_ffi_deps_are_pinned() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;
    assert!(
        cargo_toml.contains("cbindgen = \"0.29\""),
        "cbindgen should be pinned to a specific minor for reproducible headers"
    );
    assert!(cargo_toml.contains("serde = \"1\""));
    assert!(cargo_toml.contains("ignored = [\"ahash\", \"serde\", \"serde_json\", \"tokio\"]"));
    assert!(cargo_toml.contains("serde_json = \"1\""));
    assert!(cargo_toml.contains("tokio = "));
    assert!(cargo_toml.contains("[dev-dependencies]"));
    assert!(cargo_toml.contains("tempfile = \"3\""));
}

#[test]
fn test_scaffold_ffi_merges_extra_dependencies() {
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
    for member in ["my-lib-core", "my-lib-http"] {
        deps.insert(
            member.to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "path".to_string(),
                toml::Value::String(format!("../{member}")),
            )])),
        );
    }
    deps.insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies = deps;

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    for member in ["my-lib-core", "my-lib-http"] {
        assert!(
            cargo_toml.contains(&format!("{member} = {{ path = \"../{member}\", version = \"4.2.0\" }}")),
            "FFI manifest must version-inject internal workspace dep {member}; got:\n{cargo_toml}"
        );
    }
    assert!(
        cargo_toml.contains("anyhow = \"1.0\""),
        "external dep must be emitted unchanged, got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_ffi_target_dep_overrides_emit_cfg_blocks() {
    // [target.'cfg(...)'.dependencies] tables. This is the only shape that
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
        extra_features: vec![],
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        capsule_types: Default::default(),
        rename_fields: Default::default(),
        plugin_error_constructor: None,
        target_dep_overrides: vec![FfiTargetDepOverride {
            cfg: "all(target_os = \"android\", target_arch = \"x86_64\")".to_string(),
            features: vec!["android-target".to_string()],
            default_features: true,
        }],
        excluded_default_features: vec![],
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
        cargo_toml.contains("my-lib = { path = \"../..\", version = \"0.1.0\", features = [\"full\", \"ocr\"] }"),
        "default branch should keep the full feature set, got:\n{cargo_toml}"
    );

    assert!(
        cargo_toml.contains("[target.'cfg(all(target_os = \"android\", target_arch = \"x86_64\"))'.dependencies]"),
        "expected override target table, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib = { path = \"../..\", version = \"0.1.0\", features = [\"android-target\"] }"),
        "override branch should emit android-target feature, got:\n{cargo_toml}"
    );

    assert!(cargo_toml.contains("[dependencies]\nahash = \"0.8\""));
    assert!(
        !cargo_toml.contains("\n[dependencies]\nmy-lib ="),
        "core-crate dep should have moved out of [dependencies], got:\n{cargo_toml}"
    );
}

/// Regression test for the dropped `[crates.ffi.target_dep_overrides].default_features`
/// key: a real crate's `windows-target` / `macos-intel-target` overrides set
/// `default_features = false` to drop the core crate's own `default = ["tokio-runtime",
/// "simd-utf8"]` set on those targets, alongside swapping in the reduced feature list.
/// Before `FfiTargetDepOverride` gained this field the key was silently discarded, so the
/// override target block always inherited the core dep's default features regardless of
/// what the config said.
#[test]
fn test_scaffold_ffi_target_dep_overrides_default_features_false_drops_defaults() {
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
        extra_features: vec![],
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        capsule_types: Default::default(),
        rename_fields: Default::default(),
        plugin_error_constructor: None,
        target_dep_overrides: vec![FfiTargetDepOverride {
            cfg: "target_os = \"windows\"".to_string(),
            features: vec!["windows-target".to_string()],
            default_features: false,
        }],
        excluded_default_features: vec![],
    });

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains("[target.'cfg(target_os = \"windows\")'.dependencies]"),
        "expected the windows override target table, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(
            "my-lib = { path = \"../..\", version = \"0.1.0\", default-features = false, features = [\"windows-target\"] }"
        ),
        "default_features: false must drop the core dep's default features on the override branch, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(
            "[target.'cfg(not(target_os = \"windows\"))'.dependencies]\nmy-lib = { path = \"../..\", version = \"0.1.0\", features = [\"full\", \"ocr\"] }"
        ),
        "the default (non-overridden) branch must keep the full default feature set with no default-features key, got:\n{cargo_toml}"
    );
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders
/// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
/// predicate string (plain byte-wise comparison), NOT with the default
/// `cfg(not(any(...)))` branch always first. With multiple overrides whose
/// combined cfg is wrapped in `any(...)`, an `all(...)`-prefixed override (as
/// a real crate configures for its macOS-Intel target) must sort *before* the
/// `not(any(...))` default branch — `'a'` < `'n'` — while a `target_os = ...`
/// override sorts after it (`'n'` < `'t'`).
#[test]
fn test_scaffold_ffi_target_dep_overrides_sort_all_before_not() {
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
        extra_features: vec![],
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        capsule_types: Default::default(),
        rename_fields: Default::default(),
        plugin_error_constructor: None,
        target_dep_overrides: vec![
            FfiTargetDepOverride {
                cfg: "target_os = \"android\"".to_string(),
                features: vec!["android-target".to_string()],
                default_features: true,
            },
            FfiTargetDepOverride {
                cfg: "target_os = \"windows\"".to_string(),
                features: vec!["windows-target".to_string()],
                default_features: true,
            },
            FfiTargetDepOverride {
                cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
                features: vec!["macos-intel-target".to_string()],
                default_features: true,
            },
        ],
        excluded_default_features: vec![],
    });

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    let all_pos = cargo_toml
        .find("[target.'cfg(all(target_os = \"macos\", target_arch = \"x86_64\"))'.dependencies]")
        .expect("expected the macOS-Intel `all(...)` override block");
    let not_pos = cargo_toml
        .find("[target.'cfg(not(any(")
        .expect("expected the default `not(any(...))` block");
    let android_pos = cargo_toml
        .find("[target.'cfg(target_os = \"android\")'.dependencies]")
        .expect("expected the android override block");

    assert!(
        all_pos < not_pos,
        "the `all(...)` override must sort BEFORE the `not(...)` default branch; got:\n{cargo_toml}"
    );
    assert!(
        not_pos < android_pos,
        "the `not(...)` default branch must sort before `target_os = \"android\"`; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated FFI Cargo.toml must be valid TOML");
}

#[test]
fn test_scaffold_ffi_emits_android_target_aggregate_feature() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample_crate\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/sample_crate/src")).unwrap();
    fs::write(root.join("crates/sample_crate/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/sample_crate/Cargo.toml"),
        r#"[package]
name = "sample_crate"
version = "0.1.0"

[features]
android-target = ["no-ort-target", "ocr"]
no-ort-target = ["pdf", "html"]
pdf = []
html = []
ocr = []
embeddings = []
"#,
    )
    .unwrap();

    let mut config = test_config();
    config.name = "sample_crate".to_string();
    config.workspace_root = Some(root.to_path_buf());
    config.sources = vec![PathBuf::from("crates/sample_crate/src/lib.rs")];
    config.features = vec![
        "full".to_string(),
        "pdf".to_string(),
        "ocr".to_string(),
        "html".to_string(),
        "embeddings".to_string(),
    ];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains(r#"android-target = ["sample_crate/android-target", "html", "ocr", "pdf"]"#),
        "FFI manifest must emit the android-target aggregate feature; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn test_scaffold_ffi_omits_android_target_when_core_lacks_it() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample_crate\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/sample_crate/src")).unwrap();
    fs::write(root.join("crates/sample_crate/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/sample_crate/Cargo.toml"),
        "[package]\nname = \"sample_crate\"\nversion = \"0.1.0\"\n\n[features]\npdf = []\nocr = []\n",
    )
    .unwrap();

    let mut config = test_config();
    config.name = "sample_crate".to_string();
    config.workspace_root = Some(root.to_path_buf());
    config.sources = vec![PathBuf::from("crates/sample_crate/src/lib.rs")];
    config.features = vec!["pdf".to_string(), "ocr".to_string()];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    assert!(
        !cargo_toml.contains("android-target"),
        "FFI manifest must not emit android-target when core crate lacks it; got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_go_production_format() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 3);
    let content = &files[0].content;
    assert!(content.contains("go 1.26"));
    assert!(!content.contains("require ("));
}

#[test]
fn test_scaffold_go_injects_capsule_require() {
    let config = minimal_config_from_toml(
        r#"
[crates.go]
module = "github.com/test/my-lib"

[crates.go.capsule_types.Language]
host_type = "*tree_sitter.Language"
package = "github.com/tree-sitter/go-tree-sitter"
package_version = "v0.25.0"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    let go_mod = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("go.mod"))
        .expect("go.mod must be emitted");
    assert!(
        go_mod.content.contains("github.com/tree-sitter/go-tree-sitter v0.25.0"),
        "go.mod must require the go-tree-sitter capsule package, got:\n{}",
        go_mod.content
    );
    assert!(go_mod.content.contains("require ("), "go.mod must have a require block");
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
    assert_eq!(files.len(), 5);
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
    // The `spec/<gem>_spec.rb` seed is appended last inside `scaffold_ruby`, so it takes index
    // 6 and pushes `scaffold_ruby_cargo`'s manifest — extended onto the tail by the `Language::
    // Ruby` scaffold arm, and therefore always last overall — from 6 to 7. Every index before
    // it is unchanged. ~keep
    assert_eq!(files.len(), 8);
    let content = &files[0].content;
    assert!(content.contains(r#"spec.required_ruby_version = ">= 3.2.0""#));
    assert!(!content.contains("< 4.0"));
    assert!(content.contains("spec.extensions"));
    assert!(content.contains("README*"));
    assert!(content.contains("LICENSE*"));
    assert!(content.contains("lib/**/*"));
    assert!(content.contains("ext/**/*"));
    assert!(content.contains("sig/**/*"));
    assert!(content.contains("spec.metadata[\"keywords\"]"));
    assert!(content.contains("frozen_string_literal: true"));
    assert!(content.contains("spec.metadata[\"rubygems_mfa_required\"] = \"true\""));
    assert_eq!(files[1].path, PathBuf::from("packages/ruby/.rubocop.yml"));
    assert_eq!(files[2].path, PathBuf::from("packages/ruby/Rakefile"));
    assert!(files[2].content.contains("RbSys::ExtensionTask"));
    assert!(files[2].content.contains("my_lib_rb"));
    assert!(files[2].content.contains("require \"rb_sys/extensiontask\""));
    assert!(files[2].content.contains("MANIFEST_PATH"));
    assert!(files[2].content.contains("--manifest-path"));
    assert!(files[2].content.contains("task compile: \"compile:ruby\""));
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
    assert_eq!(files[4].path, PathBuf::from("packages/ruby/Gemfile"));
    assert_eq!(files[5].path, PathBuf::from("packages/ruby/Steepfile"));
    assert_eq!(files[6].path, PathBuf::from("packages/ruby/spec/my_lib_spec.rb"));
    assert!(
        !files[6].generated_header,
        "the spec seed must stay create-only so a real suite is never overwritten"
    );
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
fn test_scaffold_ruby_gemspec_excludes_native_build_artifacts() {
    let all_files = scaffold(&test_api(), &test_config(), &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let gemspec = &files[0].content;

    assert!(gemspec.contains("/(?:target|tmp)/"));
    assert!(gemspec.contains("\\.(?:bundle|so|dylib|dll|o|a|log)\\z"));
    assert!(gemspec.contains("\\.dSYM/"));
}

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
    assert!(!golangci.content.contains("linters-settings:"));
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
    let csproj = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".csproj"))
        .expect("C# scaffold must produce a .csproj file");
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
        !csproj
            .content
            .contains("<GenerateAssemblyInfo>false</GenerateAssemblyInfo>"),
        "csproj must NOT suppress SDK AssemblyInfo so version stays in sync with <Version> tag"
    );
    assert!(
        csproj.content.contains("<Company>Alice</Company>"),
        "csproj must derive Company from the configured authors (first author), \
         not a hardcoded alef literal, to provide SDK-generated AssemblyCompanyAttribute; got:\n{}",
        csproj.content
    );
    assert!(
        csproj.content.contains("<Product>"),
        "csproj must set Product to provide SDK-generated AssemblyProductAttribute"
    );
    assert!(
        !csproj.generated_header,
        "csproj must be scaffold-once (generated_header = false)"
    );
}

#[test]
fn test_render_csharp_csproj_omits_project_url_and_tags_when_unconfigured() {
    // `test_config()` sets `[crates.scaffold] keywords = ["test"]`, which is a
    // genuinely-configured keyword list, not an unconfigured one — using it here
    // would assert PackageTags is absent while the fixture guarantees it's present.
    // `minimal_config_from_toml` carries no `[crates.scaffold]`/`[crates.package_metadata]`
    // at all, so `ScaffoldMeta.homepage`/`.keywords` are genuinely empty. ~keep
    let config = minimal_config_from_toml("");
    let content = render_csharp_csproj(&config, "1.2.3");

    assert!(
        !content.contains("<PackageProjectUrl>"),
        "unconfigured homepage must not invent a PackageProjectUrl: {content}"
    );
    assert!(
        !content.contains("<PackageTags>"),
        "unconfigured keywords must not invent PackageTags: {content}"
    );
}

#[test]
fn test_render_csharp_csproj_includes_project_url_and_tags_when_configured() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
homepage = "https://example.com/my-lib"
keywords = ["llm", "bindings"]
"#,
    );
    let meta_csproj = render_csharp_csproj(&config, "1.2.3");
    let meta_runtime_csproj = render_csharp_runtime_csproj(&config, "1.2.3");

    for content in [&meta_csproj, &meta_runtime_csproj] {
        assert!(
            content.contains("<PackageProjectUrl>https://example.com/my-lib</PackageProjectUrl>"),
            "csproj must carry the configured homepage as PackageProjectUrl: {content}"
        );
        assert!(
            content.contains("<PackageTags>bindings;llm</PackageTags>"),
            "csproj must carry the configured keywords (alphabetised by scaffold_meta) as \
             semicolon-separated PackageTags: {content}"
        );
    }
}

#[test]
fn test_render_csharp_runtime_csproj_derives_company_from_authors() {
    let config = test_config();
    let content = render_csharp_runtime_csproj(&config, "1.2.3");

    assert!(
        content.contains("<Company>Alice</Company>"),
        "runtime csproj must derive Company from the configured authors, not a hardcoded alef literal: {content}"
    );
}

#[test]
fn test_render_csharp_csproj_rid_comment_matches_thin_meta_package_reality() {
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.2.3");

    assert!(
        !content.contains("Native asset resolution at consumer build time is driven by the\n         `runtimes/<rid>/native/` payload baked into the NuGet package"),
        "the RID comment must not claim this package embeds runtimes/<rid>/native/ payload — \
         the surrounding csproj is explicitly a thin meta-package that carries none: {content}"
    );
    assert!(
        content.contains("RID-fallback graph"),
        "the RID comment must describe NuGet's RID-fallback graph as the real resolution mechanism: {content}"
    );
}

#[test]
fn test_render_csharp_csproj_is_thin_meta_package() {
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.2.3");
    assert!(
        !content.contains(r#"Include="runtimes/**""#),
        "meta csproj must NOT pack the fat runtimes/** payload (413 regression): {content}"
    );
    assert!(
        content.contains(r#"Include="runtime.json" Pack="true" PackagePath="/" Condition="Exists('runtime.json')""#),
        "meta csproj must pack the thin runtime.json RID-fallback graph: {content}"
    );
    assert!(
        content.contains(r#"<Target Name="RequireRuntimeJson" BeforeTargets="Pack">"#),
        "meta csproj must hard-error if runtime.json is missing before pack: {content}"
    );
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
fn test_render_csharp_csproj_stamps_assembly_version_properties() {
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.9.0-rc.48");

    assert!(
        content.contains("<Version>1.9.0-rc.48</Version>"),
        "Version must carry the full SemVer including prerelease: {content}"
    );
    assert!(
        content.contains("<InformationalVersion>1.9.0-rc.48</InformationalVersion>"),
        "InformationalVersion must carry the full SemVer for diagnostics: {content}"
    );

    assert!(
        content.contains("<AssemblyVersion>1.9.0.0</AssemblyVersion>"),
        "AssemblyVersion must be a 4-component numeric (prerelease stripped): {content}"
    );
    assert!(
        content.contains("<FileVersion>1.9.0.0</FileVersion>"),
        "FileVersion must be a 4-component numeric (prerelease stripped): {content}"
    );

    assert!(
        !content.contains("0.0.0.0"),
        "no version property may be 0.0.0.0: {content}"
    );
}

#[test]
fn test_render_csharp_csproj_advertises_all_published_runtime_identifiers() {
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.9.0-rc.48");

    for rid in [
        "win-x64",
        "win-arm64",
        "linux-x64",
        "linux-arm64",
        "osx-x64",
        "osx-arm64",
    ] {
        assert!(
            content.contains(rid),
            "RuntimeIdentifiers must include {rid}: {content}"
        );
    }
    assert!(
        content.contains("<RuntimeIdentifiers>"),
        "csproj must declare <RuntimeIdentifiers> (plural) for multi-RID packaging: {content}"
    );
    assert!(
        content.contains("<PlatformTarget>AnyCPU</PlatformTarget>"),
        "managed assembly must be AnyCPU so PE Machine header stays processor-neutral: {content}"
    );
    assert!(
        !content.contains("<RuntimeIdentifier Condition="),
        "package csproj must NOT use conditional singular <RuntimeIdentifier> (forces runtime-specific build): {content}"
    );
}

#[test]
fn test_scaffold_csharp_emits_runtime_json_template() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    let template = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("runtime.json.template"))
        .expect("C# scaffold must emit runtime.json.template so CI can render runtime.json before pack");

    assert_eq!(
        template.path,
        PathBuf::from("packages/csharp/MyLib/runtime.json.template"),
        "runtime.json.template must sit beside the csproj so the RequireRuntimeJson target finds it"
    );
    assert!(
        !template.generated_header,
        "runtime.json.template must be scaffold-once (no DO NOT EDIT header — it is JSON)"
    );
    assert!(
        template.content.contains("{{VERSION}}"),
        "template must keep the literal version placeholder for CI substitution: {}",
        template.content
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&template.content).expect("runtime.json.template must be valid JSON");
    let runtimes = parsed["runtimes"]
        .as_object()
        .expect("runtime.json must have a runtimes object");

    // Each enabled glibc/windows/macos RID pulls in its per-RID native package pinned
    // to the placeholder. Assert against the actual package id so the test tracks config.
    for rid in [
        "win-x64",
        "win-arm64",
        "linux-x64",
        "linux-arm64",
        "osx-x64",
        "osx-arm64",
    ] {
        let rid_entry = runtimes[rid]
            .as_object()
            .unwrap_or_else(|| panic!("RID {rid} entry must be an object"));
        assert_eq!(
            rid_entry.len(),
            1,
            "RID {rid} must map exactly one package id: {}",
            template.content
        );
        let (package_id, dependencies) = rid_entry.iter().next().unwrap();
        let expected_dependency = format!("{package_id}.runtime.{rid}");
        assert_eq!(
            dependencies.get(&expected_dependency).and_then(|value| value.as_str()),
            Some("{{VERSION}}"),
            "RID {rid} must depend on {expected_dependency} pinned to the version placeholder: {}",
            template.content
        );
    }

    // musl RIDs ship no native package of their own; they fall back to the glibc asset.
    assert_eq!(
        runtimes["linux-musl-x64"]["#import"][0].as_str(),
        Some("linux-x64"),
        "linux-musl-x64 must #import linux-x64: {}",
        template.content
    );
    assert_eq!(
        runtimes["linux-musl-arm64"]["#import"][0].as_str(),
        Some("linux-arm64"),
        "linux-musl-arm64 must #import linux-arm64: {}",
        template.content
    );
}

#[test]
fn test_scaffold_csharp_emits_runtime_project_for_meta_split() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    let runtime = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".Runtime.csproj"))
        .expect("C# scaffold must emit the native-only .Runtime project (native half of the meta+runtime split)");

    assert_eq!(
        runtime.path,
        PathBuf::from("packages/csharp/MyLib.Runtime/MyLib.Runtime.csproj"),
        "runtime project must be the sibling ../<Namespace>.Runtime the meta csproj references"
    );
    assert!(
        !runtime.generated_header,
        "runtime csproj must be scaffold-once (generated_header = false)"
    );
}

#[test]
fn test_render_csharp_runtime_csproj_is_native_only_per_rid_package() {
    let config = test_config();
    let content = render_csharp_runtime_csproj(&config, "1.9.0-rc.48");

    // PackageId is parameterized per RID so one project packs every enabled RID.
    assert!(
        content.contains("<PackageId>MyLib.runtime.$(PublishedRID)</PackageId>"),
        "runtime package id must be <PackageId>.runtime.$(PublishedRID): {content}"
    );
    assert!(
        content.contains("<PublishedRID Condition=\"'$(PublishedRID)' == ''\">"),
        "runtime csproj must default PublishedRID so a bare pack still evaluates: {content}"
    );
    // Native payload comes from the meta package's staged runtimes/ dir.
    assert!(
        content.contains(
            r#"Include="../MyLib/runtimes/$(PublishedRID)/native/**" Pack="true" PackagePath="runtimes/$(PublishedRID)/native/""#
        ),
        "runtime csproj must pack the meta package's staged native payload under runtimes/<rid>/native/: {content}"
    );
    // Native-only: no managed assembly, deps suppressed, NU5128 tolerated.
    assert!(
        content.contains("<IncludeBuildOutput>false</IncludeBuildOutput>"),
        "runtime package must not ship a managed assembly: {content}"
    );
    assert!(
        content.contains("NU5128"),
        "runtime csproj must suppress the native-only NU5128 warning: {content}"
    );
    // Hard-error guard so an empty native payload fails the pack loudly.
    assert!(
        content.contains(r#"<Target Name="RequireRuntimeAssets" BeforeTargets="Pack">"#),
        "runtime csproj must guard against packing without staged natives: {content}"
    );
    assert!(
        content.contains("<Version>1.9.0-rc.48</Version>"),
        "runtime version must be substituted: {content}"
    );
    assert!(
        content.contains(r#"Include="../../../LICENSE""#),
        "runtime project sits at the same depth as the meta package, so LICENSE is ../../../LICENSE: {content}"
    );
}

#[test]
fn test_ruby_cargo_machete_rb_sys_only() {
    use crate::core::ir::*;

    let config = test_config_from_toml(
        r#"
[crates.ruby]
gem_name = "test_lib"
"#,
    );

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let result = crate::scaffold::languages::scaffold_ruby_cargo(&api, &config);
    assert!(result.is_ok(), "scaffold_ruby_cargo should succeed");

    let files = result.unwrap();
    let cargo_toml_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Should generate Cargo.toml");

    let content = &cargo_toml_file.content;

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
