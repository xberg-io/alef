use crate::backends::magnus::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn generated_optional_string(is_async: bool) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let api = ApiSurface {
        crate_name: "sample_core".to_string(),
        functions: vec![FunctionDef {
            name: "echo_optional".to_string(),
            rust_path: "sample_core::echo_optional".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            is_async,
            ..Default::default()
        }],
        ..Default::default()
    };
    MagnusBackend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content
}

fn assert_optional_string_errors_propagate(is_async: bool) {
    let source = generated_optional_string(is_async);
    syn::parse_file(&source).expect("generated bindings must parse");
    assert!(source.contains("Option<magnus::Value>"), "must exercise scan_args");
    assert!(
        source.contains("String::try_convert"),
        "must exercise string conversion"
    );
    assert!(
        source.contains("Some(String::try_convert(v)?)"),
        "wrong-type optional strings must propagate Magnus conversion errors:\n{source}"
    );
    assert!(!source.contains("String::try_convert(v).unwrap_or_default()"));
}

#[test]
fn should_propagate_wrong_type_optional_string_in_sync_wrapper() {
    assert_optional_string_errors_propagate(false);
}

#[test]
fn should_propagate_wrong_type_optional_string_in_async_wrappers() {
    assert_optional_string_errors_propagate(true);
}

fn write_native_fixture(directory: &std::path::Path) {
    let source = generated_optional_string(false);
    let conversions: Vec<_> = source
        .lines()
        .filter(|line| line.contains("let value: Option<String>"))
        .collect();
    assert_eq!(conversions.len(), 1, "compile the actual emitted conversion");
    let wrapper = include_str!("../../../tests/fixtures/magnus_optional_string/wrapper.rs.txt");
    std::fs::create_dir(directory.join("src")).unwrap();
    std::fs::write(
        directory.join("src/lib.rs"),
        wrapper.replace("CONVERSION", conversions[0]),
    )
    .unwrap();
    std::fs::write(
        directory.join("Cargo.toml"),
        include_str!("../../../tests/fixtures/magnus_optional_string/Cargo.toml.txt"),
    )
    .unwrap();
    std::fs::write(
        directory.join("check.rb"),
        include_str!("../../../tests/fixtures/magnus_optional_string/check.rb"),
    )
    .unwrap();
    std::fs::write(
        directory.join("Taskfile.yml"),
        "version: '3'\ntasks:\n  build:\n    cmds: ['cargo build']\n",
    )
    .unwrap();
    if cfg!(target_os = "macos") {
        std::fs::write(
            directory.join("build.rs"),
            "fn main() { println!(\"cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup\"); }",
        )
        .unwrap();
    }
}

fn ruby_native_target(ruby_binary: &std::path::Path) -> Option<&'static str> {
    let config = std::process::Command::new(ruby_binary)
        .args(["-rrbconfig", "-e", "print RbConfig::CONFIG.fetch('arch')"])
        .output()
        .unwrap();
    assert!(config.status.success(), "{}", String::from_utf8_lossy(&config.stderr));
    native_rust_target(String::from_utf8(config.stdout).unwrap().trim())
}

fn build_native_fixture(directory: &std::path::Path, ruby_binary: &std::path::Path, target: Option<&str>) {
    let mut command = std::process::Command::new("task");
    command
        .arg("build")
        .env("RUBY", ruby_binary)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .env_remove("CARGO_BUILD_TARGET")
        .current_dir(directory);
    // ~keep RubyInstaller uses MinGW even when Alef's test executable was compiled with MSVC.
    if let Some(target) = target {
        command.env("CARGO_BUILD_TARGET", target);
    }
    let build = command.output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
}

#[test]
fn should_raise_type_error_in_compiled_ruby_extension() {
    let Some(ruby_binary) = crate::test_support::toolchain::RUBY.open() else {
        return;
    };
    let fixture = tempfile::tempdir().unwrap();
    write_native_fixture(fixture.path());
    let target = ruby_native_target(&ruby_binary);
    build_native_fixture(fixture.path(), &ruby_binary, target);
    let filename = if cfg!(target_os = "macos") {
        "liboptional_string_fixture.dylib"
    } else if cfg!(target_os = "windows") {
        "optional_string_fixture.dll"
    } else {
        "liboptional_string_fixture.so"
    };
    let extension_name = if cfg!(target_os = "macos") {
        "optional_string_fixture.bundle"
    } else {
        "optional_string_fixture.so"
    };
    let extension = fixture.path().join(extension_name);
    std::fs::copy(
        native_artifact_directory(fixture.path(), target).join(filename),
        &extension,
    )
    .unwrap();
    let ruby = std::process::Command::new(ruby_binary)
        .arg(fixture.path().join("check.rb"))
        .arg(extension)
        .output()
        .unwrap();
    assert!(ruby.status.success(), "{}", String::from_utf8_lossy(&ruby.stderr));
    assert_eq!(
        String::from_utf8(ruby.stdout).unwrap().trim(),
        "PASS: 4 valid controls, 4 TypeError cases"
    );
}

fn native_rust_target(ruby_arch: &str) -> Option<&'static str> {
    match ruby_arch {
        "x64-mingw-ucrt" | "x64-mingw32" | "x86_64-mingw32" => Some("x86_64-pc-windows-gnu"),
        _ => None,
    }
}

fn native_artifact_directory(directory: &std::path::Path, target: Option<&str>) -> std::path::PathBuf {
    let mut artifact = directory.join("target");
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
fn should_read_the_explicit_target_artifact_instead_of_the_host_artifact() {
    let root = std::path::Path::new("fixture");
    assert_eq!(
        native_artifact_directory(root, Some("x86_64-pc-windows-gnu")),
        root.join("target/x86_64-pc-windows-gnu/debug")
    );
    assert_eq!(native_artifact_directory(root, None), root.join("target/debug"));
}
