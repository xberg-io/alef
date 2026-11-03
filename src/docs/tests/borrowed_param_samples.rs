//! Since the extractor records a param's borrow shape in `original_type` (`&Config`,
//! `&mut Config`, `Option<&Config>`), the Rust docs sampler must resolve the named type behind the
//! borrow. Matching the borrowed spelling against `rust_path`/`name` found nothing, so every
//! borrowed struct param rendered as `todo!()` instead of `Config::default()`.

use super::*;
use crate::core::ir::{ParamDef, TypeDef};
use crate::docs::rust_param_samples::rust_original_param_sample;

fn api_with_default_config() -> ApiSurface {
    ApiSurface {
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "sample_core::Config".to_string(),
            has_default: true,
            ..Default::default()
        }],
        ..make_minimal_api("0.1.0")
    }
}

fn borrowed_param(original: &str) -> ParamDef {
    ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        original_type: Some(original.to_string()),
        is_ref: true,
        ..Default::default()
    }
}

#[test]
fn borrowed_struct_param_samples_the_types_default() {
    let api = api_with_default_config();
    for original in [
        "&Config",
        "&mut Config",
        "&'a Config",
        "Option<&Config>",
        "&sample_core::Config",
    ] {
        let sample = rust_original_param_sample(&borrowed_param(original), Language::Rust, &api);
        let expected = if original.contains("sample_core::") {
            "sample_core::Config::default()"
        } else {
            "Config::default()"
        };
        assert_eq!(sample.as_deref(), Some(expected), "original_type `{original}`");
    }
}

/// Negative control: a borrowed type with no `Default` still renders the placeholder, so the fix
/// resolves the borrowed name rather than defaulting every borrowed param.
#[test]
fn borrowed_struct_param_without_default_keeps_the_placeholder() {
    let mut api = api_with_default_config();
    api.types[0].has_default = false;
    let sample = rust_original_param_sample(&borrowed_param("&Config"), Language::Rust, &api);
    assert_eq!(sample.as_deref(), Some("todo!()"));
}
