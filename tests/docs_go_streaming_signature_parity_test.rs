//! Golden/parity coverage for issue #446: `src/docs/language_pages/streaming.rs` hand-builds the
//! documented Go streaming signature as a `format!` string instead of rendering the real
//! `src/backends/go/templates/streaming_*.jinja` templates, so nothing forces the two to agree.
//! #441 changed Go's streaming shape from a bare `(<-chan Item, error)` to a `*<Recv><Method>Stream`
//! iterator with `Chan()`/`Err()`, and the backend and docs sides had to be hand-edited separately.
//!
//! This test drives both public generators from the exact same fixture `ApiSurface` /
//! `ResolvedCrateConfig` -- `GoBackend::generate_bindings` (the real templates) and
//! `alef::docs::generate_docs` (the hand-built docs override) -- and compares the *shape* each
//! one emits for the same streaming method: the iterator type name, the `Chan()`/`Err()`
//! accessor signatures, the starting method's parameter list, and its return type. It does not
//! compile the output as Go (no toolchain dependency) and does not compare the emitted method
//! body -- only the declared signatures both sides agree a caller sees.
//!
//! `gen_streaming_method_wrapper` (`backends/go/gen_bindings/methods.rs`) is `pub(super)` to
//! `backends::go::gen_bindings` and unreachable from `src/docs`, so a shared-helper fix (issue's
//! option 1) would require a visibility change under `src/backends/go/` -- out of scope for this
//! change and owned by another concurrent workstream. This integration test (issue's option 2)
//! only needs the public `Backend` trait and the public `alef::docs::generate_docs` entry point.

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "example.invalid/test-lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the minimal shape that exercises the iterator type, both accessors, and a
/// non-empty parameter list on the starting method.
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
/// so the comparison survives cosmetic differences (whitespace, comments, the real method body
/// vs the docs' `/* ... */` placeholder) that are not part of the issue's "shape" concern.
///
/// `receiver_var` (the one-letter receiver name: backend uses `h` for an opaque receiver, docs
/// always uses `o` -- see `src/docs/signatures.rs`) is deliberately NOT compared: that is a real,
/// pre-existing, narrower divergence noted in issue #446 and left to a separate decision.
#[derive(Debug, PartialEq)]
struct StreamingShape {
    stream_type: String,
    chan_item_type: String,
    err_owner_matches_stream_type: bool,
    receiver_type: String,
    method_name: String,
    params: String,
    return_type: String,
}

fn extract_backend_shape(binding_go: &str) -> StreamingShape {
    // Anchored on the `Stream` suffix both sides mint (`format!("{{recv}}{{method}}Stream")`),
    // not just "any struct", because the fixture also declares plain data-carrying structs
    // (`CrawlRequest`, `CrawlEvent`) that would otherwise be the first `type ... struct` match.
    let struct_re = regex::Regex::new(r"type (\w+Stream) struct \{").unwrap();
    let stream_type = struct_re
        .captures(binding_go)
        .unwrap_or_else(|| panic!("no stream struct declaration found in:\n{binding_go}"))
        .get(1)
        .unwrap()
        .as_str()
        .to_string();

    let chan_re = regex::Regex::new(r"func \(s \*(\w+)\) Chan\(\) <-chan (\w+)").unwrap();
    let chan_caps = chan_re
        .captures(binding_go)
        .unwrap_or_else(|| panic!("no Chan() accessor found in:\n{binding_go}"));
    assert_eq!(
        chan_caps.get(1).unwrap().as_str(),
        stream_type,
        "Chan() must be declared on the stream type"
    );
    let chan_item_type = chan_caps.get(2).unwrap().as_str().to_string();

    let err_re = regex::Regex::new(r"func \(s \*(\w+)\) Err\(\) error").unwrap();
    let err_caps = err_re
        .captures(binding_go)
        .unwrap_or_else(|| panic!("no Err() accessor found in:\n{binding_go}"));
    let err_owner_matches_stream_type = err_caps.get(1).unwrap().as_str() == stream_type;

    let method_re = regex::Regex::new(r"func \(\w+ \*(\w+)\) (\w+)\(([^)]*)\) \(\*(\w+), error\) \{").unwrap();
    let method_caps = method_re
        .captures(binding_go)
        .unwrap_or_else(|| panic!("no streaming start-method signature found in:\n{binding_go}"));

    StreamingShape {
        stream_type,
        chan_item_type,
        err_owner_matches_stream_type,
        receiver_type: method_caps.get(1).unwrap().as_str().to_string(),
        method_name: method_caps.get(2).unwrap().as_str().to_string(),
        params: method_caps.get(3).unwrap().as_str().to_string(),
        return_type: format!("(*{}, error)", method_caps.get(4).unwrap().as_str()),
    }
}

