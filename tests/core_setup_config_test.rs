//! Integration test: per-language default setup workdir.
//!
//! Locks in the `SetupConfig::workdir` defaults so `alef setup` runs install
//! commands from each binding's manifest directory for languages whose project
//! file does not live at the repo root (Swift, Kotlin-Android, Dart, Zig).

use std::path::PathBuf;

use alef::core::config::extras::Language;
use alef::core::config::setup_defaults::setup_config_for_language;

#[test]
fn swift_workdir_is_packages_swift() {
    let cfg = setup_config_for_language(Language::Swift);
    assert_eq!(cfg.workdir, Some(PathBuf::from("packages/swift")));
}

#[test]
fn kotlin_android_workdir_is_packages_kotlin_android() {
    let cfg = setup_config_for_language(Language::KotlinAndroid);
    assert_eq!(cfg.workdir, Some(PathBuf::from("packages/kotlin-android")));
}

#[test]
fn dart_workdir_is_packages_dart() {
    let cfg = setup_config_for_language(Language::Dart);
    assert_eq!(cfg.workdir, Some(PathBuf::from("packages/dart")));
}

#[test]
fn zig_workdir_is_packages_zig() {
    let cfg = setup_config_for_language(Language::Zig);
    assert_eq!(cfg.workdir, Some(PathBuf::from("packages/zig")));
}

#[test]
fn python_has_no_workdir_default() {
    let cfg = setup_config_for_language(Language::Python);
    assert_eq!(cfg.workdir, None);
}

#[test]
fn languages_without_manifest_outside_root_have_no_workdir() {
    for lang in [
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Go,
        Language::Java,
        Language::Csharp,
        Language::Elixir,
        Language::Wasm,
        Language::Ffi,
        Language::Rust,
    ] {
        let cfg = setup_config_for_language(lang);
        assert_eq!(cfg.workdir, None, "{lang} should have no default workdir");
    }
}
