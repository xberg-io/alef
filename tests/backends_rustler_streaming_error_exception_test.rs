//! Regression tests for BLK-14 — verifies the rustler streaming-adapter emitter
//! raises a typed `StreamError` exception on mid-stream NIF errors instead of
//! silently returning `nil` (the EOS sentinel).

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;

fn make_config_with_streaming(app_name: &str) -> ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"

[[crates.adapters]]
name = "test_stream"
pattern = "streaming"
core_path = "core::test_stream"
owner_type = "TestClient"
item_type = "TestChunk"

[[crates.adapters.params]]
name = "request"
type = "String"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_no_streaming(app_name: &str) -> ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn empty_api(crate_name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: crate_name.to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

fn find_module<'a>(files: &'a [alef::core::backend::GeneratedFile], app_name: &str) -> &'a str {
    let expected = format!("{app_name}.ex");
    files
        .iter()
        .find(|f| f.path.file_name().map(|n| n.to_string_lossy().to_string()).as_deref() == Some(expected.as_str()))
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| {
            let paths: Vec<_> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
            panic!("expected {expected} in generated files, got: {paths:?}");
        })
}

#[test]
fn streaming_emits_typed_stream_error_exception_module() {
    let config = make_config_with_streaming("test_streaming");
    let api = empty_api("test-streaming");
    let files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let content = find_module(&files, "test_streaming");

    assert!(
        content.contains("defmodule TestStreaming.StreamError do"),
        "StreamError exception module must be defined; content:\n{content}"
    );
    assert!(
        content.contains("defexception [:message, :reason, :adapter]"),
        "StreamError must declare :message, :reason, :adapter fields; content:\n{content}"
    );
    assert!(
        content.contains("def message(%__MODULE__{message: msg}), do: msg"),
        "StreamError must implement message/1; content:\n{content}"
    );
}

#[test]
fn streaming_wrapper_raises_on_mid_stream_error() {
    let config = make_config_with_streaming("test_stream_errors");
    let api = empty_api("test-stream-errors");
    let files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let content = find_module(&files, "test_stream_errors");

    assert!(
        content.contains("raise TestStreamErrors.StreamError"),
        "unfold closure must raise the typed StreamError exception; content:\n{content}"
    );
    assert!(
        content.contains("{:error, reason} ->"),
        "error arm must capture reason; content:\n{content}"
    );
    assert!(
        content.contains("reason: reason"),
        "exception must receive the reason field; content:\n{content}"
    );
    assert!(
        content.contains("adapter: :test_stream"),
        "exception must receive the adapter name as :test_stream; content:\n{content}"
    );
    assert!(
        !content.contains("{:error, _} ->\n                nil"),
        "the previous nil-swallow arm must be gone; content:\n{content}"
    );
}

#[test]
fn no_stream_error_exception_when_no_streaming_adapters() {
    let config = make_config_no_streaming("test_no_stream");
    let api = empty_api("test-no-stream");
    let files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let content = find_module(&files, "test_no_stream");

    assert!(
        !content.contains("defmodule TestNoStream.StreamError"),
        "StreamError must NOT be emitted when no streaming adapters are configured; content:\n{content}"
    );
}
