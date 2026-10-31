use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, c::CCodegen, go::GoCodegen, rust::RustE2eCodegen};
use alef::e2e::fixture::{Fixture, FixtureGroup};

#[path = "support/mise_task.rs"]
mod mise_task;

fn generate_mode(generator: &dyn E2eCodegen, extension: &str, streaming: bool) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust", "go", "c"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "sample::chat_stream"
owner_type = "Client"
request_type = "ChatRequest"
item_type = "StreamChunk"
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat_stream"
module = "sample"
returns_result = true
async = true
[crates.e2e.call.streaming]
enabled = true
item_type = "StreamChunk"
[crates.e2e.call.overrides.c]
client_factory = "client_new"
"#,
    )
    .expect("fixture config");
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id":"late_stream_failure", "call":"chat_stream", "input":{},
        "mock_response":{"status":200,"stream_chunks":[{"delta":"one"},{"delta":"two"},{"error":{"message":"limited"}}]},
        "assertions":[{"type":"error", "value":"limited"}]
    })).expect("fixture");
    let mut e2e = config.crates[0].e2e.clone().unwrap();
    let mut fixture = fixture;
    if !streaming {
        fixture.mock_response = None;
        e2e.call.streaming = None;
    }
    let resolved = config.resolve().unwrap().remove(0);
    let files = generator
        .generate(
            &[FixtureGroup {
                category: "streaming".into(),
                fixtures: vec![fixture],
            }],
            &e2e,
            &resolved,
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
    files
        .into_iter()
        .find(|f| f.path.to_string_lossy().ends_with(extension))
        .unwrap_or_else(|| panic!("missing {extension}"))
        .content
}

fn generate(generator: &dyn E2eCodegen, extension: &str) -> String {
    generate_mode(generator, extension, true)
}

#[test]
fn rust_should_check_the_error_from_consuming_the_stream() {
    let source = generate(&RustE2eCodegen, "streaming_test.rs");
    assert!(source.contains("tokio_stream::StreamExt::next"), "{source}");
    assert!(source.contains("Some(Err(error)) => break Err(error)"), "{source}");
    assert!(source.contains("None => break Ok(())"), "{source}");
    assert!(source.contains("result.is_err()"), "{source}");
}

#[test]
fn go_should_drain_then_assert_the_terminal_error() {
    let source = generate(&GoCodegen, "streaming_test.go");
    assert!(source.contains("for range stream.Chan()"), "{source}");
    assert!(source.contains("err = stream.Err()"), "{source}");
    assert!(
        source.find("err = stream.Err()").unwrap() < source.rfind("if err == nil").unwrap(),
        "{source}"
    );
}

#[test]
fn c_should_observe_the_iterator_error_before_freeing_handles() {
    let source = generate(&CCodegen, "test_streaming.c");
    assert!(
        source.contains("sample_client_chat_stream_next(stream_handle)"),
        "{source}"
    );
    assert!(source.contains("assert(sample_last_error_code() != 0"), "{source}");
    assert!(!source.contains("expected stream-start to fail"), "{source}");
    assert!(
        source.find("assert(sample_last_error_code() != 0").unwrap()
            < source.find("sample_client_chat_stream_free(stream_handle)").unwrap(),
        "{source}"
    );
}

#[test]
fn nonstream_error_paths_should_still_assert_the_call_error() {
    let rust = generate_mode(&RustE2eCodegen, "streaming_test.rs", false);
    assert!(
        rust.contains("result.is_err()") && !rust.contains("StreamExt::next"),
        "{rust}"
    );
    let go = generate_mode(&GoCodegen, "streaming_test.go", false);
    assert!(go.contains("if err == nil") && !go.contains("stream.Chan()"), "{go}");
    let c = generate_mode(&CCodegen, "test_streaming.c", false);
    assert!(
        c.contains("expected call to fail") && !c.contains("chat_stream_next(stream_handle)"),
        "{c}"
    );
}

fn run_task(directory: &std::path::Path, command: &str) {
    run_task_with_tools(directory, command, &[]);
}

