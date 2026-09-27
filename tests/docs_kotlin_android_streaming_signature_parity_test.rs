//! Golden/parity coverage for issue #446 (Kotlin Android side): `src/docs/language_pages/streaming.rs`
//! has NO override signature arm for `Language::KotlinAndroid` at all --
//! `streaming_method_signature_override`'s match falls through its `_ => None` catch-all, so the
//! documented streaming method signature is produced by the *generic* (non-streaming)
//! method-signature renderer (`render_method_signature_with_override`'s
//! `Language::Kotlin | Language::KotlinAndroid` arm, `src/docs/signatures.rs`), with only the
//! return type substituted from `streaming_return_type`'s `Language::KotlinAndroid` arm
//! (`format!("kotlinx.coroutines.flow.Flow<{item}>")`).
//!
//! UNLIKE `tests/docs_kotlin_streaming_signature_parity_test.rs`, this is NOT a real divergence:
//! `KotlinAndroidBackend::generate_bindings` does not use its own `handle_wrappers.rs`/
//! `android_streaming_method.jinja` emitter (bare, imported `Flow<T>`) for a type that has its own
//! instance methods -- that path (`emit_handle_wrapper`) only fires for a "handle-only" opaque type
//! that is *returned* by some other function/method but has no instance methods of its own
//! (`handle_only_type_names` explicitly excludes any type already classified as a "client type").
//! Our fixture's `Engine` type HAS an instance method (the streaming one itself), so it IS a
//! client type, and `KotlinAndroidBackend::generate_bindings`
//! (`src/backends/kotlin_android/gen_bindings.rs`) instead reuses the PLAIN Kotlin backend's own
//! JNI emitter wholesale: `crate::backends::kotlin::gen_bindings::jni_emitter::client_class::emit_jni_client_class`,
//! rendered through the shared `src/backends/kotlin/templates/jni_streaming_client_method.jinja`
//! template -- the exact same fully-qualified `kotlinx.coroutines.flow.Flow<T>` shape the docs
//! override's `Language::KotlinAndroid` return-type arm already documents. So for THIS backend,
//! docs and backend already agree, and this test is expected to PASS.
//!
//! Pattern mirrored from `tests/docs_kotlin_streaming_signature_parity_test.rs`. Because
//! KotlinAndroid also has no streaming signature override, the *parameter list* documented also
//! comes from `MethodDef::params` (the generic renderer's `method.params.iter()...`), while the
//! real backend's `emit_jni_streaming_client_method` (shared with plain Kotlin's JNI mode) reads
//! its parameter list from `AdapterConfig::params` (see its `adapter.params.iter()...`). This
//! fixture keeps `MethodDef::params` and `AdapterConfig::params` in sync so the parameter list
//! itself is not an unrelated source of mismatch.

use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["kotlin_android", "ffi"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.kotlin_android]
package = "dev.test_lib"
namespace = "dev.test_lib"
artifact_id = "test-lib-android"
group_id = "dev.test_lib"

[crates.package_metadata]
repository = "https://github.com/example/test-lib"
license = "MIT"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP/Java/Swift/Kotlin parity tests.
/// `Engine` having this method (rather than zero instance methods) is load-bearing: it is exactly
/// what makes `KotlinAndroidBackend` classify `Engine` as a "client type" and route it through the
/// shared plain-Kotlin JNI emitter instead of `handle_wrappers.rs` -- see the module doc above.
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
        // The shared JNI streaming emitter reads its parameter list from here, not from
        // `MethodDef` -- kept identical to `MethodDef::params` above because the docs side
        // (having no streaming override for KotlinAndroid either) reads `MethodDef::params`
        // instead.
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
/// docs' bare signature, an optional `@Throws`/`@Suppress` decorator).
#[derive(Debug, PartialEq)]
struct StreamingShape {
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(client_kt: &str) -> StreamingShape {
    let re =
        regex::Regex::new(r"fun (\w+)\(([^)]*)\): ([\w.<>]+) = kotlinx\.coroutines\.flow\.callbackFlow \{").unwrap();
    let caps = re
        .captures(client_kt)
        .unwrap_or_else(|| panic!("no streaming method wrapper found in:\n{client_kt}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

fn extract_docs_shape(api_kotlin_android_md: &str) -> StreamingShape {
    let re = regex::Regex::new(r"fun (\w+)\(([^)]*)\): ([\w.<>]+)").unwrap();
    let caps = re
        .captures(api_kotlin_android_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_kotlin_android_md}"));
    StreamingShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
        return_type: caps.get(3).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real Kotlin Android client class through the public
/// `Backend::generate_bindings` trait method and the documented signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// Catches: a change to either side's method name, parameter list, or return type that the other
/// side does not also make -- including a regression that makes `KotlinAndroidBackend` stop
/// reusing the plain Kotlin JNI emitter's fully-qualified `Flow` spelling (e.g. by routing a
/// client type through `handle_wrappers.rs`'s bare, imported `Flow<T>` instead), which would
/// silently reintroduce the short-vs-qualified divergence `tests/docs_kotlin_streaming_signature_parity_test.rs`
/// already pins for plain Kotlin.
#[test]
fn kotlin_android_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = KotlinAndroidBackend
        .generate_bindings(&api, &config)
        .expect("kotlin_android backend generates bindings");
    let client_kt = &backend_files
        .iter()
        .find(|file| file.path.file_name().and_then(|name| name.to_str()) == Some("DefaultClient.kt"))
        .unwrap_or_else(|| {
            panic!(
                "DefaultClient.kt is generated; got paths: {:?}",
                backend_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let backend_shape = extract_backend_shape(client_kt);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::KotlinAndroid], "docs/reference").expect("docs generate");
    let api_kotlin_android_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-kotlin-android.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-kotlin-android.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_shape(api_kotlin_android_md);

    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "method name must match between backend and docs.\nbackend:\n{client_kt}\ndocs:\n{api_kotlin_android_md}"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "parameter list must match between backend and docs"
    );
    assert_eq!(
        backend_shape.return_type, docs_shape.return_type,
        "return type must match between backend and docs"
    );

    // Pin the real backend-emitted values too, not just cross-side equality -- so a change that
    // moves both sides in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.method_name, "crawlStream");
    assert_eq!(backend_shape.params, "req: CrawlRequest");
    assert_eq!(backend_shape.return_type, "kotlinx.coroutines.flow.Flow<CrawlEvent>");
}
