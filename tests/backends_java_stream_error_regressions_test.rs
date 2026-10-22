//! Runtime proof for the generated Java stream iterator. The generated `chatStream` method runs
//! against a scripted stand-in for the native library, so the errors the native layer reports in
//! the middle of a stream reach the same code the shipped binding runs.

#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};
use support::{compile_java, extract_java_method, java_available, run_java_args, write_file};

const METHOD_SIGNATURE: &str = "public java.util.stream.Stream<ChatCompletionChunk> chatStream(";

fn generated_client() -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tl"

[crates.java]
package = "com.test"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"
error_type = "TestError"
request_type = "test_lib::ChatCompletionRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#,
    )
    .expect("valid streaming config");
    let config = config.resolve().expect("resolved streaming config").remove(0);
    let serde_type = |name: &str, is_return_type: bool| TypeDef {
        name: name.into(),
        rust_path: format!("test_lib::{name}"),
        is_clone: true,
        has_default: true,
        has_serde: true,
        is_return_type,
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "DefaultClient".into(),
                rust_path: "test_lib::DefaultClient".into(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "chat_stream".into(),
                    return_type: TypeRef::Unit,
                    is_async: true,
                    error_type: Some("TestError".into()),
                    receiver: Some(ReceiverKind::Ref),
                    ..Default::default()
                }],
                ..Default::default()
            },
            serde_type("ChatCompletionRequest", false),
            serde_type("ChatCompletionChunk", true),
        ],
        ..Default::default()
    };
    JavaBackend
        .generate_bindings(&api, &config)
        .expect("Java streaming generation")
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("DefaultClient.java"))
        .expect("generated DefaultClient.java")
        .content
}

fn run_scenario(scenario: &str) {
    if !java_available() {
        return;
    }
    let client = generated_client();
    let method = extract_java_method(&client, METHOD_SIGNATURE);
    assert!(
        method.contains("throws TestLibRsException"),
        "the harness stands in for TestLibRsException:\n{method}"
    );
    let directory = tempfile::tempdir().expect("temporary stream directory");
    write_file(
        directory.path(),
        "com/test/StreamHarness.java",
        &include_str!("fixtures/java_stream_error_harness.java").replace("GENERATED_METHOD", &method),
    );
    write_file(
        directory.path(),
        "com/test/NativeLib.java",
        include_str!("fixtures/java_stream_error_native_lib.java"),
    );
    write_file(
        directory.path(),
        "com/test/Types.java",
        include_str!("fixtures/java_stream_error_types.java"),
    );
    write_file(
        directory.path(),
        "com/test/StreamErrorMain.java",
        include_str!("fixtures/java_stream_error_main.java"),
    );
    compile_java(
        directory.path(),
        &[
            "com/test/Types.java",
            "com/test/NativeLib.java",
            "com/test/StreamHarness.java",
            "com/test/StreamErrorMain.java",
        ],
    );
    run_java_args(directory.path(), &["-cp", ".", "com.test.StreamErrorMain", scenario]);
}

#[test]
fn the_chunk_before_a_stream_error_is_returned_and_the_error_follows() {
    run_scenario("error_after_two_chunks");
}

#[test]
fn the_chunk_before_a_stream_error_is_returned_through_the_stream_api() {
    run_scenario("error_through_stream_api");
}

#[test]
fn a_stream_error_is_thrown_as_the_typed_binding_exception() {
    run_scenario("error_on_first_read");
}

#[test]
fn a_stream_rejected_on_open_is_thrown_as_the_typed_binding_exception() {
    run_scenario("stream_rejected_on_open");
}

#[test]
fn a_stream_that_ends_cleanly_delivers_every_chunk() {
    run_scenario("clean_end");
}
