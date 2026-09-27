//! Golden/parity coverage for issue #446 (Kotlin side): `src/docs/language_pages/streaming.rs`
//! has NO override signature arm for `Language::Kotlin` at all --
//! `streaming_method_signature_override`'s match falls through its `_ => None` catch-all for
//! Kotlin, so the documented streaming method signature is produced by the *generic*
//! (non-streaming) method-signature renderer (`render_method_signature_with_override`'s
//! `Language::Kotlin | Language::KotlinAndroid` arm, `src/docs/signatures.rs`), with only the
//! return type substituted from `streaming_return_type`'s `Language::Kotlin` arm. Nothing else
//! checks that substituted spelling against what the real backend emits.
//!
//! The real backend (`KotlinBackend::generate_bindings`, JVM target,
//! `emit_streaming_client_method` in `src/backends/kotlin/gen_bindings/mod.rs`, rendered through
//! `src/backends/kotlin/templates/kotlin_streaming_client_method.jinja`) declares the return type
//! as the FULLY QUALIFIED `kotlinx.coroutines.flow.Flow<T>`. That arm used to say the bare
//! `Flow<T>` -- a real divergence this guard was written against, and since corrected in
//! `streaming_return_type` to the qualified spelling. The test pins the backend's value, so a
//! revert of that correction on either side fails here rather than silently re-diverging.
//!
//! Pattern mirrored from `tests/docs_go_streaming_signature_parity_test.rs`. Because Kotlin has no
//! streaming override, the *parameter list* documented also comes from `MethodDef::params`
//! (the generic renderer's `method.params.iter()...`), while the real backend's
//! `emit_streaming_client_method` reads its parameter list from `AdapterConfig::params` (see its
//! `adapter.params.iter()...`) -- unlike Go, but like PHP/Java/Swift. This fixture keeps
//! `MethodDef::params` and `AdapterConfig::params` in sync so the parameter list itself is not a
//! second, unrelated source of mismatch on top of the return-type divergence this test targets.

use alef::backends::kotlin::KotlinBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["kotlin", "java", "ffi"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "dev.testlib"

[crates.kotlin]
package = "dev.testlib"
target = "jvm"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP/Java/Swift parity tests.
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
        // The Kotlin backend's real streaming method reads its parameter list from here, not
        // from `MethodDef` -- kept identical to `MethodDef::params` above because the docs side
        // (having no streaming override for Kotlin) reads `MethodDef::params` instead.
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

/// The pieces of the documented/emitted streaming shape this test compares, extracted by regex so
/// the comparison survives cosmetic differences (the real method's `callbackFlow` body vs the
/// docs' bare signature, an optional `@Throws` decorator) that are not part of the issue's "shape"
/// concern -- except `return_type`, which is exactly the divergence under test.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(engine_kt: &str) -> StreamingShape {
    let re =
        regex::Regex::new(r"fun (\w+)\(([^)]*)\): ([\w.<>]+) = kotlinx\.coroutines\.flow\.callbackFlow \{").unwrap();
    let caps = re
        .captures(engine_kt)
        .unwrap_or_else(|| panic!("no streaming method wrapper found in:\n{engine_kt}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

fn extract_docs_shape(api_kotlin_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"fun (\w+)\(([^)]*)\): ([\w.<>]+)").unwrap();
    let caps = re
        .captures(api_kotlin_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_kotlin_md}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Kotlin client class through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, parameter list, or `Flow<T>` return-type
/// spelling that the other side does not also make. The return-type assertion is the one that
/// found a real divergence when this guard was written -- the backend emitted the fully-qualified
/// `kotlinx.coroutines.flow.Flow<CrawlEvent>` while `streaming_return_type`'s Kotlin arm
/// substituted the bare `Flow<CrawlEvent>` into the generic renderer. That arm has since been
/// corrected to the qualified spelling, so this now passes and guards against the regression.
#[test]
fn kotlin_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = KotlinBackend
        .generate_bindings(&api, &config)
        .expect("kotlin backend generates bindings");
    let engine_kt = &backend_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("Engine.kt"))
        .unwrap_or_else(|| {
            panic!(
                "Engine.kt is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_backend_shape(engine_kt);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Kotlin], "docs/reference").expect("docs generate");
    let api_kotlin_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-kotlin.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-kotlin.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_kotlin_md);

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{engine_kt}\ndocs:\n{api_kotlin_md}"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "parameter list must match between backend and docs"
    );
    assert_eq!(
        backend_shape.return_type, docs_shape.return_type,
        "return type must match between backend and docs -- the backend emits the fully-qualified \
         `kotlinx.coroutines.flow.Flow<T>`, and `streaming_return_type`'s Kotlin arm must keep \
         saying the same (it used to say the bare `Flow<T>`; issue #446)"
    );

    // Pin the real backend-emitted values too, not just cross-side equality -- so a change that
    // moves both sides in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.params, "req: CrawlRequest");
    assert_eq!(backend_shape.return_type, "kotlinx.coroutines.flow.Flow<CrawlEvent>");
}
