use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn native_rust_target(ruby_arch: &str) -> Option<&'static str> {
    match ruby_arch {
        "x64-mingw-ucrt" | "x64-mingw32" | "x86_64-mingw32" => Some("x86_64-pc-windows-gnu"),
        _ => None,
    }
}

pub(crate) fn ruby_native_target(ruby_binary: &Path) -> Option<&'static str> {
    let config = Command::new(ruby_binary)
        .args(["-rrbconfig", "-e", "print RbConfig::CONFIG.fetch('arch')"])
        .output()
        .expect("read the selected Ruby's architecture");
    assert!(config.status.success(), "{}", String::from_utf8_lossy(&config.stderr));
    native_rust_target(String::from_utf8(config.stdout).unwrap().trim())
}

pub(crate) fn configure_build_target(command: &mut Command, target: Option<&str>) {
    command.env_remove("CARGO_BUILD_TARGET");
    if let Some(target) = target {
        command.env("CARGO_BUILD_TARGET", target);
    }
}

pub(crate) fn artifact_directory(target_directory: &Path, target: Option<&str>) -> PathBuf {
    let mut artifact = target_directory.to_path_buf();
    if let Some(target) = target {
        artifact.push(target);
    }
    artifact.join("debug")
}

#[test]
fn should_match_rubyinstaller_abi_instead_of_the_msvc_host_target() {
    for arch in ["x64-mingw-ucrt", "x64-mingw32", "x86_64-mingw32"] {
        assert_eq!(native_rust_target(arch), Some("x86_64-pc-windows-gnu"), "{arch}");
    }
}

#[test]
fn should_keep_the_native_target_for_non_mingw_ruby() {
    for arch in ["arm64-darwin27", "x86_64-linux", "x64-mswin64_140"] {
        assert_eq!(native_rust_target(arch), None, "{arch}");
    }
}

#[test]
fn should_replace_an_inherited_cargo_target_with_the_selected_ruby_abi() {
    let mut command = Command::new("fixture");
    command.env("CARGO_BUILD_TARGET", "x86_64-pc-windows-msvc");
    configure_build_target(&mut command, native_rust_target("x64-mingw-ucrt"));
    let target = command.get_envs().find(|(key, _)| *key == "CARGO_BUILD_TARGET");
    assert_eq!(
        target,
        Some(("CARGO_BUILD_TARGET".as_ref(), Some("x86_64-pc-windows-gnu".as_ref())))
    );
}

#[test]
fn should_clear_an_inherited_cargo_target_for_native_ruby() {
    let mut command = Command::new("fixture");
    command.env("CARGO_BUILD_TARGET", "x86_64-pc-windows-msvc");
    configure_build_target(&mut command, native_rust_target("arm64-darwin27"));
    assert_eq!(
        command.get_envs().find(|(key, _)| *key == "CARGO_BUILD_TARGET"),
        Some(("CARGO_BUILD_TARGET".as_ref(), None))
    );
}

#[test]
fn should_read_the_explicit_target_artifact_instead_of_the_host_artifact() {
    let root = Path::new("fixture/target");
    assert_eq!(
        artifact_directory(root, Some("x86_64-pc-windows-gnu")),
        root.join("x86_64-pc-windows-gnu/debug")
    );
    assert_eq!(artifact_directory(root, None), root.join("debug"));
}