fn extract_docs_shape(api_go_md: &str) -> StreamingShape {
    let struct_re = regex::Regex::new(r"type (\w+Stream) struct\{").unwrap();
    let stream_type = struct_re
        .captures(api_go_md)
        .unwrap_or_else(|| panic!("no stream struct declaration found in:\n{api_go_md}"))
        .get(1)
        .unwrap()
        .as_str()
        .to_string();

    let chan_re = regex::Regex::new(r"func \(s \*(\w+)\) Chan\(\) <-chan (\w+)").unwrap();
    let chan_caps = chan_re
        .captures(api_go_md)
        .unwrap_or_else(|| panic!("no Chan() accessor found in:\n{api_go_md}"));
    assert_eq!(
        chan_caps.get(1).unwrap().as_str(),
        stream_type,
        "Chan() must be declared on the stream type"
    );
    let chan_item_type = chan_caps.get(2).unwrap().as_str().to_string();

    let err_re = regex::Regex::new(r"func \(s \*(\w+)\) Err\(\) error").unwrap();
    let err_caps = err_re
        .captures(api_go_md)
        .unwrap_or_else(|| panic!("no Err() accessor found in:\n{api_go_md}"));
    let err_owner_matches_stream_type = err_caps.get(1).unwrap().as_str() == stream_type;

    let method_re = regex::Regex::new(r"func \(\w+ \*(\w+)\) (\w+)\(([^)]*)\) \(\*(\w+), error\)").unwrap();
    let method_caps = method_re
        .captures(api_go_md)
        .unwrap_or_else(|| panic!("no streaming start-method signature found in:\n{api_go_md}"));

    StreamingShape {
        stream_type,
        chan_item_type,
        err_owner_matches_stream_type,
        receiver_type: method_caps.get(1).unwrap().as_str().to_string(),
        method_name: method_caps.get(2).unwrap().as_str().to_string(),
        params: method_caps.get(3).unwrap().as_str().to_string(),
        return_type: format!("(*{}, error)", method_caps.get(4).unwrap().as_str()),
    }
}

