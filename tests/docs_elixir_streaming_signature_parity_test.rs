//! Golden/parity coverage for issue #446 (Elixir side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Elixir streaming signature as a `format!` string instead of
//! rendering the real wrapper the way `RustlerBackend::generate_public_api` does
//! (`elixir_streaming_unfold_wrapper.jinja`, rendered from
//! `src/backends/rustler/gen_bindings/public_api.rs`), so nothing forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `RustlerBackend::generate_public_api` (the real generated `.ex` module) and
//! `alef::docs::generate_docs` (the hand-built docs override) -- and compare the *shape* each one
//! emits: the method name and the parameter list.
//!
//! Like the PHP backend, the rustler streaming wrapper reads its request-parameter name from
//! `AdapterConfig::params` (`req_param = adapter.params.first()...`,
//! `src/backends/rustler/gen_bindings/public_api.rs`), not from `MethodDef::params` -- so this
//! fixture's `streaming_adapter()` populates `params`, matching the PHP fixture's convention.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "test_lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/PHP/Python parity tests.
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
        // The rustler backend's streaming wrapper reads the request parameter's name from here,
        // not from `MethodDef` -- see `req_param` in `public_api.rs`.
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
/// so the comparison survives cosmetic differences (the real wrapper's `do ... end` body vs the
/// docs' bare signature line) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    params: String,
}

fn extract_backend_shape(app_module_ex: &str) -> StreamingShape {
    // Anchored on the literal `crawl_stream` name, not a generic `def NAME(...)` capture: the
    // same file also emits `def engine_crawl_stream_start(client, req) do` and
    // `def engine_crawl_stream_next(handle) do` (the private `_start`/`_next` NIF delegates,
    // `elixir_streaming_start_wrapper.jinja` / `elixir_streaming_next_wrapper.jinja`), whose
    // `_start` wrapper happens to share the exact same `(client, req)` param shape as the public
    // method this test targets -- a generic capture finds that one first instead.
    let re = regex::Regex::new(r"def (crawl_stream)\(([^)]*)\) do").unwrap();
    let caps = re
        .captures(app_module_ex)
        .unwrap_or_else(|| panic!("no streaming method wrapper found in:\n{app_module_ex}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
    }
}

fn extract_docs_shape(api_elixir_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"def (\w+)\(([^)]*)\)").unwrap();
    let caps = re
        .captures(api_elixir_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_elixir_md}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Elixir public API module through the public
/// `Backend::generate_public_api` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name or parameter list (including the receiver
/// parameter's name) that the other side does not also make.
///
/// This test currently pins a REAL divergence rather than passing: the streaming wrapper
/// (`elixir_streaming_unfold_wrapper.jinja`) declares `def crawl_stream(client, req) do` --
/// the receiver parameter is named `client` -- while `streaming_method_signature_override`'s
/// `Language::Elixir` arm (`src/docs/language_pages/streaming.rs`) hand-builds
/// `format!("def {}(obj, req)", ...)`, always naming the receiver `obj`. The `~keep` comment
/// immediately above that arm attributes the `obj` spelling to
/// `gen_bindings/helpers/conversions.rs`'s `def_args.push("obj".to_string())`, but that helper
/// backs the *non-streaming* opaque-method wrapper path; the streaming wrapper is a separate,
/// hand-built template that never calls it and never uses `obj`.
#[test]
fn elixir_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("rustler backend generates public api");
    let app_module_ex = &backend_files
        .iter()
        .find(|file| file.path.ends_with("test_lib.ex"))
        .unwrap_or_else(|| {
            panic!(
                "test_lib.ex is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_backend_shape(app_module_ex);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Elixir], "docs/reference").expect("docs generate");
    let api_elixir_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-elixir.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-elixir.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_elixir_md);

    // Pin the real backend value first, independent of the docs side, so a future fix to
    // either side that lands on some other wrong shape still fails here.
    assert_eq!(backend_shape.method_name, "crawl_stream");
    assert_eq!(
        backend_shape.params, "client, req",
        "the real rustler streaming wrapper's receiver parameter is named `client`, not `obj`"
    );

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{app_module_ex}\ndocs:\n{api_elixir_md}"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "parameter list (including the receiver parameter's name) must match between backend \
         and docs.\nbackend: {}\ndocs: {}",
        backend_shape.params, docs_shape.params
    );
}
