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
    // `test_config()` is root-flat (`sources = ["src/lib.rs"]`), so from
    // `crates/my-lib-py` the core crate's `Cargo.toml` at the project root is `../..`, not
    // the single-`..` workspace-sibling path a fixed nesting depth would assume. ~keep
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../..", features = ["full", "ocr"] }"#),
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
    // See `test_scaffold_python_core_dep_is_dual_form`: `test_config()` is root-flat. ~keep
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../..", features = ["full", "ocr"] }"#),
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
    // See `test_scaffold_python_core_dep_is_dual_form`: `test_config()` is root-flat. ~keep
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../..", features = ["full", "ocr"] }"#),
        "php core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains("ext-php-rs = \"0.16.1\""),
        "generated PHP crate must use ext-php-rs 0.16.1; content:\n{content}"
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
        default_features: true,
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
    assert!(
        !blocks.contains("default-features"),
        "default_features: true must not emit a default-features key:\n{blocks}"
    );
}

/// Regression test for the dropped `default_features` config key: an override with
/// `default_features = false` must emit `default-features = false` in its target block so
/// consumers can drop the core dep's `default = [...]` set (e.g. `tokio-runtime`,
/// `simd-utf8`) on a target that cannot support it, while an override that leaves
/// `default_features` at its default (`false`, matching `DartTargetDepOverride` /
/// `SwiftTargetDepOverride`) behaves identically — both are the restrictive case.
#[test]
fn render_core_dep_with_overrides_emits_default_features_false_when_override_disables_it() {
    let overrides = vec![crate::core::config::FfiTargetDepOverride {
        cfg: "target_os = \"windows\"".to_string(),
        features: vec!["windows-target".to_string()],
        default_features: false,
    }];
    let (line, blocks) =
        render_core_dep_with_overrides("my-lib", "../my-lib", ", features = [\"full\"]", "1.2.3", &overrides);
    assert!(line.is_empty(), "with overrides the core dep moves into target blocks");
    assert!(
        blocks.contains(r#"[target.'cfg(target_os = "windows")'.dependencies]"#),
        "expected the windows override block:\n{blocks}"
    );
    assert!(
        blocks.contains("default-features = false"),
        "default_features: false must emit default-features = false:\n{blocks}"
    );
    assert!(
        blocks.contains(r#"features = ["windows-target"]"#),
        "override block must still emit its feature list alongside default-features = false:\n{blocks}"
    );
}

/// End-to-end regression for `packages/elixir/native/xberg_nif/Cargo.toml`: xberg's
/// intent was to drop the core dep's own `default = ["tokio-runtime", "simd-utf8"]` on
/// Windows via `[crates.elixir.target_dep_overrides]` with `default_features = false`, but
/// `FfiTargetDepOverride` (shared by `[crates.ffi]` and `[crates.elixir]`) previously had
/// no `default_features` field, so `deny_unknown_fields` rejected the key outright.
#[test]
fn elixir_target_dep_override_with_default_features_false_drops_default_features() {
    let config = test_config_from_toml(
        r#"
[[crates.elixir.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]
default_features = false
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let content = language_files(&all_files)
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml") && f.content.contains("[target.'cfg"))
        .map(|f| f.content.clone())
        .expect("target-gated elixir Cargo.toml emitted");

    assert!(
        content.contains(r#"[target.'cfg(target_os = "windows")'.dependencies]"#),
        "expected the windows override block:\n{content}"
    );
    assert!(
        content.contains("default-features = false"),
        "elixir override must emit default-features = false to drop the core dep's own defaults:\n{content}"
    );
    assert!(
        content.contains(r#"features = ["windows-target"]"#),
        "elixir override must still request windows-target:\n{content}"
    );
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders
/// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
/// predicate string (plain byte-wise comparison), NOT with the default
/// `cfg(not(any(...)))` branch always first. With multiple overrides, an
/// `all(...)`-prefixed override sorts *before* the `not(any(...))` default
/// branch (`'a'` < `'n'`), while a `target_os = ...` override sorts after it
/// (`'n'` < `'t'`). `render_core_dep_with_overrides` backs every scripting
/// binding (python/node/ruby/php/elixir), so this test covers all of them at
/// once.
#[test]
fn render_core_dep_with_overrides_sorts_all_before_not_before_target_os() {
    let overrides = vec![
        crate::core::config::FfiTargetDepOverride {
            cfg: "target_os = \"android\"".to_string(),
            features: vec!["android-target".to_string()],
            default_features: true,
        },
        crate::core::config::FfiTargetDepOverride {
            cfg: "target_os = \"windows\"".to_string(),
            features: vec!["windows-target".to_string()],
            default_features: true,
        },
        crate::core::config::FfiTargetDepOverride {
            cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
            features: vec!["macos-intel-target".to_string()],
            default_features: true,
        },
    ];
    let (line, blocks) =
        render_core_dep_with_overrides("my-lib", "../my-lib", ", features = [\"full\"]", "1.2.3", &overrides);
    assert!(line.is_empty(), "with overrides the core dep moves into target blocks");

    let all_pos = blocks
        .find("[target.'cfg(all(target_os = \"macos\", target_arch = \"x86_64\"))'.dependencies]")
        .expect("expected the macOS-Intel `all(...)` override block");
    let not_pos = blocks
        .find("[target.'cfg(not(any(")
        .expect("expected the default `not(any(...))` block");
    let android_pos = blocks
        .find("[target.'cfg(target_os = \"android\")'.dependencies]")
        .expect("expected the android override block");
    let windows_pos = blocks
        .find("[target.'cfg(target_os = \"windows\")'.dependencies]")
        .expect("expected the windows override block");

    assert!(
        all_pos < not_pos,
        "the `all(...)` override must sort BEFORE the `not(...)` default branch; got:\n{blocks}"
    );
    assert!(
        not_pos < android_pos,
        "the `not(...)` default branch must sort before `target_os = \"android\"`; got:\n{blocks}"
    );
    assert!(
        android_pos < windows_pos,
        "`target_os = \"android\"` must sort before `target_os = \"windows\"`; got:\n{blocks}"
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

/// Regression test for the FFI/Python/Node/PHP scaffolders hard-coding the core-crate
/// dependency as `path = "../{core_crate_dir}"`, which only resolves when the core crate
/// is a workspace-shaped sibling (`crates/{core}` beside `crates/{core}-<lang>`) and is
/// simply wrong for a root-flat core crate (`Cargo.toml` at the project root -- the shape
/// alef itself has used since 0.18.0), pointing at a `crates/<name>` directory that does
/// not exist.
///
/// This does not compare the emitted string against a second hard-coded string -- two
/// independently-typed literals that happen to agree is exactly the failure mode this
/// codebase keeps producing. Instead it lays down real files matching each layout on disk
/// and canonicalizes the emitted path against them, the same resolution `cargo build`
/// itself would perform, and asserts it lands on the file the fixture actually put there.
#[test]
fn scaffold_core_dep_path_resolves_to_the_real_core_crate_manifest_for_both_layouts() {
    let layouts: &[(&str, Vec<PathBuf>, PathBuf)] = &[
        ("root-flat", vec![PathBuf::from("src/lib.rs")], PathBuf::new()),
        (
            "workspace",
            vec![PathBuf::from("crates/my-lib/src/lib.rs")],
            PathBuf::from("crates/my-lib"),
        ),
    ];

    for (label, sources, core_crate_root) in layouts {
        for lang in [Language::Ffi, Language::Python, Language::Node, Language::Php] {
            let mut config = test_config();
            config.sources = sources.clone();
            let api = test_api();
            let all_files = scaffold(&api, &config, &[lang]).unwrap();
            let manifest = language_files(&all_files)
                .into_iter()
                .find(|f| f.path.ends_with("Cargo.toml") && f.content.contains("my-lib = {"))
                .unwrap_or_else(|| panic!("{label}/{lang:?}: no core-dependency Cargo.toml emitted"));
            let binding_root = manifest.path.parent().expect("manifest path has a parent directory");

            let dep_path = manifest
                .content
                .lines()
                .find_map(|line| {
                    let trimmed = line.trim_start();
                    if !trimmed.starts_with("my-lib = {") {
                        return None;
                    }
                    trimmed.split("path = \"").nth(1)?.split('"').next()
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{label}/{lang:?}: no `my-lib` path dependency in:\n{}",
                        manifest.content
                    )
                });

            let dir = tempfile::tempdir().expect("tempdir");
            let project_root = dir.path();
            let core_dir = project_root.join(core_crate_root);
            std::fs::create_dir_all(&core_dir).expect("create core crate dir");
            std::fs::write(core_dir.join("Cargo.toml"), "[package]\nname = \"my-lib\"\n")
                .expect("write core Cargo.toml");
            let binding_dir = project_root.join(binding_root);
            std::fs::create_dir_all(&binding_dir).expect("create binding crate dir");

            let resolved = binding_dir.join(dep_path).canonicalize().unwrap_or_else(|error| {
                panic!(
                    "{label}/{lang:?}: emitted path `{dep_path}` from `{}` does not resolve: {error}",
                    binding_root.display()
                )
            });
            let expected = core_dir
                .canonicalize()
                .expect("canonicalize the fixture's own core crate dir");
            assert_eq!(
                resolved,
                expected,
                "{label}/{lang:?}: emitted path `{dep_path}` from `{}` must resolve to the core crate's own directory",
                binding_root.display()
            );
        }
    }
}