/// The parity guard proper: generate the real Go binding through the public `Backend` trait and
/// the documented signature through the public `alef::docs::generate_docs` entry point from one
/// shared fixture, then assert the two agree on the streaming shape.
///
/// Catches: a change to either side's iterator type name, accessor signatures, start-method
/// name, parameter list, or return type that the other side does not also make.
///
/// Does NOT catch: a change to the emitted method *body* (goroutine, error handling), a change
/// to the receiver variable letter (known pre-existing divergence, see `StreamingShape`'s doc),
/// or drift in any language other than Go.
#[test]
fn go_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = GoBackend
        .generate_bindings(&api, &config)
        .expect("Go backend generates bindings");
    let binding_go = &backend_files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated")
        .content;
    let backend_shape = extract_backend_shape(binding_go);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Go], "docs/reference").expect("docs generate");
    let api_go_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-go.md"))
        .expect("api-go.md is generated")
        .content;
    let docs_shape = extract_docs_shape(api_go_md);

    assert_eq!(
        backend_shape.stream_type, docs_shape.stream_type,
        "iterator type name must match between backend and docs.\nbackend:\n{binding_go}\ndocs:\n{api_go_md}"
    );
    assert_eq!(
        backend_shape.chan_item_type, docs_shape.chan_item_type,
        "Chan() item type must match between backend and docs"
    );
    assert!(
        backend_shape.err_owner_matches_stream_type,
        "backend Err() must be declared on the stream type"
    );
    assert!(
        docs_shape.err_owner_matches_stream_type,
        "docs Err() must be declared on the stream type"
    );
    assert_eq!(
        backend_shape.receiver_type, docs_shape.receiver_type,
        "receiver type must match between backend and docs"
    );
    assert_eq!(
        backend_shape.method_name, docs_shape.method_name,
        "start-method name must match between backend and docs"
    );
    assert_eq!(
        backend_shape.params, docs_shape.params,
        "start-method parameter list must match between backend and docs"
    );
    assert_eq!(
        backend_shape.return_type, docs_shape.return_type,
        "start-method return type must match between backend and docs"
    );

    // Pin the real values too, not just cross-side equality -- so a change that moves both
    // sides in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_shape.stream_type, "EngineCrawlStreamStream");
    assert_eq!(backend_shape.chan_item_type, "CrawlEvent");
    assert_eq!(backend_shape.method_name, "CrawlStream");
    // #448: the starting method now takes a leading `ctx context.Context` (Go convention), so a
    // caller can unblock the forwarding goroutine's channel send after cancelling.
    assert_eq!(backend_shape.params, "ctx context.Context, req CrawlRequest");
    assert_eq!(backend_shape.return_type, "(*EngineCrawlStreamStream, error)");
}

/// Issue #449 fixed the mismatch pinned here previously: `src/docs/naming.rs::type_name`
/// (via `public_type_name`) used to run `heck::to_pascal_case()` on a Go type name before
/// calling `go_type_name`, while the backend calls `go_type_name(&typ.name)` directly on the
/// already-PascalCase Rust name. `heck::to_pascal_case()` mis-segments an irregular acronym run
/// (`RDFa` -> `RdFa`, see `go_variant_name`'s doc comment in `src/codegen/naming/languages.rs`),
/// so docs and the backend disagreed on an already-PascalCase type name that contains one.
///
/// `public_type_name`'s Go arm no longer applies that pre-step (it now calls
/// `go_type_name(name)` directly, matching every real backend call site), so the two sides now
/// agree here too. Verified directly against the naming primitive rather than through the full
/// docs/backend pipelines, since it is a naming-primitive fact, not a codegen-shape one --
/// `docs::naming::type_name` is `pub(crate)` and unreachable from an integration test.
///
/// If this ever starts failing, `public_type_name`'s Go arm has regained a pre-step that
/// re-mangles an already-PascalCase name -- treat that as a regression of #449, not as this test
/// needing updating.
#[test]
fn go_type_name_agrees_with_docs_public_type_name_on_irregular_acronym_run() {
    use alef::codegen::naming::{PublicIdentifierKind, public_host_identifier};

    let irregular = "RDFaChunk";

    let backend_type_name = alef::codegen::naming::go_type_name(irregular);
    assert_eq!(
        backend_type_name, "RDFaChunk",
        "backend must not re-segment an already-PascalCase acronym run"
    );

    // Drives the actual shared authority `src/docs/naming.rs::type_name` calls for
    // `Language::Go` (`public_casing` -> `public_type_name`, both `pub(crate)`/`pub(super)` and
    // unreachable from an integration test): its public entry point `public_host_identifier`.
    // Before #449 this returned `RdFaChunk` (heck's `to_pascal_case()` pre-step re-segmenting the
    // acronym run); it now calls `go_type_name(name)` directly, matching the backend.
    let docs_type_name = public_host_identifier(Language::Go, PublicIdentifierKind::Type, irregular);

    assert_eq!(
        backend_type_name, docs_type_name,
        "backend and docs must agree on an already-PascalCase acronym run after #449"
    );
    assert_eq!(
        docs_type_name, "RDFaChunk",
        "pin the exact spelling both sides now emit"
    );
}
