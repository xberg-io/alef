use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{AdapterConfig, ResolvedCrateConfig};
use alef::snippets::validators::{CapturedStreams, run_command_streams};
use quote::ToTokens;
use std::path::Path;

#[path = "support/dart_sdk.rs"]
mod dart_sdk;
#[path = "support/mise_task.rs"]
mod mise_task;

const CORE: &str = include_str!("fixtures/dart_stream_usize/core.rs");

fn generate(root: &Path) -> String {
    let input = root.join("lib.rs");
    std::fs::write(&input, CORE).unwrap();
    let mut api = alef::extract::extractor::extract(&[&input], "stream-sample", "1.0.0", None).unwrap();
    let client = api.types.iter_mut().find(|ty| ty.name == "Client").unwrap();
    client.is_opaque = true;
    client.is_clone = true;
    assert_eq!(client.methods.len(), 8);
    for method in &mut client.methods {
        method.sanitized = true;
    }
    let mut config = ResolvedCrateConfig {
        name: "stream-sample".into(),
        ..Default::default()
    };
    for (name, item) in [
        ("usize_scalar", "usize"),
        ("usize_record", "UsizeRecord"),
        ("usize_optional", "Option<usize>"),
        ("usize_vector", "Vec<usize>"),
        ("usize_nested", "Vec<Option<usize>>"),
        ("usize_map", "std::collections::HashMap<usize, u64>"),
        ("u64_scalar", "u64"),
        ("u64_vector", "Vec<u64>"),
    ] {
        let adapter: AdapterConfig = serde_json::from_value(serde_json::json!({
            "name": name, "pattern": "streaming", "core_path": name,
            "owner_type": "Client", "item_type": item, "error_type": "StreamError",
        }))
        .unwrap();
        config.adapters.push(adapter);
    }
    DartBackend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content
}

#[test]
fn stream_wire_payloads_use_the_public_usize_representation() {
    let directory = tempfile::tempdir().unwrap();
    let source = generate(directory.path());
    let parsed = syn::parse_file(&source).unwrap();
    let expected = [
        ("AlefStreamClientUsizeScalarWire", "i64"),
        ("AlefStreamClientUsizeRecordWire", "UsizeRecord"),
        ("AlefStreamClientUsizeOptionalWire", "Option<i64>"),
        ("AlefStreamClientUsizeVectorWire", "Vec<i64>"),
        ("AlefStreamClientUsizeNestedWire", "Vec<Option<i64>>"),
        ("AlefStreamClientUsizeMapWire", "std::collections::HashMap<i64,u64>"),
        ("AlefStreamClientU64ScalarWire", "u64"),
        ("AlefStreamClientU64VectorWire", "Vec<u64>"),
    ];
    assert_eq!(expected.len(), 8);
    for (name, payload) in expected {
        let wire = parsed.items.iter().find_map(|item| match item {
            syn::Item::Enum(wire) if wire.ident == name => Some(wire),
            _ => None,
        });
        let wire = wire.expect(name);
        let item = wire.variants.iter().find(|variant| variant.ident == "Item").unwrap();
        let value = item
            .fields
            .iter()
            .find(|field| field.ident.as_ref().unwrap() == "value")
            .unwrap();
        let actual = value.ty.to_token_stream().to_string().replace(' ', "");
        assert_eq!(actual, format!("::std::boxed::Box<{payload}>"), "{name}");
    }
}

#[test]
fn named_stream_payload_reuses_the_public_usize_mirror() {
    let directory = tempfile::tempdir().unwrap();
    let source = generate(directory.path());
    let parsed = syn::parse_file(&source).unwrap();
    let record = parsed.items.iter().find_map(|item| match item {
        syn::Item::Struct(record) if record.ident == "UsizeRecord" => Some(record),
        _ => None,
    });
    let record = record.expect("public FRB mirror for UsizeRecord");
    let count = record
        .fields
        .iter()
        .find(|field| field.ident.as_ref().unwrap() == "count")
        .unwrap();
    assert_eq!(count.ty.to_token_stream().to_string(), "i64");
    assert!(source.contains("let value: UsizeRecord = chunk.into();"));
}

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("rust/src")).unwrap();
    for (name, content) in [
        ("core/src/lib.rs", CORE),
        (
            "core/Cargo.toml",
            include_str!("fixtures/dart_stream_errors/core_Cargo.toml"),
        ),
        (
            "rust/Cargo.toml",
            include_str!("fixtures/dart_stream_errors/bridge_Cargo.toml"),
        ),
        ("mise.toml", include_str!("fixtures/dart_stream_errors/mise.toml")),
        ("mise.lock", include_str!("fixtures/dart_stream_errors/mise.lock")),
        ("pubspec.yaml", include_str!("fixtures/dart_stream_errors/pubspec.yaml")),
        ("check.dart", include_str!("fixtures/dart_stream_usize/check.dart")),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

fn successful(root: &Path, name: &str, arguments: &[&str]) -> CapturedStreams {
    let mut command = mise_task::mise_run(root, name);
    command
        .args(arguments)
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_TARGET_DIR", root.join("rust/target"));
    dart_sdk::forward_machine_caches(&mut command);
    let output = run_command_streams(&mut command, 600).unwrap();
    assert!(output.success, "{name}: {}\n{}", output.stdout, output.stderr);
    output
}

#[test]
#[ignore = "requires pinned FRB 2.13, Dart 3.13.7 and native Rust toolchains"]
#[allow(clippy::print_stderr, reason = "report the retained native regression fixture path")]
fn usize_stream_payloads_compile_and_run_through_actual_frb() {
    let directory = tempfile::Builder::new()
        .prefix("alef-dart-stream-usize-")
        .tempdir()
        .unwrap();
    let root = directory.path().to_path_buf();
    let _cleanup = if std::env::var_os("ALEF_DART_STREAM_USIZE_KEEP").is_some() {
        let _ = directory.keep();
        None
    } else {
        Some(directory)
    };
    eprintln!("usize_stream_fixture_root={}", root.display());
    write_fixture(&root);
    let sdk = successful(&root, "sdk", &[]);
    dart_sdk::assert_pinned_sdk(&sdk, "usize_stream");
    let source = generate(&root);
    std::fs::write(root.join("rust/src/lib.rs"), source).unwrap();
    successful(&root, "setup", &[]);
    successful(&root, "generate", &[]);
    successful(&root, "build", &[]);
    let native = root.join("rust/target/debug").join(format!(
        "{}stream_sample_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(native.is_file() && native.metadata().unwrap().len() > 0);
    let output = successful(&root, "test", &["--", native.to_str().unwrap()]);
    assert_eq!(output.stdout.matches("usize_stream_cases=8").count(), 1);
}
