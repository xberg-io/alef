//! Golden/parity coverage for issue #446 (Ruby side): `src/docs/language_pages/streaming.rs`
//! hand-builds the documented Ruby streaming signature as a `format!` string instead of
//! rendering the real Magnus method the way `MagnusBackend::generate_bindings` does
//! (`gen_streaming_method_body`/`gen_iterator_struct`, `src/backends/magnus/gen_bindings/streaming.rs`),
//! so nothing forces the two to agree.
//!
//! Ruby `def` syntax carries no return-type annotation, so the docs signature line
//! (`streaming_method_signature_override` has no `Language::Ruby` arm; the generic
//! `Language::Ruby` renderer in `src/docs/signatures.rs` prints only `def name(params)`) is
//! compared on method name and parameter list, and the streaming iterator *type name* is
//! compared separately against the docs' "Returns" override text
//! (`streaming_return_type`'s `Language::Ruby` arm) and against the real `pub struct
//! {iterator}` Magnus emits.
//!
//! The real Ruby method's return value is dynamically typed (`Result<magnus::Value, Error>`):
//! it returns `nil` when a block is given (each chunk is yielded instead) and the iterator
//! instance otherwise -- see `gen_streaming_method_body`'s doc comment. Comparing a single
//! frozen "return type" is a documentation simplification true only of the no-block call form,
//! which is exactly the form the docs' own example uses (`stream = instance.crawl_stream(...)`,
//! no block).
//!
//! `gen_streaming_method_body`/`gen_iterator_struct` are `pub(super)` to
//! `backends::magnus::gen_bindings`, unreachable from `src/docs` -- same story as the Go test's
//! `gen_streaming_method_wrapper`. This integration test only needs the public
//! `Backend::generate_bindings` entry point and the public `alef::docs::generate_docs` entry
//! point.

use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config parses");
    cfg.resolve().expect("test config resolves").remove(0)
}

/// An opaque `Engine` with one streaming method (`crawl_stream(req: CrawlRequest)`, yielding
/// `CrawlEvent`) -- the same fixture shape as the Go/Python/PHP/Node/Wasm parity tests.
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
        core_path: "crawl_stream".to_string(),
        // The Magnus streaming method body (`gen_streaming_method_body`) reads params from
        // `MethodDef::params` via the `StreamingAdapter::from_config` construction, not from
        // here -- same authority the Go/Node/Wasm fixtures rely on.
        params: Vec::new(),
        returns: None,
        error_type: Some("String".to_string()),
        owner_type: Some("Engine".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        // `StreamingAdapter::from_config` requires `request_type` (used for the request
        // parameter's core-path conversion) -- unlike the Go/Node/Wasm fixtures, Magnus needs
        // this populated or the adapter is silently skipped (`from_config` returns `None`).
        request_type: Some("test_lib::CrawlRequest".to_string()),
        skip_languages: Vec::new(),
    }
}

fn streaming_config() -> ResolvedCrateConfig {
    let mut resolved = resolved_config();
    resolved.adapters = vec![streaming_adapter()];
    resolved
}

/// The pieces of the documented/emitted streaming shape this test compares: the method name, the
/// parameter list (Ruby `def` syntax carries no return-type annotation, so there is no return
/// type to extract from the signature line itself), and the streaming iterator type name
/// (compared separately, against the backend's real `pub struct {iterator}` declaration and the
/// docs' "Returns" override text).
#[derive(Debug, PartialEq)]
struct StreamingSignatureShape {
    method_name: String,
    params: String,
}

fn extract_backend_signature_shape(lib_rs: &str) -> StreamingSignatureShape {
    // Only the param NAME is compared, not its type: the Magnus binding declares a typed Rust
    // param (`req: CrawlRequest`), while Ruby `def` syntax -- and so the docs signature line --
    // has no type annotation at all.
    let re = regex::Regex::new(r"fn (\w+)\(&self, (\w+): \w+\) -> Result<magnus::Value, Error>").unwrap();
    let caps = re
        .captures(lib_rs)
        .unwrap_or_else(|| panic!("no streaming method declaration found in:\n{lib_rs}"));
    StreamingSignatureShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
    }
}

