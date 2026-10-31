use super::generated;
use alef::snippets::validators::{CapturedStreams, run_command_streams};
use std::path::Path;

#[path = "../support/dart_sdk.rs"]
mod dart_sdk;

fn task(root: &Path, name: &str, arguments: &[&str]) -> CapturedStreams {
    let mut command = super::mise_task::mise_run(root, name);
    command.args(arguments);
    run_command_streams(&mut command, 600).unwrap()
}

fn successful(root: &Path, name: &str, arguments: &[&str]) -> CapturedStreams {
    let output = task(root, name, arguments);
    assert!(output.success, "{name}:\n{}\n{}", output.stdout, output.stderr);
    output
}

fn write_fixture(root: &Path) {
    for (name, contents) in [
        ("core/src/lib.rs", super::CORE),
        (
            "core/Cargo.toml",
            include_str!("../fixtures/dart_u64_bigint/core_Cargo.toml"),
        ),
        (
            "rust/Cargo.toml",
            include_str!("../fixtures/dart_u64_bigint/bridge_Cargo.toml"),
        ),
        ("mise.toml", include_str!("../fixtures/dart_u64_bigint/mise.toml")),
        ("mise.lock", include_str!("../fixtures/dart_u64_bigint/mise.lock")),
        ("pubspec.yaml", include_str!("../fixtures/dart_u64_bigint/pubspec.yaml")),
        ("check.dart", include_str!("../fixtures/dart_u64_bigint/check.dart")),
    ] {
        let destination = root.join(name);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, contents).unwrap();
    }
    for file in generated(root) {
        let relative = file.path.strip_prefix("packages/dart").unwrap();
        if relative == Path::new("rust/src/lib.rs") || relative.starts_with("lib") {
            let destination = root.join(relative);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, file.content).unwrap();
        }
    }
}

/// Executes FRB 2.13's real native codec, including its unsigned BigInt list implementation. ~keep
#[test]
#[ignore = "requires pinned FRB codegen, cached Dart packages and native Rust toolchains"]
fn dart_u64_boundaries_survive_the_actual_native_codec_in_both_directions() {
    let directory = tempfile::Builder::new().prefix("alef-dart-u64-").tempdir().unwrap();
    let root = directory.path().to_path_buf();
    let _cleanup = if std::env::var_os("ALEF_DART_U64_PROBE_KEEP").is_some() {
        let _ = directory.keep();
        None
    } else {
        Some(directory)
    };
    write_fixture(&root);
    let sdk = successful(&root, "sdk", &[]);
    dart_sdk::assert_pinned_sdk(&sdk, "unsigned");
    successful(&root, "setup", &[]);
    successful(&root, "generate", &[]);
    successful(&root, "build", &[]);
    let native = root.join("rust/target/debug").join(format!(
        "{}wide_sample_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX,
    ));
    let native = native.to_str().unwrap();
    let output = successful(&root, "test", &["--", native]);
    assert!(output.stdout.contains("u64_cases=121"), "{}", output.stdout);
    let negative = task(&root, "test", &["--", native, "wrong-oracle"]);
    assert!(!negative.success, "a poisoned unsigned boundary oracle must fail");
    assert!(
        negative.stderr.contains("unsigned boundary mismatch"),
        "{}\n{}",
        negative.stdout,
        negative.stderr
    );
}
