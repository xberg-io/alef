//! Golden/parity coverage for issue #446 (FFI/C side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented C streaming start-function signature as a `format!` string
//! (`streaming_c_start_signature` / `streaming_c_handle_type`) instead of rendering the real
//! `_start`/`_next`/`_free` handle triple the way `FfiBackend::generate_bindings` does
//! (`gen_stream_handle_functions`, `src/backends/ffi/gen_bindings/helpers.rs`), so nothing forces
//! the two to agree. This is the highest-value comparison in the streaming-docs sweep: Go, Java,
//! C#, Kotlin, Swift, and Dart all bind against this same generated C ABI, so a wrong documented
//! C signature here misleads every one of those pages' own "what does the underlying native call
//! look like" framing.
//!
//! Unlike the method-level streaming adapters (Go, PHP, Python, Elixir), the FFI backend emits
//! the `_start`/`_next`/`_free` triple unconditionally per streaming adapter -- gated only on the
//! adapter's `owner_type`/`item_type`/`request_type` all being `Some`, in a loop that runs before
//! (and independently of) the per-type method loop (`lib_rs.rs` lines ~191-227). No matching
//! `MethodDef` is required for the backend to emit it, but one is still included in this fixture
//! so `alef::docs::generate_docs`'s per-method streaming override (which does key off a matching
//! method) has something to attach to.
//!
//! # The real divergence this test pins
//!
//! Under the handle-ABI migration, every value crossing the C ABI -- including a streaming
//! adapter's `client`/`req` parameters -- is the scalar `AlefHandle` token
//! (`type AlefHandle = u64;`, declared literally and unprefixed in the generated Rust source,
//! `src/backends/ffi/templates/handle_registry.rs.jinja`). cbindgen's `[export] prefix` then
//! renames it in the final header to `{PREFIX}AlefHandle` -- this is the single source of truth
//! `crate::codegen::c_consumer::handle_type(prefix)` (public, used by every consumer backend)
//! already gives, and `doc_type`'s `TypeRef::Named` arm for `Ffi`/`C` already reaches the
//! identical string via `type_name(FFI_HANDLE_TYPE_NAME, lang, ffi_prefix)` -- see the `~keep`
//! comment on `streaming_c_start_signature`'s `owner_type`/`request_param` computation.
//!
//! The streaming *start function's return type*, however, is computed by a completely separate,
//! un-migrated function, `streaming_c_handle_type`, which still hand-builds a per-adapter,
//! uniquely-named `struct {PREFIX-SHOUTY}{Prefix}{Owner}{Adapter}StreamHandle *` pointer type --
//! the pre-handle-ABI-migration shape. The real `_start` function
//! (`src/backends/ffi/gen_bindings/helpers.rs`, `gen_stream_handle_functions`) declares its
//! return type as the bare scalar `AlefHandle`, exactly like its own parameters, not a pointer to
//! any such struct. `streaming_c_handle_type` never migrated when the params did.

use alef::backends::ffi::FfiBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi"]

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
/// `CrawlEvent`) -- the same fixture shape as the Go/PHP/Python parity tests.
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
        params: Vec::new(),
        returns: None,
        error_type: Some("String".to_string()),
        owner_type: Some("Engine".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        // Required (alongside `owner_type`/`item_type`) for `FfiBackend::generate_bindings` to
        // emit the `_start`/`_next`/`_free` triple at all -- see `lib_rs.rs`'s
        // `let Some(request_type) = adapter.request_type.as_deref() else { continue };`.
        request_type: Some("test_lib::CrawlRequest".to_string()),
        skip_languages: Vec::new(),
    }
}

fn streaming_config() -> ResolvedCrateConfig {
    let mut resolved = resolved_config();
    resolved.adapters = vec![streaming_adapter()];
    resolved
}

struct StartFnShape {
    name: String,
    client_type: String,
    req_type: String,
    return_type: String,
}