fn extract_docs_signature_shape(api_ruby_md: &str) -> StreamingSignatureShape {
    let re = regex::Regex::new(r"def (\w+)\(([^)]*)\)").unwrap();
    let caps = re
        .captures(api_ruby_md)
        .unwrap_or_else(|| panic!("no streaming method signature found in:\n{api_ruby_md}"));
    StreamingSignatureShape {
        method_name: caps.get(1).unwrap().as_str().to_string(),
        params: caps.get(2).unwrap().as_str().to_string(),
    }
}

fn extract_backend_iterator_type(lib_rs: &str) -> String {
    let re = regex::Regex::new(r"pub struct (\w+Iterator) \{").unwrap();
    re.captures(lib_rs)
        .unwrap_or_else(|| panic!("no streaming iterator struct declaration found in:\n{lib_rs}"))
        .get(1)
        .unwrap()
        .as_str()
        .to_string()
}

fn extract_docs_iterator_type(api_ruby_md: &str) -> String {
    // `push_returns_with_override` renders `**Returns:** \`{ty}\`` (`returns.jinja`) for a
    // `TypeRef::Unit`-declared method with an override -- the streaming case here.
    let re = regex::Regex::new(r"Returns:?\**\s*`?(\w+Iterator)").unwrap();
    re.captures(api_ruby_md)
        .unwrap_or_else(|| panic!("no streaming iterator type mentioned in:\n{api_ruby_md}"))
        .get(1)
        .unwrap()
        .as_str()
        .to_string()
}

/// The parity guard proper: generate the real Magnus binding through the public
/// `Backend::generate_bindings` trait method and the documented signature/return description
/// through the public `alef::docs::generate_docs` entry point from one shared fixture, then
/// assert the two agree on the method name, parameter list (from the `def` line, since Ruby has
/// no signature-level return-type syntax to compare), and streaming iterator type name.
///
/// Catches: a change to either side's method name, parameter name, or iterator type name that the
/// other side does not also make.
#[test]
fn ruby_streaming_signature_matches_backend_emitted_shape() {
    let api = streaming_api();
    let config = streaming_config();

    let backend_files = MagnusBackend
        .generate_bindings(&api, &config)
        .expect("magnus backend generates bindings");
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
    let backend_signature = extract_backend_signature_shape(lib_rs);
    let backend_iterator_type = extract_backend_iterator_type(lib_rs);

    let docs_files =
        alef::docs::generate_docs(&api, &config, &[Language::Ruby], "docs/reference").expect("docs generate");
    let api_ruby_md = &docs_files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("api-ruby.md"))
        .unwrap_or_else(|| {
            panic!(
                "api-ruby.md is generated; got paths: {:?}",
                docs_files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
            )
        })
        .content;
    let docs_signature = extract_docs_signature_shape(api_ruby_md);
    let docs_iterator_type = extract_docs_iterator_type(api_ruby_md);

    assert_eq!(
        backend_signature.method_name, docs_signature.method_name,
        "method name must match between backend and docs.\nbackend:\n{lib_rs}\ndocs:\n{api_ruby_md}"
    );
    assert_eq!(
        backend_signature.params, docs_signature.params,
        "parameter list must match between backend and docs (backend params are typed, docs are \
         bare Ruby idents, so only the name is compared)."
    );
    assert_eq!(
        backend_iterator_type, docs_iterator_type,
        "streaming iterator type name must match between backend and docs.\nbackend:\n{lib_rs}\ndocs:\n{api_ruby_md}"
    );

    // Pin the real values too, not just cross-side equality -- so a change that moves both sides
    // in lockstep to some other WRONG shape still fails.
    assert_eq!(backend_signature.method_name, "crawl_stream");
    assert_eq!(backend_iterator_type, "CrawlStreamIterator");
}
