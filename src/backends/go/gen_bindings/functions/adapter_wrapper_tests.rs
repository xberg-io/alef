//! Tests for [`super::adapter_flattened_field`] and [`super::gen_adapter_wrapper`].
//!
//! `adapter_flattened_field` is the single source of truth for whether a streaming adapter's Go
//! wrapper decomposes its one configured parameter into a scalar field, or exposes
//! `[[adapters]] params` unchanged. These tests pin that decision directly, then pin the exact
//! rendered `func`/`req :=`/`return` lines `gen_adapter_wrapper` builds from it, so a change to
//! either the decision or its rendering fails a test instead of silently drifting from what
//! e2e/snippet code (which now consults the same function -- see
//! `crate::e2e::codegen::go::snippet`) expects.

use super::{adapter_flattened_field, gen_adapter_wrapper};
use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};

fn streaming_adapter(params: Vec<AdapterParam>, request_type: Option<&str>) -> AdapterConfig {
    AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "sample::CrawlEngine::crawl_stream".to_string(),
        params,
        returns: None,
        error_type: None,
        owner_type: Some("CrawlEngineHandle".to_string()),
        item_type: Some("sample::CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: request_type.map(str::to_string),
        skip_languages: Vec::new(),
    }
}

fn adapter_param(name: &str, ty: &str) -> AdapterParam {
    AdapterParam {
        name: name.to_string(),
        ty: ty.to_string(),
        optional: false,
    }
}

fn dto(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..TypeDef::default()
    }
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// The exact `func ...(...) (...)` signature line, located by content rather than position so
/// the assertion does not depend on how many blank/whitespace-only lines the template's
/// `{% if %}` block leaves around it.
fn signature_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("func "))
        .expect("gen_adapter_wrapper always emits a `func` line")
}

/// The exact `return ...` call line, located the same way as [`signature_line`].
fn call_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("return "))
        .expect("gen_adapter_wrapper always emits a `return` line")
}

/// The exact `req := &Type{...}` construction line, when the wrapper flattened its parameter.
fn request_construction_line(rendered: &str) -> Option<&str> {
    rendered.lines().find(|line| line.trim_start().starts_with("req :="))
}

// -- `adapter_flattened_field`: the shared decision --

#[test]
fn flattens_the_single_configured_params_first_field() {
    let adapter = streaming_adapter(
        vec![adapter_param("request", "CrawlRequest")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

    let flattened = adapter_flattened_field(&adapter, &types);

    assert_eq!(flattened.map(|f| f.name.as_str()), Some("url"));
    assert_eq!(flattened.map(|f| &f.ty), Some(&TypeRef::String));
}

#[test]
fn does_not_flatten_two_configured_params() {
    let adapter = streaming_adapter(
        vec![adapter_param("url", "String"), adapter_param("depth", "u32")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

    assert!(adapter_flattened_field(&adapter, &types).is_none());
}

#[test]
fn does_not_flatten_a_fieldless_request_type() {
    let adapter = streaming_adapter(
        vec![adapter_param("request", "CrawlRequest")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", Vec::new())];

    assert!(adapter_flattened_field(&adapter, &types).is_none());
}

#[test]
fn does_not_flatten_when_the_param_type_is_absent_from_the_ir() {
    let adapter = streaming_adapter(
        vec![adapter_param("request", "CrawlRequest")],
        Some("sample::CrawlRequest"),
    );

    assert!(adapter_flattened_field(&adapter, &[]).is_none());
}

#[test]
fn does_not_flatten_without_a_configured_request_type() {
    let adapter = streaming_adapter(vec![adapter_param("request", "CrawlRequest")], None);
    let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

    assert!(adapter_flattened_field(&adapter, &types).is_none());
}

// -- `gen_adapter_wrapper`: the Go wrapper rendered from that decision --

/// Control: the common, currently-live shape (single configured param, request DTO resolves
/// with a first field) must keep rendering exactly as it did before `adapter_flattened_field`
/// was extracted out of `gen_adapter_wrapper`.
#[test]
fn single_param_adapter_decomposes_into_a_scalar_go_parameter() {
    let adapter = streaming_adapter(
        vec![adapter_param("request", "CrawlRequest")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

    let rendered = gen_adapter_wrapper(&adapter, "pkg", &types);

    assert_eq!(
        signature_line(&rendered),
        "func CrawlStream(ctx context.Context, engine *CrawlEngineHandle, URL string) (*CrawlEngineHandleCrawlStreamStream, error) {"
    );
    assert_eq!(
        request_construction_line(&rendered),
        Some("\treq := &CrawlRequest{URL: URL}")
    );
    assert_eq!(call_line(&rendered), "\treturn engine.CrawlStream(ctx, *req)");
}

/// Else branch, arity sub-case: two configured adapter params never satisfy the `params.len()
/// == 1` gate, so each is exposed as its own Go parameter and the call passes them positionally
/// -- no `req :=` construction at all.
#[test]
fn two_param_adapter_exposes_each_configured_param_directly() {
    let adapter = streaming_adapter(
        vec![adapter_param("url", "String"), adapter_param("depth", "u32")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

    let rendered = gen_adapter_wrapper(&adapter, "pkg", &types);

    assert_eq!(
        signature_line(&rendered),
        "func CrawlStream(ctx context.Context, engine *CrawlEngineHandle, url string, depth u32) (*CrawlEngineHandleCrawlStreamStream, error) {"
    );
    assert_eq!(request_construction_line(&rendered), None);
    assert_eq!(call_line(&rendered), "\treturn engine.CrawlStream(ctx, url, depth)");
}

/// Else branch, fieldless sub-case: a single configured param whose declared type resolves in
/// the IR but has no fields to decompose falls back to the same raw-params rendering as the
/// two-param case.
#[test]
fn fieldless_request_type_falls_back_to_the_configured_param() {
    let adapter = streaming_adapter(
        vec![adapter_param("request", "CrawlRequest")],
        Some("sample::CrawlRequest"),
    );
    let types = vec![dto("CrawlRequest", Vec::new())];

    let rendered = gen_adapter_wrapper(&adapter, "pkg", &types);

    assert_eq!(
        signature_line(&rendered),
        "func CrawlStream(ctx context.Context, engine *CrawlEngineHandle, request CrawlRequest) (*CrawlEngineHandleCrawlStreamStream, error) {"
    );
    assert_eq!(request_construction_line(&rendered), None);
    assert_eq!(call_line(&rendered), "\treturn engine.CrawlStream(ctx, request)");
}

// -- issue #447: `owner_type` must go through the same normalization as the receiver type --

/// An `owner_type` containing an initialism (`Api`) must render through `go_type_name` exactly
/// the way `methods.rs` renders the IR type name it names -- `ApiEngine` -> `APIEngine` -- for
/// both the receiver parameter and the `<Owner><Method>Stream` type name. A config value without
/// an initialism (`CrawlEngineHandle`, as used above) is a no-op under this normalization and
/// would pass even with the bug present; this case is the one that discriminates. ~keep
#[test]
fn owner_type_with_an_initialism_is_normalized_like_the_receiver_type() {
    let mut adapter = streaming_adapter(Vec::new(), Some("sample::CrawlRequest"));
    adapter.owner_type = Some("ApiEngine".to_string());

    let rendered = gen_adapter_wrapper(&adapter, "pkg", &[]);

    assert_eq!(
        signature_line(&rendered),
        "func CrawlStream(ctx context.Context, engine *APIEngine) (*APIEngineCrawlStreamStream, error) {"
    );
    assert_eq!(call_line(&rendered), "\treturn engine.CrawlStream(ctx)");
}
