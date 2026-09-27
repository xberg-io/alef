use alef::backends::ffi::FfiBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;

const SENTINEL: [u8; 3] = [0xff, 0xfe, 0xfd];

fn generated_build_script() -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
header_name = "mylib.h"

[crates.go]
module = "github.com/example/mylib"

[crates.output]
ffi = "crates/mylib-ffi/src/"
go = "packages/go/"
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        ..ApiSurface::default()
    };
    FfiBackend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("build.rs"))
        .unwrap()
        .content
}

#[test]
fn generated_build_script_gates_transactional_utf8_checked_exports() {
    let generated = generated_build_script();
    let gate = generated.find("ALEF_EXPORT_GENERATED_HEADERS").unwrap();
    let generation = generated.find("cbindgen::generate").unwrap();
    assert!(gate < generation);
    assert!(generated.contains("if export_generated_headers_requested()"));
    assert!(generated.contains("cargo:rerun-if-changed=src"));
    assert!(generated.contains("cargo:rerun-if-changed=cbindgen.toml"));
    assert!(generated.contains("cargo:rerun-if-env-changed={ALEF_EXPORT_GENERATED_HEADERS}"));
    assert!(generated.contains("String::from_utf8"));
    assert!(generated.contains("atomic_write"));
    assert!(!generated.contains("std::fs::copy"));
    syn::parse_file(&generated).expect("generated build.rs must be valid Rust");
}

#[test]
fn generated_build_script_preserves_headers_until_valid_explicit_export() {
    let generated = generated_build_script();
    let fixture = tempfile::tempdir().unwrap();
    let ffi_dir = fixture.path().join("crates/mylib-ffi");
    let canonical = ffi_dir.join("include/mylib.h");
    let go_header = fixture.path().join("packages/go/include/mylib.h");
    std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    std::fs::create_dir_all(go_header.parent().unwrap()).unwrap();
    std::fs::write(&canonical, SENTINEL).unwrap();
    std::fs::write(&go_header, SENTINEL).unwrap();

    let source = fixture.path().join("build-script.rs");
    let executable = fixture
        .path()
        .join(format!("build-script{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&source, format!("{}\n{generated}", cbindgen_stub())).unwrap();
    let compile = std::process::Command::new("rustc")
        .args(["--edition", "2024"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "generated build.rs failed to compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let ordinary = run_build_script(&executable, &ffi_dir, &[]);
    assert!(ordinary.status.success());
    assert_headers(&canonical, &go_header, &SENTINEL);

    let invalid = run_build_script(
        &executable,
        &ffi_dir,
        &[("ALEF_EXPORT_GENERATED_HEADERS", "1"), ("STUB_INVALID_UTF8", "1")],
    );
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("cbindgen generated invalid UTF-8"));
    assert_headers(&canonical, &go_header, &SENTINEL);

    std::fs::remove_file(&go_header).unwrap();
    std::fs::create_dir(&go_header).unwrap();
    let blocked_destination = run_build_script(&executable, &ffi_dir, &[("ALEF_EXPORT_GENERATED_HEADERS", "1")]);
    assert!(!blocked_destination.status.success());
    assert_eq!(std::fs::read(&canonical).unwrap(), SENTINEL);
    std::fs::remove_dir(&go_header).unwrap();
    std::fs::write(&go_header, SENTINEL).unwrap();

    let explicit = run_build_script(
        &executable,
        &ffi_dir,
        &[("ALEF_EXPORT_GENERATED_HEADERS", "1"), ("CARGO_FEATURE_DEMO", "1")],
    );
    assert!(
        explicit.status.success(),
        "explicit export failed: {}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    let exported = std::fs::read(&canonical).unwrap();
    assert_ne!(exported, SENTINEL);
    assert_headers(&canonical, &go_header, &exported);
}

fn run_build_script(
    executable: &std::path::Path,
    ffi_dir: &std::path::Path,
    environment: &[(&str, &str)],
) -> std::process::Output {
    let mut command = std::process::Command::new(executable);
    command.current_dir(ffi_dir).env("CARGO_MANIFEST_DIR", ffi_dir);
    command.envs(environment.iter().copied());
    command.output().unwrap()
}

fn assert_headers(canonical: &std::path::Path, go_header: &std::path::Path, expected: &[u8]) {
    assert_eq!(std::fs::read(canonical).unwrap(), expected);
    assert_eq!(std::fs::read(go_header).unwrap(), expected);
}

fn cbindgen_stub() -> &'static str {
    r##"
mod cbindgen {
    use std::io::{self, Write};

    pub struct Bindings;

    impl Bindings {
        pub fn write<W: Write>(&self, mut output: W) {
            let bytes: &[u8] = if std::env::var_os("STUB_INVALID_UTF8").is_some() {
                &[0xff, 0xfe]
            } else {
                b"#ifndef ML_H\n#define ML_H\n#if defined(ML_FEATURE_DEMO)\n#endif\n"
            };
            output.write_all(bytes).unwrap();
        }
    }

    pub fn generate(_: &str) -> io::Result<Bindings> {
        Ok(Bindings)
    }
}
"##
}
