//! Golden/parity coverage for issue #446 (Java side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Java streaming signature as a `format!` string instead of
//! rendering the real `streaming_iterator_method.jinja` template the way
//! `gen_streaming_method` (`src/backends/java/gen_bindings/types/opaque/extended.rs`) does, so
//! nothing forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `JavaBackend::generate_bindings` (the real opaque-class emitter) and
//! `alef::docs::generate_docs` (the hand-built docs override) -- and compare the *shape* each
//! one emits: the `Stream<T>` item type, the method name, whether the request parameter is
//! declared `final`, the request parameter's type and name, and the `throws` exception class.
//!
//! `gen_streaming_method`/`streaming_method_symbols` are private to
//! `src/backends/java/gen_bindings/types/opaque/`, unreachable from `src/docs` -- a shared-helper
//! fix would need a visibility change under `src/backends/java/`. This integration test only
//! needs the public `Backend::generate_bindings` entry point and the public
//! `alef::docs::generate_docs` entry point, exactly like the Go parity test needs no production
//! change.
//!
//! Unlike the Go/Python fixtures, the Java backend's streaming wrapper reads its request type and
//! parameter name from `AdapterConfig::params` (`streaming_method_symbols`'s
//! `adapter.params[0]` access), not from `MethodDef::params` -- matching the PHP/Dart fixtures'
//! convention. Docs, however, still reads the request TYPE from `MethodDef::params` via
//! `first_param_type` and hardcodes the parameter NAME as the literal `req` -- this fixture keeps
//! both param lists in agreement (`req: CrawlRequest` on both the method and the adapter) so that
//! only the real, structural divergence below is what makes this test fail.

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "java"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.test"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP parity tests.
fn streaming_api() -> ApiSurface {
    ApiSurface {
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
        // The Java backend's streaming wrapper reads params from here, not from `MethodDef` --
        // see `streaming_method_symbols`'s `adapter.params[0]` access.
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "CrawlRequest".to_string(),
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

/// The pieces of the documented/emitted streaming shape this test compares, extracted by regex
/// so the comparison survives cosmetic differences (the real wrapper's iterator body vs the
/// docs' bare signature line) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    item_type: String,
    method_name: String,
    is_final_param: bool,
    request_type: String,
    request_param: String,
    exception_class: String,
}

fn extract_shape(java_text: &str) -> StreamingShape {
    let re = regex::Regex::new(r"public java\.util\.stream\.Stream<(\w+)> (\w+)\((final )?(\w+) (\w+)\) throws (\w+)")
        .unwrap();
    let caps = re
        .captures(java_text)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{java_text}"));
    StreamingShape {
        item_type: caps.get(1).unwrap().as_str().to_string(),
        method_name: caps.get(2).unwrap().as_str().to_string(),
        is_final_param: caps.get(3).is_some(),
        request_type: caps.get(4).unwrap().as_str().to_string(),
        request_param: caps.get(5).unwrap().as_str().to_string(),
        exception_class: caps.get(6).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Java opaque class through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's item type, method name, request parameter type/name, or
/// exception class that the other side does not also make.
///
/// Does NOT catch, deliberately: the real wrapper declares its request parameter `final`
/// (`streaming_iterator_method.jinja`'s `final {{ request_type }} {{ request_param }}`) while the
/// docs override emits no `final` modifier. That is not a caller-visible shape difference -- a
/// parameter's `final` modifier constrains only the method body, and is absent from the method's
/// overload resolution, its erased signature and everything a caller writes -- so documenting it
/// would add noise rather than accuracy. It is pinned below as a backend-only fact instead, so the
/// modifier cannot silently disappear from the emitted code without this test noticing.
#[test]
fn java_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = JavaBackend
        .generate_bindings(&api, &config)
        .expect("Java backend generates bindings");
    let engine_java = &backend_files
        .iter()
        .find(|file| file.path.ends_with("Engine.java"))
        .expect("Engine.java is generated")
        .content;
    let backend_shape = extract_shape(engine_java);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Java], "docs/reference").expect("docs generate");
    let api_java_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-java.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-java.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_shape(api_java_md);

    assert_eq!(
        backend_shape.item_type, docs_shape.item_type,
        "Stream item type must match between backend and docs.\nbackend:\n{engine_java}\ndocs:\n{api_java_md}"
    );
    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.request_type, docs_shape.request_type,
        "request parameter type must match between backend and docs"
    );
    assert_eq!(
        backend_shape.request_param, docs_shape.request_param,
        "request parameter name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.exception_class, docs_shape.exception_class,
        "throws exception class must match between backend and docs"
    );
    // Pin the real backend-emitted values too, not just cross-side equality -- so a change that
    // moves both sides in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.item_type, "CrawlEvent");
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.request_type, "CrawlRequest");
    assert_eq!(backend_shape.request_param, "req");
    assert_eq!(backend_shape.exception_class, "TestLibRsException");
    assert!(
        backend_shape.is_final_param,
        "the real wrapper always declares its request parameter final"
    );
}
