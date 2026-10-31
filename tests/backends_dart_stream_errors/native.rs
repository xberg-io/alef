use super::{CORE, bridge};
use alef::snippets::validators::{CapturedStreams, run_command_streams};
use std::path::Path;

const BUILD_TIMEOUT_SECONDS: u64 = 600;
const RUNTIME_TIMEOUT_SECONDS: u64 = 60;

fn task(root: &Path, name: &str, arguments: &[&str]) -> CapturedStreams {
    let mut command = super::mise_task::mise_run(root, name);
    command.args(arguments);
    let timeout = if name == "test" {
        RUNTIME_TIMEOUT_SECONDS
    } else {
        BUILD_TIMEOUT_SECONDS
    };
    run_command_streams(&mut command, timeout).unwrap()
}

fn successful(root: &Path, name: &str, arguments: &[&str]) -> CapturedStreams {
    let output = task(root, name, arguments);
    assert!(output.success, "{root:?} {name}:\n{}\n{}", output.stdout, output.stderr);
    output
}

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("core/src")).unwrap();
    std::fs::create_dir_all(root.join("rust/src")).unwrap();
    for (name, contents) in [
        ("core/src/lib.rs", CORE),
        (
            "core/Cargo.toml",
            include_str!("../fixtures/dart_stream_errors/core_Cargo.toml"),
        ),
        (
            "rust/Cargo.toml",
            include_str!("../fixtures/dart_stream_errors/bridge_Cargo.toml"),
        ),
        ("mise.toml", include_str!("../fixtures/dart_stream_errors/mise.toml")),
        (
            "pubspec.yaml",
            include_str!("../fixtures/dart_stream_errors/pubspec.yaml"),
        ),
        ("check.dart", include_str!("../fixtures/dart_stream_errors/check.dart")),
    ] {
        std::fs::write(root.join(name), contents).unwrap();
    }
}

/// Requires FRB codegen, Dart, and native Rust dependencies; executes the real FFI channel. ~keep
#[test]
#[ignore = "requires flutter_rust_bridge_codegen and Dart native build toolchains"]
fn typed_stream_errors_survive_the_actual_frb_dart_runtime() {
    let directory = tempfile::Builder::new()
        .prefix("alef-dart-stream-errors-")
        .tempdir()
        .unwrap();
    let root = directory.path().to_path_buf();
    let _cleanup = if std::env::var_os("ALEF_DART_STREAM_PROBE_KEEP").is_some() {
        let _ = directory.keep();
        None
    } else {
        Some(directory)
    };
    let path = root.as_path();
    write_fixture(path);
    let source = bridge(path, true);
    let native = path.join("rust/target/debug").join(format!(
        "{}stream_sample_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let old_source = super::bridge_for_error(path, true, "external::OtherError");
    assert_ne!(old_source, source);
    negative_control(path, &old_source, native.to_str().unwrap());
    std::fs::write(path.join("rust/src/lib.rs"), source).unwrap();
    successful(path, "generate", &[]);
    successful(path, "build", &[]);
    let output = successful(path, "test", &["--", native.to_str().unwrap()]);
    assert!(
        output.stdout.contains("stream_cases=12"),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
}

fn negative_control(path: &Path, old_source: &str, native: &str) {
    std::fs::write(path.join("rust/src/lib.rs"), old_source).unwrap();
    successful(path, "setup", &[]);
    successful(path, "generate", &[]);
    successful(path, "build", &[]);
    let old = task(path, "test", &["--", native]);
    assert!(
        !old.success,
        "the string-only stream codec must fail typed-error assertions"
    );
    assert!(
        old.stderr.contains("Bad state: expected typed Authentication:"),
        "{}\n{}",
        old.stdout,
        old.stderr
    );
}

pub(super) fn assert_disabled_transport_compiles(root: &Path, source: &str) {
    use quote::ToTokens;
    let parsed = syn::parse_file(source).unwrap();
    let wire = parsed
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "AlefStreamClientRecordsWire" => Some(item),
            _ => None,
        })
        .expect("generated records wire enum");
    std::fs::write(
        root.join("mise.toml"),
        include_str!("../fixtures/dart_stream_errors/mise.toml"),
    )
    .unwrap();
    std::fs::write(root.join("cfg_probe.rs"), wire.to_token_stream().to_string()).unwrap();
    successful(root, "compile-cfg", &[]);
    let mut unguarded = wire;
    unguarded.attrs.retain(|attr| !attr.path().is_ident("cfg"));
    std::fs::write(root.join("cfg_probe.rs"), unguarded.to_token_stream().to_string()).unwrap();
    let negative = task(root, "compile-cfg", &[]);
    assert!(
        !negative.success,
        "the unguarded wire must reference the unavailable item"
    );
    assert!(
        negative.stderr.contains("cannot find type `Chunk`"),
        "{}",
        negative.stderr
    );
}
