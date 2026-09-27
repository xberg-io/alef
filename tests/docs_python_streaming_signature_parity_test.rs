//! Golden/parity coverage for issue #446 (Python side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Python streaming signature as a `format!` string instead of
//! rendering the real `.pyi` stub the way `Backend::generate_type_stubs` does
//! (`src/backends/pyo3/gen_stubs/classes.rs::gen_method_stub`), so nothing forces the two to
//! agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `Pyo3Backend::generate_type_stubs` (the real `.pyi` emitter) and `alef::docs::generate_docs`
//! (the hand-built docs override) -- and compare the *shape* each one emits: the `def`/`async
//! def` keyword, the method name, the parameter list, and the return type.
//!
//! `gen_method_stub` (`backends/pyo3/gen_stubs/classes.rs`) is a private function inside a
//! non-`pub` `gen_stubs` module, unreachable from `src/docs` -- a shared-helper fix would need a
//! visibility change under `src/backends/pyo3/`. This integration test only needs the public
//! `Backend::generate_type_stubs` entry point and the public `alef::docs::generate_docs` entry
//! point, exactly like the Go parity test needs no production change.

use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/src/"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go parity test, so any future addition of a
/// shared streaming fixture helper covers both.
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
/// so the comparison survives cosmetic differences (the real `.pyi` line ending in `: ...` vs
/// the docs' bare signature, indentation, self-vs-static) that are not part of the issue's
/// "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    def_keyword: String,
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(stub: &str) -> StreamingShape {
    // Anchored on an `AsyncIterator[...]` return type, not just "any `def`", because the fixture
    // also declares non-opaque dataclass-like stubs (`CrawlRequest`, `CrawlEvent`) whose
    // generated `__init__` methods otherwise match this same shape and sort earlier in the file.
    let re =
        regex::Regex::new(r"(async def|def) (\w+)\(self(?:, ([^)]*))?\) -> (AsyncIterator\[[^\]]+\]): \.\.\.").unwrap();
    let caps = re
        .captures(stub)
        .unwrap_or_else(|| panic!("no streaming method stub found in:\n{stub}"));
    StreamingShape {
        def_keyword: caps.get(1).unwrap().as_str().to_string(),
        method_name: caps.get(2).unwrap().as_str().to_string(),
        params: caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string(),
        return_type: caps.get(4).unwrap().as_str().trim().to_string(),
    }
}

fn extract_docs_shape(api_python_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"(async def|def) (\w+)\(self(?:, ([^)]*))?\) -> (AsyncIterator\[[^\]]+\])").unwrap();
    let caps = re
        .captures(api_python_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_python_md}"));
    StreamingShape {
        def_keyword: caps.get(1).unwrap().as_str().to_string(),
        method_name: caps.get(2).unwrap().as_str().to_string(),
        params: caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string(),
        return_type: caps.get(4).unwrap().as_str().trim().to_string(),
    }
}

/// The parity guard proper: generate the real `.pyi` stub through the public
/// `Backend::generate_type_stubs` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's `def`/`async def` keyword, method name, parameter list, or
/// return type that the other side does not also make.
#[test]
fn python_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = Pyo3Backend
        .generate_type_stubs(&api, &config)
        .expect("pyo3 backend generates type stubs");
    let stub = &backend_files.first().expect("a .pyi stub file is generated").content;
    let backend_shape = extract_backend_shape(stub);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Python], "docs/reference").expect("docs generate");
    let api_python_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-python.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-python.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_python_md);

    assert_eq!(
        backend_shape.def_keyword, docs_shape.def_keyword,
        "def/async def keyword must match between backend .pyi stub and docs.\n\
         backend:\n{stub}\ndocs:\n{api_python_md}"
    );
    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "parameter list must match between backend and docs"
    );
    assert_eq!(
        backend_shape.return_type, docs_shape.return_type,
        "return type must match between backend and docs"
    );

    // Pin the real values too, not just cross-side equality -- so a change that moves both
    // sides in lockstep to some other WRONG shape still fails.
    //
    // The `.pyi` stub deliberately types a streaming method as plain `def`, never `async def`,
    // even though the real generated wrapper (`adapter_streaming_wrapper.jinja`) IS an
    // `async def ...: ... yield ...` async generator function: calling an async generator
    // function returns the iterator synchronously (no `await` needed), and typing the call as
    // `async def -> AsyncIterator[T]` would type-check it as `Coroutine[Any, Any,
    // AsyncIterator[T]]`, rejecting the exact `async for chunk in stream_method(...)` call (no
    // `await`) both the real runtime shape and the docs' OWN example use. See
    // `gen_method_stub`'s doc comment in `src/backends/pyo3/gen_stubs/classes.rs`.
    assert_eq!(backend_shape.def_keyword, "def");
    assert_eq!(backend_shape.method_name, "crawl_stream");
    assert_eq!(backend_shape.params, "req: CrawlRequest");
    assert_eq!(backend_shape.return_type, "AsyncIterator[CrawlEvent]");
}