fn run_task_with_tools(directory: &std::path::Path, command: &str, tools: &[&str]) {
    let config = format!("[tasks.probe]\nrun = {}\n", toml::Value::String(command.into()));
    let config = if tools.is_empty() {
        config
    } else {
        mise_task::fixture_config_with_tools(&config, tools)
    };
    std::fs::write(directory.join("mise.toml"), config).unwrap();
    let output = mise_task::mise_run(directory, "probe")
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_BUILD_JOBS", "2")
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .env_remove("CARGO_BUILD_TARGET")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn assert_controls(mut command: impl FnMut() -> std::process::Command) {
    let mut count = 0;
    for (mode, success) in [("immediate", true), ("late", true), ("wrong", false), ("clean", false)] {
        let output = command()
            .env("STREAM_MODE", mode)
            .env("MOCK_SERVER_URL", "http://127.0.0.1")
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !success {
            let evidence = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                evidence.contains("error message mismatch")
                    || evidence.contains("expected")
                    || evidence.contains("error does not contain"),
                "{mode}: {evidence}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn rust_generated_error_body_should_accept_only_actual_expected_stream_failures() {
    let source = generate(&RustE2eCodegen, "streaming_test.rs");
    let start = source.find("    let result = chat_stream(").expect("rendered call");
    let end = source[start..].find("    });").unwrap() + start;
    let body = &source[start..end];
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("src/main.rs"),
        include_str!("fixtures/stream_error_runtime/rust.rs").replace("GENERATED_BODY", body),
    )
    .unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), "[package]\nname='stream-error-runtime'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ntokio-stream='0.1'\n[workspace]\n").unwrap();
    run_task(directory.path(), "cargo build --quiet");
    let executable = directory.path().join(format!(
        "target/debug/stream-error-runtime{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert_controls(|| std::process::Command::new(&executable));
}

#[test]
fn go_generated_test_should_accept_only_actual_expected_stream_failures() {
    let source = generate(&GoCodegen, "streaming_test.go");
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("go.mod"), "module sample\n\ngo 1.22\n").unwrap();
    std::fs::write(
        directory.path().join("sample.go"),
        include_str!("fixtures/stream_error_runtime/go.go"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("stream_test.go"),
        source.replace("package e2e_test", "package sample_test"),
    )
    .unwrap();
    let filename = format!("probe{}", std::env::consts::EXE_SUFFIX);
    run_task_with_tools(directory.path(), &format!("go test -c -o {filename}"), &["go"]);
    let executable = directory.path().join(filename);
    assert_controls(|| std::process::Command::new(&executable));
}

#[test]
#[ignore = "requires a native C compiler"]
fn c_generated_test_should_accept_only_actual_expected_stream_failures_and_free_handles() {
    let source = generate(&CCodegen, "test_streaming.c");
    let body = &source[source.find("void test_late_stream_failure(").unwrap()..];
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("main.c"),
        include_str!("fixtures/stream_error_runtime/c.c").replace("GENERATED_BODY", body),
    )
    .unwrap();
    let filename = format!("probe{}", std::env::consts::EXE_SUFFIX);
    run_task(directory.path(), &format!("cc main.c -o {filename}"));
    let executable = directory.path().join(filename);
    assert_controls(|| std::process::Command::new(&executable));
}

fn standalone_rust_files(returns_result: bool, async_call: bool) -> Vec<alef::core::backend::GeneratedFile> {
    use alef::core::config::ResolvedCrateConfig;
    use alef::e2e::config::{CallConfig, E2eConfig, StreamingConfig};
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..Default::default()
    };
    let e2e = E2eConfig {
        call: CallConfig {
            function: "chat_stream".into(),
            module: "sample".into(),
            returns_result,
            r#async: async_call,
            streaming: Some(StreamingConfig::Enabled(true)),
            ..Default::default()
        },
        ..Default::default()
    };
    let fixture: Fixture = serde_json::from_value(
        serde_json::json!({"id":"late_stream_failure", "assertions":[{"type":"error","value":"limited"}]}),
    )
    .unwrap();
    RustE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "streaming".into(),
                fixtures: vec![fixture],
            }],
            &e2e,
            &config,
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap()
}

fn assert_standalone_rust_runtime(returns_result: bool, async_call: bool) {
    let files = standalone_rust_files(returns_result, async_call);
    let emitted = |suffix: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(suffix))
            .map(|file| file.content.clone())
    };
    let source = emitted("streaming_test.rs").expect("generated test file");
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("tests")).unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("src/lib.rs"),
        if async_call {
            include_str!("fixtures/stream_error_runtime/sync.rs")
                .replace("pub fn chat_stream", "pub async fn chat_stream")
        } else {
            include_str!("fixtures/stream_error_runtime/sync.rs").to_string()
        },
    )
    .unwrap();
    std::fs::write(directory.path().join("tests/streaming_test.rs"), &source).unwrap();
    std::fs::write(
        directory.path().join("tests/common.rs"),
        emitted("tests/common.rs").expect("generated runtime module"),
    )
    .unwrap();
    let feature = if returns_result { "" } else { "default=['infallible']\n" };
    std::fs::write(directory.path().join("Cargo.toml"), format!("[package]\nname='sample'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ntokio-stream='0.1'\ntokio={{version='1',features=['rt-multi-thread','time','net']}}\n[features]\n{feature}infallible=[]\n[workspace]\n")).unwrap();
    run_task(directory.path(), "cargo test --quiet --test streaming_test --no-run");
    assert!(source.contains("common::runtime().block_on(async"), "{source}");
    assert_eq!(source.contains("chat_stream().await"), async_call, "{source}");
    assert!(emitted("tests/common.rs").is_some(), "runtime module must be generated");
    let manifest: toml::Value = toml::from_str(&emitted("rust/Cargo.toml").expect("generated manifest")).unwrap();
    assert!(manifest["dependencies"].get("tokio").is_some());
    let executables: Vec<_> = std::fs::read_dir(directory.path().join("target/debug/deps"))
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("streaming_test-")
                && path.extension().is_none_or(|ext| ext == "exe")
        })
        .collect();
    assert_eq!(executables.len(), 1);
    assert_controls(|| std::process::Command::new(&executables[0]));
}

#[test]
fn synchronous_fallible_rust_stream_factory_should_run_error_iteration_in_a_runtime() {
    assert_standalone_rust_runtime(true, false);
}

#[test]
fn synchronous_infallible_rust_stream_factory_should_run_error_iteration_in_a_runtime() {
    assert_standalone_rust_runtime(false, false);
}

#[test]
fn asynchronous_rust_stream_factories_without_mock_should_compile_the_actual_runtime_module() {
    for returns_result in [true, false] {
        assert_standalone_rust_runtime(returns_result, true);
    }
}
