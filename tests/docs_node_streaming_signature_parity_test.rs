//! Golden/parity coverage for issue #446 (Node side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Node streaming signature as a `format!` string instead of
//! rendering the real `index.d.ts` stub the way `NapiBackend::generate_type_stubs` does
//! (`gen_dts`'s per-class-method loop, `src/backends/napi/gen_bindings/errors.rs`), so nothing
//! forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `NapiBackend::generate_type_stubs` (the real `index.d.ts` emitter) and `alef::docs::generate_docs`
//! (the hand-built docs override) -- and compare the *shape* each one emits: the method name, the
//! parameter list, and the declared return type.
//!
//! Unlike the Go/Python/PHP fixtures, `method.is_async` is set to `true` here: `gen_dts`'s
//! streaming-return-type branch (`errors.rs`) unconditionally wraps the declared return type in
//! `Promise<...>` for a streaming class method, which is only accurate when the real emitted
//! `#[napi]` wrapper is genuinely `async` (`gen_opaque_instance_method`, `types.rs`, gates the
//! `async` keyword on `method.is_async`). A real streaming adapter always awaits the core call
//! before it can produce a stream, so `is_async: true` is the realistic shape, not an arbitrary
//! fixture choice -- see `gen_node_body`'s `inner.{core_path}(...).await` in
//! `src/adapters/streaming.rs`.
//!
//! `gen_dts` is `pub(super)` to `backends::napi::gen_bindings`, unreachable from `src/docs` --
//! same story as the Go test's `gen_streaming_method_wrapper`. This integration test only needs
//! the public `Backend::generate_type_stubs` entry point and the public
//! `alef::docs::generate_docs` entry point.

use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.node]
package_name = "test-lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP parity tests, with `is_async: true`
/// (see the module doc comment for why that is the realistic value here, not just a fixture
/// convenience).
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
                    is_async: true,
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
        // The napi `.d.ts` class-method loop reads params from `MethodDef::params`, not from
        // here -- see `gen_dts`'s `dts_params(&method.params, ...)` call, the same authority the
        // Go fixture relies on (unlike the PHP fixture, which must populate this field).
        params: Vec::new(),
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
/// so the comparison survives cosmetic differences (interface ordering, blank lines) that are
/// not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(index_d_ts: &str) -> StreamingShape {
    let re = regex::Regex::new(r"crawlStream\(([^)]*)\): ([^\n]+)").unwrap();
    let caps = re
        .captures(index_d_ts)
        .unwrap_or_else(|| panic!("no streaming method declaration found in:\n{index_d_ts}"));
    StreamingShape {
        method_name: "crawlStream".to_string(),
        params: caps.get(1).unwrap().as_str().to_string(),
        return_type: caps.get(2).unwrap().as_str().trim().to_string(),
    }
}

fn extract_docs_shape(api_node_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"crawlStream\(([^)]*)\): ([^\n]+)").unwrap();
    let caps = re
        .captures(api_node_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_node_md}"));
    StreamingShape {
        method_name: "crawlStream".to_string(),
        params: caps.get(1).unwrap().as_str().to_string(),
        return_type: caps.get(2).unwrap().as_str().trim().to_string(),
    }
}

/// The parity guard proper: generate the real `index.d.ts` stub through the public
/// `Backend::generate_type_stubs` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, parameter list, or return type that the other
/// side does not also make.
///
/// As of this writing this FAILS: `gen_dts`'s streaming branch
/// (`src/backends/napi/gen_bindings/errors.rs:220-233`) declares the class method's return type as
/// `Promise<AsyncGenerator<{item_type}, void, undefined>>`, while the docs override
/// (`src/docs/language_pages/streaming.rs:241-248`, the `Language::Node | Language::Wasm` arm of
/// `streaming_return_type`) says `Promise<{Adapter}Iterator>`. See the handback report for the
/// full citation and both literal values.
#[test]
fn node_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = NapiBackend
        .generate_type_stubs(&api, &config)
        .expect("napi backend generates type stubs");
    let index_d_ts = &backend_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("index.d.ts"))
        .unwrap_or_else(|| {
            panic!(
                "index.d.ts is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_backend_shape(index_d_ts);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Node], "docs/reference").expect("docs generate");
    let api_node_md = &docs_files
        .iter()
        // `docs::naming::lang_slug(Language::Node)` is `"typescript"`, not `"node"` -- the page
        // file is named after the slug, not the `Language` variant.
        .find(|file| file.path.to_string_lossy().ends_with("api-typescript.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-typescript.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_node_md);

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{index_d_ts}\ndocs:\n{api_node_md}"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "parameter list must match between backend and docs"
    );
    assert_eq!(
        backend_shape.return_type, docs_shape.return_type,
        "return type must match between backend and docs.\nbackend:\n{index_d_ts}\ndocs:\n{api_node_md}"
    );

    // Pin the real backend value too, not just cross-side equality -- so a change that moves
    // both sides in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.params, "req: CrawlRequest");
    assert_eq!(
        backend_shape.return_type,
        "Promise<AsyncGenerator<CrawlEvent, void, undefined>>"
    );
}
