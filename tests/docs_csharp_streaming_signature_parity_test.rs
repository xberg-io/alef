//! Golden/parity coverage for issue #446 (C# side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented C# streaming signature as a `format!` string instead of rendering
//! the real instance method the way `CsharpBackend::generate_bindings` does
//! (`gen_opaque_streaming_method`, `src/backends/csharp/gen_bindings/types/opaque.rs`, rendered
//! through `src/backends/csharp/templates/opaque_streaming_method.jinja`), so nothing forces the
//! two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `CsharpBackend::generate_bindings` (the real opaque-class emitter) and `alef::docs::generate_docs`
//! (the hand-built docs override) -- and compare the *shape* each one emits: the
//! `IAsyncEnumerable<T>` item type, the method name (including the `Async` suffix), and the
//! request parameter's type and name.
//!
//! Like the Go fixture and unlike the PHP/Java/Dart ones, the C# backend's real instance method
//! (`gen_opaque_streaming_method`) reads its request parameter from `MethodDef::params` -- see its
//! `method.params.iter().find(|p| matches!(&p.ty, TypeRef::Named(_)))` -- not from
//! `AdapterConfig::params`, so this fixture deliberately leaves `AdapterConfig::params` empty (as
//! `tests/e2e_csharp_opaque_streaming_wrapper.rs` already does).

use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.csharp]
namespace = "Test.Lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP parity tests.
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
        // The C# backend's real instance method reads its request param from `MethodDef`, not
        // from here -- see `gen_opaque_streaming_method`'s `method.params.iter().find(...)`. Left
        // empty to match `tests/e2e_csharp_opaque_streaming_wrapper.rs`'s existing convention;
        // only `owner_type`/`item_type` matter, to build the streaming-method lookup.
        params: Vec::new(),
        returns: Some("CrawlEvent".to_string()),
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
/// the comparison survives cosmetic differences (the real method's multi-line parameter list and
/// body vs the docs' single-line bare signature) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    item_type: String,
    method_name: String,
    param_type: String,
    param_name: String,
    has_enumerator_cancellation_attribute: bool,
}

fn extract_shape(source: &str, side: &str) -> StreamingShape {
    // The `[EnumeratorCancellation]` attribute is captured (not swallowed as an optional
    // non-capturing group) precisely because its presence/absence IS the shape divergence this
    // test exists to catch -- see the KNOWN DIVERGENCE note below.
    let re = regex::Regex::new(
        r"(?s)public async IAsyncEnumerable<(\w+)> (\w+)\(\s*(\w+) (\w+),\s*(\[EnumeratorCancellation\]\s*)?CancellationToken cancellationToken = default\)",
    )
    .unwrap();
    let caps = re
        .captures(source)
        .unwrap_or_else(|| panic!("no {side} streaming method signature found in:\n{source}"));
    StreamingShape {
        item_type: caps.get(1).unwrap().as_str().to_string(),
        method_name: caps.get(2).unwrap().as_str().to_string(),
        param_type: caps.get(3).unwrap().as_str().to_string(),
        param_name: caps.get(4).unwrap().as_str().to_string(),
        has_enumerator_cancellation_attribute: caps.get(5).is_some(),
    }
}

/// The parity guard proper: generate the real C# opaque class through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's `IAsyncEnumerable<T>` item type, method name (including the
/// `Async` suffix), or request parameter type/name that the other side does not also make.
///
/// Does NOT catch, deliberately: the real instance method marks its `CancellationToken` parameter
/// `[EnumeratorCancellation]` (required for `yield return` inside an `IAsyncEnumerable<T>` method
/// to observe cancellation -- see `System.Runtime.CompilerServices.EnumeratorCancellation`) while
/// the docs override (`streaming_method_signature_override`'s `Language::Csharp` arm,
/// `src/docs/language_pages/streaming.rs`) emits no such attribute. That attribute binds the
/// method's own iterator plumbing, not its call site: a caller passes a `CancellationToken` (or
/// uses `.WithCancellation(token)`) identically either way, so it is an implementer-side detail
/// rather than a caller-visible shape difference, and documenting it would add noise. It is
/// pinned below as a backend-only fact instead, so it cannot silently vanish from the emitted
/// code unnoticed.
#[test]
fn csharp_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = CsharpBackend
        .generate_bindings(&api, &config)
        .expect("csharp backend generates bindings");
    let engine_cs = &backend_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("Engine.cs"))
        .unwrap_or_else(|| {
            panic!(
                "Engine.cs is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_shape(engine_cs, "backend");

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Csharp], "docs/reference").expect("docs generate");
    let api_csharp_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-csharp.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-csharp.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_shape(api_csharp_md, "docs");

    assert_eq!(
        backend_shape.item_type, docs_shape.item_type,
        "IAsyncEnumerable<T> item type must match between backend and docs.\n\
         backend:\n{engine_cs}\ndocs:\n{api_csharp_md}"
    );
    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.param_type, docs_shape.param_type,
        "request parameter type must match between backend and docs"
    );
    assert_eq!(
        backend_shape.param_name, docs_shape.param_name,
        "request parameter name must match between backend and docs"
    );
    // Pin the real values too, not just cross-side equality -- so a change that moves both sides
    // in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.item_type, "CrawlEvent");
    assert_eq!(backend_shape.method_name, "CrawlStreamAsync");
    assert_eq!(backend_shape.param_type, "CrawlRequest");
    assert_eq!(backend_shape.param_name, "req");
    assert!(
        backend_shape.has_enumerator_cancellation_attribute,
        "the real method always marks its CancellationToken parameter [EnumeratorCancellation]"
    );
}
