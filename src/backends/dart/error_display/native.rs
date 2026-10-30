#![allow(clippy::print_stderr)]

use super::*;
use crate::backends::dart::DartBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use std::path::Path;

#[path = "native_pruning.rs"]
mod pruning;

fn task(root: &Path, name: &str, arguments: &[&str]) -> std::process::Output {
    let output = crate::test_support::mise_task::mise_run(root, name)
        .args(arguments)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("CARGO_TARGET_DIR", root.join("frb-target"))
        .output()
        .expect("execute Dart error display fixture task");
    std::fs::write(
        root.join(format!("proof-{name}.log")),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    output
}

fn successful(root: &Path, name: &str, arguments: &[&str]) -> std::process::Output {
    let output = task(root, name, arguments);
    assert!(
        output.status.success(),
        "{name}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn prepare(root: &Path, error_name: &str) -> ResolvedCrateConfig {
    let core = include_str!("../../../../tests/fixtures/dart_error_display/core.rs").replace("SampleError", error_name);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), core).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"display-sample\"\nversion = \"1.0.0\"\nedition = \"2024\"\n[dependencies]\nthiserror = \"2\"\n").unwrap();
    let api = crate::extract::extractor::extract(&[&root.join("src/lib.rs")], "display-sample", "1.0.0", None).unwrap();
    assert_eq!(api.errors.len(), 2);
    assert_eq!(api.errors[0].variants.len(), 7);
    assert_eq!(
        records(&api.errors.iter().collect::<Vec<_>>()).len(),
        6,
        "{:#?}",
        api.errors
    );
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"display-sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    assert!(
        files.iter().any(|file| file.content.contains("alef-error-display:")),
        "generated facade missing metadata"
    );
    let scaffolds = crate::scaffold::scaffold(&api, &config, &[crate::core::config::Language::Dart]).unwrap();
    for file in files.into_iter().chain(scaffolds) {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    std::fs::write(
        root.join("mise.toml"),
        include_str!("../../../../tests/fixtures/dart_error_display/mise.toml"),
    )
    .unwrap();
    std::fs::write(
        root.join("packages/dart/check.dart"),
        include_str!("../../../../tests/fixtures/dart_error_display/check.dart").replace("SampleError", error_name),
    )
    .unwrap();
    config
}

#[test]
#[ignore = "requires Dart, flutter_rust_bridge_codegen, Freezed, and a native Rust toolchain"]
fn native_error_display_survives_actual_frb_freezed_and_rust_errors() {
    verify_native_error_display("SampleError");
}

#[test]
#[ignore = "requires Dart, flutter_rust_bridge_codegen, Freezed, and a native Rust toolchain"]
fn native_reserved_error_name_survives_actual_frb_freezed_and_rust_errors() {
    verify_native_error_display("Error");
}

fn verify_native_error_display(error_name: &str) {
    let directory = tempfile::Builder::new().prefix("alef-dart-display-").tempdir().unwrap();
    let preserved = directory.keep();
    let root = preserved.as_path();
    eprintln!("dart_error_display_proof={}", root.display());
    let config = prepare(root, error_name);
    successful(root, "setup", &[]);
    pruning::verify(root, &config);
    successful(root, "generate", &[]);
    successful(root, "build", &[]);
    let binary = root.join("native-target/debug").join(format!(
        "{}display_sample_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(binary.metadata().unwrap().len() > 0);
    let library = binary.to_str().unwrap();
    rejected(root, "test", &["--", library]);
    repair(root, &config);
    successful(root, "analyze", &[]);
    let positive = successful(root, "test", &["--", library]);
    assert!(String::from_utf8_lossy(&positive.stdout).contains("error_display_cases=9"));
    rejected(root, "wrong-expectation", &["--", library]);
    std::fs::write(
        root.join("proof-summary.json"),
        r#"{"metadata_records":6,"runtime_cases":9,"negative_control":"exact Display mismatch","idempotent":true}"#,
    )
    .unwrap();
}

fn repair(root: &Path, config: &ResolvedCrateConfig) {
    let facade = root.join(crate::backends::dart::frb_rust_facade_paths(config).unwrap().0);
    let facade = std::fs::read_to_string(facade).unwrap();
    assert_eq!(read_records(&facade).unwrap().len(), 6);
    let bridge = root.join(crate::backends::dart::frb_dart_bridge_path(config).unwrap());
    let source = std::fs::read_to_string(&bridge).unwrap();
    let path = bridge.with_file_name("lib.freezed.dart");
    let before = std::fs::read_to_string(&path).unwrap();
    assert!(before.len() > 1000);
    let repaired = rewrite(&before, &source, &facade).unwrap();
    assert_ne!(repaired, before);
    assert_eq!(rewrite(&repaired, &source, &facade).unwrap(), repaired);
    assert_eq!(
        std::fs::read_to_string(&bridge).unwrap(),
        source,
        "factory constructors must remain byte-identical"
    );
    std::fs::write(path, repaired).unwrap();
}

fn rejected(root: &Path, name: &str, arguments: &[&str]) {
    let output = task(root, name, arguments);
    assert!(!output.status.success(), "incorrect Display assertion must fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Bad state: expected display 0:"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
