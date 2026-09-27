//! Golden/parity coverage for issue #446 (Zig side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Zig streaming method signature as a `format!` string instead of
//! rendering the real wrapper the way `ZigBackend::generate_bindings` does
//! (`opaque_stream_method.jinja`, rendered from
//! `src/backends/zig/gen_bindings/opaque_handles/streaming.rs::emit_opaque_streaming_method`),
//! so nothing forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `ZigBackend::generate_bindings` (the real emitted `.zig` module) and `alef::docs::generate_docs`
//! (the hand-built docs override) -- and compare the *shape* each one emits: the method name, the
//! receiver type, the request parameter, and the declared error-union return type.
//!
//! The fixture method declares no Rust `error_type` (`MethodDef::error_type: None`), unlike the
//! Go/PHP/Python fixtures' `Some("String")` placeholder: Zig's streaming return type is a real
//! function of the declared error, not a placeholder marker, and `Some("String")` (an error type
//! matching no declared `ErrorDef`) would push both sides through their respective "unmatched
//! error" fallback paths for reasons unrelated to the shape this test checks. `None` exercises
//! each side's ordinary "no declared error" fallback (`anyerror`) on both sides identically,
//! isolating the actual divergence pinned below.

use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/PHP/Python parity tests, minus a declared
/// Rust error type (see the module doc comment for why).
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
                    error_type: None,
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
        params: Vec::new(),
        returns: None,
        error_type: None,
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

/// The pieces of the documented/emitted streaming shape this test compares, extracted by regex
/// so the comparison survives cosmetic differences (the real method's body vs the docs' bare
/// signature line) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    receiver_type: String,
    req_param: String,
    error_union: String,
    stream_type: String,
}

fn extract_shape(text: &str) -> StreamingShape {
    let re = regex::Regex::new(r"pub fn (\w+)\(self: \*(\w+), (\w+): \[\]const u8\) \(([^)]+)\)!(\w+)").unwrap();
    let caps = re
        .captures(text)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{text}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        receiver_type: caps.get(2).unwrap().as_str().to_string(),
        req_param: caps.get(3).unwrap().as_str().to_string(),
        error_union: caps.get(4).unwrap().as_str().to_string(),
        stream_type: caps.get(5).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Zig module through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, receiver type, request parameter, error
/// union, or iterator struct name that the other side does not also make.
///
/// This test currently pins a REAL divergence rather than passing: the real emitted method
/// (`opaque_stream_method.jinja`) declares
/// `({{ zig_error_type }}||error{OutOfMemory,HandleClosed})!{{ struct_name }}`, but
/// `streaming_zig_return_type` (`src/docs/language_pages/streaming.rs`) hand-builds
/// `format!("({error_type}||error{{OutOfMemory}})!{stream_type}")` -- omitting `HandleClosed`
/// from the declared error set entirely. `HandleClosed` is not incidental: every opaque-handle
/// method (see `opaque_handles/instance_methods.rs::method_return_type`) declares it because the
/// emitted body's `if (handle == 0) return error.HandleClosed;` guard can actually produce it,
/// and the streaming start method's own body (`opaque_stream_method.jinja` line 3) carries the
/// identical guard.
#[test]
fn zig_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = ZigBackend
        .generate_bindings(&api, &config)
        .expect("zig backend generates bindings");
    let module_zig = &backend_files
        .iter()
        .find(|file| file.path.extension().is_some_and(|ext| ext == "zig"))
        .unwrap_or_else(|| {
            panic!(
                "a .zig module is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_shape(module_zig);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Zig], "docs/reference").expect("docs generate");
    let api_zig_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-zig.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-zig.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_shape(api_zig_md);

    // Pin the real backend value first, independent of the docs side.
    assert_eq!(backend_shape.method_name, "crawl_stream");
    assert_eq!(backend_shape.receiver_type, "Engine");
    assert_eq!(backend_shape.req_param, "req");
    assert_eq!(backend_shape.stream_type, "CrawlEventStream");
    assert_eq!(
        backend_shape.error_union, "anyerror||error{OutOfMemory,HandleClosed}",
        "the real emitted streaming method must declare HandleClosed alongside OutOfMemory"
    );

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{module_zig}\ndocs:\n{api_zig_md}"
    );
    assert_eq!(
        backend_shape.receiver_type, docs_shape.receiver_type,
        "receiver type must match between backend and docs"
    );
    assert_eq!(
        backend_shape.req_param, docs_shape.req_param,
        "request parameter name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.stream_type, docs_shape.stream_type,
        "iterator struct name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.error_union, docs_shape.error_union,
        "declared error union must match between backend and docs.\nbackend: {}\ndocs: {}",
        backend_shape.error_union, docs_shape.error_union
    );
}
