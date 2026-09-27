//! Golden/parity coverage for issue #446 (Swift side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Swift streaming signature as a `format!` string instead of
//! rendering the real `swift_streaming_client_method.swift.jinja` the way
//! `SwiftBackend::generate_bindings` does (`emit_streaming_client_method`,
//! `src/backends/swift/gen_bindings/client.rs`), so nothing forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `SwiftBackend::generate_bindings` (the real client-class emitter) and
//! `alef::docs::generate_docs` (the hand-built docs override) -- and compare the *shape* each one
//! emits: the method name, the request parameter's label and type, and the
//! `AsyncThrowingStream<T, Error>` item type. Nothing here invokes swift-bridge or a Swift
//! compiler: `generate_bindings` is pure source generation.
//!
//! The owner type must appear as a key in `[crates.swift] client_constructor_body`, or the Swift
//! backend classifies the adapter as ownerless and emits a top-level free function taking the
//! owner handle explicitly (`emit_streaming_free_functions`) instead of the client-class method a
//! caller of the documented API would see -- a structurally different shape that would make this
//! guard compare the wrong two things.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]
client_constructor_body.Engine = "Self { inner: ::test_lib::Engine::new() }"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go, Python and PHP parity tests.
fn streaming_api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Engine".to_string(),
                rust_path: "test_lib::Engine".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "crawl_stream".to_string(),
                    params: vec![ParamDef {
                        name: "req".to_string(),
                        ty: TypeRef::Named("CrawlRequest".to_string()),
                        ..ParamDef::default()
                    }],
                    return_type: TypeRef::Unit,
                    receiver: Some(ReceiverKind::Ref),
                    error_type: Some("String".to_string()),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "CrawlRequest".to_string(),
                rust_path: "test_lib::CrawlRequest".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "url".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "CrawlEvent".to_string(),
                rust_path: "test_lib::CrawlEvent".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "message".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

fn streaming_adapter() -> AdapterConfig {
    AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "test_lib::Engine::crawl_stream".to_string(),
        // The Swift backend's streaming client method reads its parameter list from here, not
        // from `MethodDef` -- the docs side reads `MethodDef::params` for the same parameter.
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "test_lib::CrawlRequest".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: Some("String".to_string()),
        owner_type: Some("Engine".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("test_lib::CrawlRequest".to_string()),
        skip_languages: Vec::new(),
    }
}

fn streaming_config() -> ResolvedCrateConfig {
    let mut resolved = resolved_config();
    resolved.adapters = vec![streaming_adapter()];
    resolved
}

/// The pieces of the documented/emitted streaming shape this test compares, extracted by regex so
/// the comparison survives cosmetic differences (the real method's body vs the docs' bare
/// signature line) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    param_label: String,
    param_type: String,
    item_type: String,
}

fn extract_shape(source: &str, side: &str) -> StreamingShape {
    let re =
        regex::Regex::new(r"public func (\w+)\(_ (\w+): (\w+)\) async throws -> AsyncThrowingStream<(\w+), Error>")
            .unwrap();
    let caps = re
        .captures(source)
        .unwrap_or_else(|| panic!("no {side} streaming method signature found in:\n{source}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        param_label: caps.get(2).unwrap().as_str().to_string(),
        param_type: caps.get(3).unwrap().as_str().to_string(),
        item_type: caps.get(4).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Swift client class through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, request parameter label or type, or
/// `AsyncThrowingStream` item type that the other side does not also make.
#[test]
fn swift_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = SwiftBackend
        .generate_bindings(&api, &config)
        .expect("swift backend generates bindings");
    let module_swift = &backend_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("TestLib.swift"))
        .unwrap_or_else(|| {
            panic!(
                "TestLib.swift is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_shape(module_swift, "backend");

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Swift], "docs/reference").expect("docs generate");
    let api_swift_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-swift.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-swift.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_shape(api_swift_md, "docs");

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\n\
         backend:\n{module_swift}\ndocs:\n{api_swift_md}"
    );
    assert_eq!(
        backend_shape.param_label, docs_shape.param_label,
        "request parameter label must match between backend and docs"
    );
    assert_eq!(
        backend_shape.param_type, docs_shape.param_type,
        "request parameter type must match between backend and docs"
    );
    assert_eq!(
        backend_shape.item_type, docs_shape.item_type,
        "AsyncThrowingStream item type must match between backend and docs"
    );

    // Pin the real values too, not just cross-side equality -- so a change that moves both sides
    // in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.param_label, "req");
    assert_eq!(backend_shape.param_type, "CrawlRequest");
    assert_eq!(backend_shape.item_type, "CrawlEvent");
}
