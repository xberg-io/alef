use super::*;

// ---------------------------------------------------------------------------
// Dual-form core-facade dependency (`{ version = "X.Y.Z", path = "..." }`).
// to a registry version-dependency. The version equals the workspace version
// (here `api.version` == "0.1.0"), the path is preserved unchanged, features

/// Locate the binding-crate `Cargo.toml` generated for `lang` and return its
/// content. Filters out the Ruby `[lib]` Cargo (which lives under `native/`)
/// by matching the dependency-bearing manifest containing `[dependencies]`.
fn core_cargo_toml_for(lang: Language) -> String {
    let mut config = test_config();
    config.features = vec!["full".to_string(), "ocr".to_string()];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[lang]).unwrap();
    let files = language_files(&all_files);
    files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml") && f.content.contains("my-lib = {"))
        .map(|f| f.content.clone())
        .unwrap_or_else(|| panic!("no core Cargo.toml with `my-lib` dep emitted for {lang:?}"))
}

#[test]
fn render_core_dep_emits_dual_form_with_version_first() {
    let line = render_core_dep("my-lib", "../my-lib", "", "1.2.3");
    assert_eq!(line, r#"my-lib = { version = "1.2.3", path = "../my-lib" }"#);
}

#[test]
fn render_core_dep_preserves_features_suffix() {
    let line = render_core_dep("my-lib", "../my-lib", ", features = [\"full\", \"ocr\"]", "1.2.3");
    assert_eq!(
        line,
        r#"my-lib = { version = "1.2.3", path = "../my-lib", features = ["full", "ocr"] }"#
    );
}

#[test]
fn render_core_dep_falls_back_to_path_only_when_version_empty() {
    let line = render_core_dep("my-lib", "../my-lib", "", "");
    assert_eq!(line, r#"my-lib = { path = "../my-lib" }"#);
}

#[test]
fn test_scaffold_python_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Python);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "python core dep must be dual form with version + path + features; content:\n{content}"
    );
    assert!(
        content.contains(r#"serde_json = "1""#),
        "external serde_json unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_node_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Node);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "node core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains(r#"serde = { version = "1", features = ["derive"] }"#),
        "external serde unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_ruby_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Ruby);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "ruby core dep must be dual form with the deep crates path preserved; content:\n{content}"
    );
    assert!(
        content.contains("magnus = "),
        "external magnus unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_php_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Php);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "php core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains("ext-php-rs = "),
        "external ext-php-rs unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_elixir_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Elixir);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "elixir core dep must be dual form with the deep crates path preserved; content:\n{content}"
    );
    assert!(
        content.contains("rustler = "),
        "external rustler unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_r_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::R);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "r core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains("extendr-api = "),
        "external extendr-api unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_swift_core_dep_is_dual_form() {
    let config = test_config();
    let api = test_api();
    let files = crate::backends::swift::gen_rust_crate::emit(&api, &config).unwrap();
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("swift Cargo.toml must be emitted");
    assert!(
        cargo
            .content
            .contains(r#"my_lib = { version = "0.1.0", path = "../../..", package = "my-lib" }"#),
        "swift core dep must be dual form (version + path) with package rename; content:\n{}",
        cargo.content
    );
    assert!(
        cargo.content.contains(r#"serde_json = "1""#),
        "external serde_json unchanged; content:\n{}",
        cargo.content
    );
}

#[test]
fn test_scaffold_dev_path_build_form_preserved() {
    for lang in [
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Elixir,
        Language::R,
    ] {
        let content = core_cargo_toml_for(lang);
        let dep_line = content
            .lines()
            .find(|l| l.trim_start().starts_with("my-lib = {"))
            .unwrap_or_else(|| panic!("no my-lib dep line for {lang:?}"));
        assert!(
            dep_line.contains("path = "),
            "{lang:?}: dev-path-build path must be preserved: {dep_line}"
        );
        assert!(
            dep_line.contains(r#"version = "0.1.0""#),
            "{lang:?}: version must be injected: {dep_line}"
        );
    }
}

// dependency moves out of `[dependencies]` into a `cfg(not(...))` default block
// plus one `[target.'cfg(<cfg>)'.dependencies]` block per override.

#[test]
fn render_core_dep_with_overrides_no_overrides_matches_plain() {
    let (line, blocks) = render_core_dep_with_overrides("my-lib", "../my-lib", ", features = [\"full\"]", "1.2.3", &[]);
    assert_eq!(
        line,
        r#"my-lib = { version = "1.2.3", path = "../my-lib", features = ["full"] }"#
    );
    assert!(blocks.is_empty(), "no overrides must produce no target blocks");
}

#[test]
fn render_core_dep_with_overrides_emits_default_and_override_blocks() {
    let overrides = vec![crate::core::config::FfiTargetDepOverride {
        cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
        features: vec!["macos-intel-target".to_string()],
    }];
    let (line, blocks) =
        render_core_dep_with_overrides("my-lib", "../my-lib", ", features = [\"full\"]", "1.2.3", &overrides);
    assert!(line.is_empty(), "with overrides the core dep moves into target blocks");
    assert!(
        blocks.contains(r#"[target.'cfg(not(all(target_os = "macos", target_arch = "x86_64")))'.dependencies]"#),
        "default block gated on the negated cfg:\n{blocks}"
    );
    assert!(
        blocks.contains(r#"features = ["full"]"#),
        "default block keeps the base features:\n{blocks}"
    );
    assert!(
        blocks.contains(r#"[target.'cfg(all(target_os = "macos", target_arch = "x86_64"))'.dependencies]"#),
        "override block gated on the cfg:\n{blocks}"
    );
    assert!(
        blocks.contains(r#"features = ["macos-intel-target"]"#),
        "override block uses the override features:\n{blocks}"
    );
}

#[test]
fn scripting_backends_emit_target_dep_override_blocks() {
    let override_for = |lang: &str| {
        format!(
            "\n[[crates.{lang}.target_dep_overrides]]\ncfg = 'all(target_os = \"macos\", target_arch = \"x86_64\")'\nfeatures = [\"macos-intel-target\"]\n"
        )
    };
    let overrides: String = ["python", "node", "ruby", "php", "elixir"]
        .iter()
        .map(|l| override_for(l))
        .collect();
    let toml = format!(
        r#"
[workspace]
languages = ["python", "node", "ruby", "php", "elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
features = ["full"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Alice"]
keywords = ["test"]
{overrides}"#
    );
    let cfg: crate::core::config::new_config::NewAlefConfig =
        toml::from_str(&toml).expect("override config must parse");
    let config = cfg.resolve().expect("override config must resolve").remove(0);
    let api = test_api();

    for lang in [
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Elixir,
    ] {
        let all_files = scaffold(&api, &config, &[lang]).unwrap();
        let content = language_files(&all_files)
            .iter()
            .find(|f| f.path.ends_with("Cargo.toml") && f.content.contains("[target.'cfg"))
            .map(|f| f.content.clone())
            .unwrap_or_else(|| panic!("no target-gated Cargo.toml emitted for {lang:?}"));

        assert!(
            content.contains(r#"[target.'cfg(not(all(target_os = "macos", target_arch = "x86_64")))'.dependencies]"#),
            "{lang:?} must gate the default core dep:\n{content}"
        );
        assert!(
            content.contains(r#"[target.'cfg(all(target_os = "macos", target_arch = "x86_64"))'.dependencies]"#),
            "{lang:?} must emit the override block:\n{content}"
        );
        assert!(
            content.contains(r#"features = ["macos-intel-target"]"#),
            "{lang:?} override must use the override features:\n{content}"
        );
    }
}
