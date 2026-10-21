//! A stream error reaches the C ABI with the same message text as a call that is not a stream.
//! Binding generators choose a typed error from the start of that text, so a prefix with the name
//! of the native function hides the error kind from them.

use super::common::resolved_one;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, TypeDef};

fn generated_lib() -> String {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatChunk"
error_type = "MyError"
request_type = "my_lib::ChatRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatRequest"
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "my_lib::DefaultClient".to_string(),
            is_opaque: true,
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let files = crate::backends::ffi::FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    syn::parse_file(&lib.content).expect("generated FFI crate must parse as Rust");
    lib.content.clone()
}

#[test]
fn a_mid_stream_error_is_captured_with_the_core_error_message() {
    let source = generated_lib();

    assert!(
        source.contains("e.capture(&e.message);"),
        "the stream error must keep the core message unchanged:\n{source}"
    );
    assert!(
        !source.contains("stream error: {}"),
        "the stream error message must not carry a native function name:\n{source}"
    );
}

#[test]
fn a_stream_open_error_is_captured_with_the_core_error_message() {
    let source = generated_lib();

    assert!(
        source.contains("set_last_error_from(&e, &e.to_string());"),
        "the open error must keep the core message unchanged:\n{source}"
    );
    assert!(
        !source.contains("failed to open stream"),
        "the open error message must not carry a native function name:\n{source}"
    );
}
