//! Regression: `alef_ffi_error_code`'s `error: &dyn std::any::Any` parameter is only read
//! inside the per-error-type `downcast_ref` arms that `gen_last_error` emits from
//! `api.errors`. A crate with no typed error variants (every fallible function returns a
//! plain `String` error, a common shape) got a `fn alef_ffi_error_code(error: &dyn
//! std::any::Any) -> i32` whose body never reads `error` at all -- an `unused_variables`
//! warning in every such consumer's build.

use super::common::resolved_one;
use crate::backends::ffi::gen_bindings::last_error::gen_last_error;
use crate::core::ir::{ApiSurface, ErrorDef, ErrorVariant};

fn config() -> crate::core::config::ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
    )
}

#[test]
fn error_param_is_underscore_prefixed_when_no_error_types_are_registered() {
    let api = ApiSurface::default();
    let config = config();
    let module = gen_last_error(&api, &config.ffi_prefix(), "my_lib");

    assert!(
        module.contains("fn alef_ffi_error_code(_error: &dyn std::any::Any) -> i32"),
        "with no typed errors the parameter is never read, so it must be `_error` to avoid an \
         unused_variables warning in every consumer build; got:\n{module}"
    );
    assert!(!module.contains("(error: &dyn std::any::Any)"));
}

#[test]
fn error_param_stays_named_when_error_types_are_registered() {
    let api = ApiSurface {
        errors: vec![ErrorDef {
            name: "CoreError".to_string(),
            rust_path: "my_lib::CoreError".to_string(),
            original_rust_path: "CoreError".to_string(),
            variants: vec![ErrorVariant {
                name: "NotFound".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    };
    let config = config();
    let module = gen_last_error(&api, &config.ffi_prefix(), "my_lib");

    assert!(
        module.contains("fn alef_ffi_error_code(error: &dyn std::any::Any) -> i32"),
        "with a registered error type the downcast arm reads `error`, so it must keep its \
         name; got:\n{module}"
    );
    assert!(module.contains("error.downcast_ref::<my_lib::CoreError>()"));
}