fn extract_backend_start_fn(lib_rs: &str) -> StartFnShape {
    let re =
        regex::Regex::new(r#"pub unsafe extern "C" fn (\w+)\(\s*client: (\w+),\s*req: (\w+),\s*\) -> (\w+)"#).unwrap();
    let caps = re
        .captures(lib_rs)
        .unwrap_or_else(|| panic!("no streaming _start function found in:\n{lib_rs}"));
    StartFnShape {
        name: caps.get(1).unwrap().as_str().to_string(),
        client_type: caps.get(2).unwrap().as_str().to_string(),
        req_type: caps.get(3).unwrap().as_str().to_string(),
        return_type: caps.get(4).unwrap().as_str().to_string(),
    }
}

fn extract_docs_start_fn(api_c_md: &str) -> StartFnShape {
    // The docs' `handle_type` (return type) is a "struct ... *" phrase with embedded spaces, so
    // it is captured non-greedily up to the function name/paren, rather than as a single `\w+`
    // token the way the backend's bare-scalar return type is.
    let re = regex::Regex::new(r"(.+?) (\w+)\((\w+) client, (\w+) req\);").unwrap();
    let caps = re
        .captures(api_c_md)
        .unwrap_or_else(|| panic!("no streaming _start function signature found in:\n{api_c_md}"));
    StartFnShape {
        return_type: caps.get(1).unwrap().as_str().to_string(),
        name: caps.get(2).unwrap().as_str().to_string(),
        client_type: caps.get(3).unwrap().as_str().to_string(),
        req_type: caps.get(4).unwrap().as_str().to_string(),
    }
}

/// The parity guard proper: generate the real FFI crate's `lib.rs` through the public
/// `Backend::generate_bindings` trait method and the documented C signature through the public
/// `alef::docs::generate_docs` entry point from one shared fixture, then assert the two agree.
///
/// This test currently pins a REAL divergence rather than passing -- see the module doc comment
/// for the full explanation. The function name and both parameter types already agree; only the
/// declared return type disagrees.
#[test]
fn ffi_c_streaming_start_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("ffi backend generates bindings");
    let lib_rs = &backend_files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("lib.rs is generated")
        .content;
    let backend_shape = extract_backend_start_fn(lib_rs);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Ffi], "docs/reference").expect("docs generate");
    let api_c_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-c.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-c.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_shape = extract_docs_start_fn(api_c_md);

    // Pin the real backend value first, independent of the docs side: every position in the
    // real `_start` function's declared signature is the identical bare scalar handle type --
    // there is no per-adapter struct pointer anywhere in it.
    assert_eq!(backend_shape.name, "test_engine_crawl_stream_start");
    assert_eq!(backend_shape.client_type, "AlefHandle");
    assert_eq!(backend_shape.req_type, "AlefHandle");
    assert_eq!(backend_shape.return_type, "AlefHandle");

    // The single public source of truth for what the raw `AlefHandle` scalar becomes in the
    // final cbindgen-rendered header, shared by every consumer backend.
    let expected_handle = alef::codegen::c_consumer::handle_type("test");
    assert_eq!(expected_handle, "TESTAlefHandle");

    assert_eq!(
        backend_shape.name, docs_shape.name,
        "start-function name must match between backend and docs"
    );
    assert_eq!(
        docs_shape.client_type, expected_handle,
        "docs client-param type must be the shared handle-type spelling"
    );
    assert_eq!(
        docs_shape.req_type, expected_handle,
        "docs request-param type must be the shared handle-type spelling"
    );
    assert_eq!(
        docs_shape.return_type, expected_handle,
        "docs return type must be the shared handle-type spelling, matching the real backend's \
         bare `AlefHandle` return -- not a per-adapter struct pointer.\nbackend return type: {}\n\
         docs return type: {}\nfull docs line: {api_c_md}",
        backend_shape.return_type, docs_shape.return_type
    );
}

/// Confirms `Language::Jni` is not independently testable this way at all: `alef::docs::generate_docs`
/// explicitly skips it before ever reaching `streaming.rs`'s `Language::Jni` arm (see the `~keep`
/// comment in `src/docs/mod.rs` on the `matches!(lang, Language::C | Language::Jni) { continue; }`
/// guard), and the only other entry point that would reach it, `language_pages::generate_lang_doc`,
/// is `pub(super)` -- unreachable from an integration test. So even though
/// `streaming_method_name`/`streaming_method_signature_override`/`streaming_return_type` all have
/// a `Language::Jni` match arm that shares the FFI/C code path (`streaming_c_start_name` et al.),
/// no doc page is ever produced for it in practice: the JNI backend's real emitted native function
/// (`Java_..._native...Start`, taking `JNIEnv`/`JClass`/`jlong`/`JString`, see
/// `src/backends/jni/templates/streaming_shims.rs.jinja`) shares neither the naming scheme nor the
/// signature shape those arms document, but no consumer of `generate_docs` can ever observe that,
/// because the arm is dead code from the documented-page perspective. This is asserted here as a
/// negative control, not a parity check: requesting a Jni-only page must produce no page.
#[test]
fn jni_produces_no_documented_streaming_page_because_generate_docs_skips_it() {
    let api = streaming_api();
    let mut config = streaming_config();
    config.adapters[0].owner_type = Some("Engine".to_string());

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Jni], "docs/reference").expect("docs generate");

    assert!(
        docs_files
            .iter()
            .all(|file| !file.path.to_string_lossy().contains("api-")),
        "requesting Language::Jni must produce no per-language api-*.md page at all; got: {:?}",
        docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
    );
}
