//! Golden/parity coverage for issue #446 (PHP side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented PHP streaming signature as a `format!` string instead of rendering
//! the real wrapper the way `PhpBackend::generate_public_api` does
//! (`gen_php_streaming_method_wrapper`, `src/backends/php/gen_bindings/opaque_files.rs`), so
//! nothing forces the two to agree.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`: drive both public
//! generators from the exact same fixture `ApiSurface` / `ResolvedCrateConfig` --
//! `PhpBackend::generate_public_api` (the real opaque-class emitter) and `alef::docs::generate_docs`
//! (the hand-built docs override) -- and compare the *shape* each one emits: the method name, the
//! parameter list, and the declared return type.
//!
//! `gen_php_streaming_method_wrapper` is a private function inside `opaque_files.rs`, unreachable
//! from `src/docs` -- a shared-helper fix would need a visibility change under
//! `src/backends/php/`. This integration test only needs the public
//! `Backend::generate_public_api` entry point and the public `alef::docs::generate_docs` entry
//! point, exactly like the Go parity test needs no production change.
//!
//! Unlike the Go/Python fixtures, the PHP backend's streaming wrapper reads its parameter list
//! from `AdapterConfig::params` (the adapter-level param list), not `MethodDef::params` -- see
//! `gen_php_streaming_method_wrapper`'s `for p in &adapter.params` loop -- so this fixture's
//! `streaming_adapter()` populates `params`, where the Go fixture deliberately leaves it empty
//! (the Go backend reads `MethodDef::params` instead).

use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go and Python parity tests.
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
        // The PHP backend's streaming wrapper reads params from here, not from `MethodDef`.
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
/// so the comparison survives cosmetic differences (the real wrapper's throw-body vs the docs'
/// bare signature line) that are not part of the issue's "shape" concern.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(opaque_php: &str) -> StreamingShape {
    let re = regex::Regex::new(r"public function (\w+)\(([^)]*)\): (\S+)").unwrap();
    let caps = re
        .captures(opaque_php)
        .unwrap_or_else(|| panic!("no streaming method wrapper found in:\n{opaque_php}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

fn extract_docs_shape(api_php_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"public function (\w+)\(([^)]*)\): (\S+)").unwrap();
    let caps = re
        .captures(api_php_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_php_md}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real PHP opaque class through the public
/// `Backend::generate_public_api` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, parameter list, or return type that the other
/// side does not also make.
#[test]
fn php_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = PhpBackend
        .generate_public_api(&api, &config)
        .expect("php backend generates public api");
    let opaque_php = &backend_files
        .iter()
        .find(|file| file.path.ends_with("Engine.php"))
        .expect("Engine.php is generated")
        .content;
    let backend_shape = extract_backend_shape(opaque_php);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Php], "docs/reference").expect("docs generate");
    let api_php_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-php.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-php.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_php_md);

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{opaque_php}\ndocs:\n{api_php_md}"
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
    // PHP can't easily pass opaque types as function parameters, so there is no
    // `_start`/`_next`/`_free` handle triple here: the wrapper keeps the streaming loop on the
    // class itself and declares its return type as a PHP `\Generator`, consumed with `foreach`.
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.return_type, "\\Generator");
}
