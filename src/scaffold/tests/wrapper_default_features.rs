use super::*;
use crate::core::config::languages::RubyConfig;
use crate::core::ir::TypeDef;

fn config_with_core_manifest(features: &[&str]) -> (tempfile::TempDir, ResolvedCrateConfig) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = test_config();
    config.workspace_root = Some(dir.path().to_path_buf());
    config.sources = vec![PathBuf::from("crates/my-lib/src/lib.rs")];
    let crate_dir = dir.path().join("crates/my-lib");
    std::fs::create_dir_all(&crate_dir).expect("core crate directory");
    let feature_rows = features
        .iter()
        .map(|feature| format!("{feature} = []"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!("[package]\nname = \"my-lib\"\nversion = \"0.1.0\"\n\n[features]\n{feature_rows}\n"),
    )
    .expect("core Cargo.toml");
    (dir, config)
}

#[test]
fn native_wrappers_preserve_configured_defaults_without_restoring_hidden_api() {
    let (_dir, mut config) = config_with_core_manifest(&["mcp-http", "hidden-api"]);
    config.languages = vec![
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Elixir,
        Language::Ffi,
    ];
    config.wrapper_default_features = vec!["mcp-http".to_string()];
    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "RustOnlyFormulaOptions".to_string(),
            rust_path: "my_lib::RustOnlyFormulaOptions".to_string(),
            cfg: Some(r#"feature = "hidden-api""#.to_string()),
            binding_excluded: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = scaffold(&api, &config, &config.languages).expect("native wrapper scaffold");
    let cargo_manifests: Vec<_> = files
        .iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|file| {
            let path = crate::test_support::portable_path_string(&file.path);
            path.contains("-py/")
                || path.contains("-node/")
                || path.contains("-rb/")
                || path.contains("-php/")
                || path.contains("/native/")
                || path.contains("-ffi/")
        })
        .collect();
    assert_eq!(
        cargo_manifests.len(),
        6,
        "all six native wrapper manifests must be emitted"
    );

    for manifest in cargo_manifests {
        assert!(
            manifest.content.contains(r#"mcp-http = ["my-lib/mcp-http"]"#),
            "{} must forward the configured wrapper default:\n{}",
            manifest.path.display(),
            manifest.content
        );
        let default_line = manifest
            .content
            .lines()
            .find(|line| line.starts_with("default = ["))
            .expect("wrapper manifest default feature array");
        assert!(
            default_line.contains(r#""mcp-http""#),
            "{} must enable the configured wrapper default: {default_line}",
            manifest.path.display()
        );
        assert!(
            !manifest.content.contains("hidden-api"),
            "{} must not restore a projected-out API feature:\n{}",
            manifest.path.display(),
            manifest.content
        );
    }

    assert!(
        files
            .iter()
            .filter(|file| file.path.file_name().is_none_or(|name| name != "Cargo.toml"))
            .all(|file| !file.content.contains("RustOnlyFormulaOptions")),
        "the manifest-only feature declaration must not restore a binding-excluded Rust API"
    );
}

#[test]
fn fresh_scaffold_rejects_wrapper_default_excluded_by_a_native_language() {
    let (_dir, mut config) = config_with_core_manifest(&["formula-recognition"]);
    config.languages = vec![Language::Ruby];
    config.wrapper_default_features = vec!["formula-recognition".to_string()];
    config.ruby = Some(
        toml::from_str::<RubyConfig>("gem_name = \"my_lib\"\nexcluded_default_features = [\"formula-recognition\"]\n")
            .expect("Ruby config"),
    );

    let error = scaffold(&ApiSurface::default(), &config, &config.languages)
        .expect_err("conflicting wrapper defaults must fail before manifest emission");

    assert!(
        error.to_string().contains(
            "wrapper_default_features feature `formula-recognition` conflicts with \
             `[crates.ruby].excluded_default_features`"
        ),
        "the error must identify both conflicting settings: {error:#}"
    );
}

#[test]
fn fresh_scaffold_rejects_invalid_or_unknown_wrapper_default_features() {
    for (feature, expected) in [
        ("", "must not be empty"),
        ("dep:escape", "may only contain ASCII letters"),
        ("evil\"]\nforged = [\"x", "may only contain ASCII letters"),
        (
            "unknown-feature",
            "is not declared in the core crate's `[features]` table",
        ),
    ] {
        let (_dir, mut config) = config_with_core_manifest(&["formula-recognition"]);
        config.languages = vec![Language::Python];
        config.wrapper_default_features = vec![feature.to_string()];

        let error = scaffold(&ApiSurface::default(), &config, &config.languages)
            .expect_err("an invalid wrapper default must fail before manifest emission");
        assert!(
            error.to_string().contains(expected),
            "`{feature}` must fail with `{expected}`, got: {error:#}"
        );
    }
}

#[test]
fn fresh_scaffold_rejects_reserved_wrapper_default_features() {
    for feature in ["default", "extension-module"] {
        let (_dir, mut config) = config_with_core_manifest(&[feature]);
        config.languages = vec![Language::Python];
        config.wrapper_default_features = vec![feature.to_string()];

        let error = scaffold(&ApiSurface::default(), &config, &config.languages)
            .expect_err("reserved wrapper defaults must fail before manifest emission");
        assert!(
            error
                .to_string()
                .contains("is reserved by generated native wrapper manifests"),
            "`{feature}` must be rejected as reserved, got: {error:#}"
        );
    }
}

#[test]
fn fresh_scaffold_ignores_exclusions_for_inactive_languages() {
    let (_dir, mut config) = config_with_core_manifest(&["formula-recognition"]);
    config.languages = vec![Language::Python];
    config.wrapper_default_features = vec!["formula-recognition".to_string()];
    config.ruby = Some(
        toml::from_str::<RubyConfig>("gem_name = \"my_lib\"\nexcluded_default_features = [\"formula-recognition\"]\n")
            .expect("Ruby config"),
    );

    let files = scaffold(&ApiSurface::default(), &config, &config.languages)
        .expect("an inactive Ruby exclusion must not invalidate Python scaffolding");
    let python_manifest = files
        .iter()
        .find(|file| crate::test_support::portable_path_string(&file.path).ends_with("my-lib-py/Cargo.toml"))
        .expect("Python Cargo.toml");
    assert!(
        python_manifest
            .content
            .contains(r#"formula-recognition = ["my-lib/formula-recognition"]"#),
        "Python must still forward the configured default:\n{}",
        python_manifest.content
    );
}
