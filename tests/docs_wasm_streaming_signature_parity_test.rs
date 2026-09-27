//! Golden/parity coverage for issue #446 (Wasm side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Wasm streaming signature as a `format!` string instead of
//! rendering the real `#[wasm_bindgen]` method the way `WasmBackend::generate_bindings` does
//! (`gen_method`'s streaming branch, `src/backends/wasm/gen_bindings/methods.rs`), so nothing
//! forces the two to agree.
//!
//! Unlike Node, the WASM backend has no `generate_type_stubs`/`generate_public_api` override --
//! the actual `.d.ts` a JS/TS consumer sees is produced by the external `wasm-bindgen`/`wasm-pack`
//! CLI at build time from the `#[wasm_bindgen]`-annotated Rust source `generate_bindings` emits,
//! not by alef itself (see `src/backends/wasm/gen_bindings/mod.rs`'s single `impl Backend`
//! override). So the only backend text reachable through a public `Backend` trait method here is
//! that Rust source, not TypeScript -- this test compares the *identifiers* both sides agree on
//! (the JS-facing method name via `#[wasm_bindgen(js_name = ...)]`, and the streaming iterator
//! type name), not the full type syntax, since the Rust return type
//! (`Result<CrawlStreamIterator, JsValue>`) and the docs' TS-flavored return type
//! (`Promise<CrawlStreamIterator>`) are two different representations of the same identifier by
//! construction, not a claim either side is emitting the other's language.
//!
//! `method.is_async: true` mirrors the Node fixture: `gen_wasm_body`'s method body awaits the
//! core call inside a `wasm_bindgen_futures::spawn_local` task, but the realistic *source* trait
//! method a streaming adapter wraps is itself async (it must await before it can start
//! producing a stream) -- see `src/adapters/streaming.rs`'s `gen_wasm_body`.
//!
//! `gen_method` (`backends/wasm/gen_bindings/methods.rs`) is `pub(super)` to
//! `backends::wasm::gen_bindings`, unreachable from `src/docs` -- same story as the Go test's
//! `gen_streaming_method_wrapper`. This integration test only needs the public
//! `Backend::generate_bindings` entry point and the public `alef::docs::generate_docs` entry
//! point.

use alef::backends::wasm::WasmBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP/Node parity tests. `is_async: true`
/// for the same reason as the Node fixture (see the module doc comment).
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
        // The WASM backend's method signature (`gen_method`, `methods.rs`) reads params from
        // `MethodDef::params`, not from here -- same authority the Go/Node fixtures rely on.
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

/// The pieces of the documented/emitted streaming shape this test compares: the JS-facing method
/// name (via `#[wasm_bindgen(js_name = ...)]` on the backend side, and the plain method name in
/// the docs signature) and the streaming iterator type's identifier. Full return-type syntax is
/// deliberately NOT compared -- the backend text is Rust (`Result<CrawlStreamIterator, JsValue>`),
/// the docs text is TypeScript-flavored (`Promise<CrawlStreamIterator>`); see the module doc
/// comment for why these are two representations of one identifier, not a shape to diff whole.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    iterator_type: String,
}

fn extract_backend_shape(lib_rs: &str) -> StreamingShape {
    let js_name_re =
        regex::Regex::new(r#"#\[wasm_bindgen\(js_name = "(\w+)"\)\]\s*\n\s*pub async fn crawl_stream"#).unwrap();
    let js_name_caps = js_name_re
        .captures(lib_rs)
        .unwrap_or_else(|| panic!("no streaming method declaration found in:\n{lib_rs}"));

    let return_re = regex::Regex::new(r"pub async fn crawl_stream\([^)]*\) -> Result<(\w+), JsValue>").unwrap();
    let return_caps = return_re
        .captures(lib_rs)
        .unwrap_or_else(|| panic!("no streaming method return type found in:\n{lib_rs}"));

    StreamingShape {
        method_name: js_name_caps.get(1).unwrap().as_str().to_string(),
        iterator_type: return_caps.get(1).unwrap().as_str().to_string(),
    }
}

fn extract_docs_shape(api_wasm_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"(\w+)\([^)]*\): Promise<(\w+)>").unwrap();
    let caps = re
        .captures(api_wasm_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_wasm_md}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        iterator_type: caps.get(2).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real `#[wasm_bindgen]` method through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree on
/// the JS-facing method name and the streaming iterator type identifier.
///
/// Catches: a change to either side's JS method name or iterator type name that the other side
/// does not also make.
#[test]
fn wasm_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = WasmBackend
        .generate_bindings(&api, &config)
        .expect("wasm backend generates bindings");
    let lib_rs = &backend_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap_or_else(|| {
            panic!(
                "lib.rs is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_backend_shape(lib_rs);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Wasm], "docs/reference").expect("docs generate");
    let api_wasm_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-wasm.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-wasm.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_wasm_md);

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "JS-facing method name must match between backend and docs.\nbackend:\n{lib_rs}\ndocs:\n{api_wasm_md}"
    );
    assert_eq!(
        backend_shape.iterator_type, docs_shape.iterator_type,
        "streaming iterator type name must match between backend and docs.\nbackend:\n{lib_rs}\ndocs:\n{api_wasm_md}"
    );

    // Pin the real values too, not just cross-side equality -- so a change that moves both sides
    // in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.iterator_type, "CrawlStreamIterator");
}
