//! alef #452: a plain (non-adapter, non-bridge, non-capsule) top-level function whose core call
//! returns `Result<_, {SomeError}>` must raise the binding's TYPED exception on failure, not a
//! generic `builtins.RuntimeError` with the error kind surviving only as message text.
//!
//! `gen_pyo3_error_converter` already emits a `{error}_to_py_err` converter that maps each
//! variant to its own Python exception class -- the trait-bridge (`bridge_methods.rs`) and
//! capsule (`capsule_methods.rs`, `capsule.rs`) call sites already route through it by name. The
//! plain free-function path (`codegen::generators::functions::gen_function_with_mutex`) never
//! received that converter list at all, so it always fell back to the pattern's generic
//! conversion (`pyo3::exceptions::PyRuntimeError::new_err(e.to_string())`) regardless of whether
//! a typed converter existed for the function's declared `error_type`. This file proves the fix:
//! `RustBindingConfig::error_converters` threads the same converter-name list `gen_bindings/mod.rs`
//! already builds into the shared function generator, and `resolve_err_conv` looks it up before
//! falling back. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FunctionDef, ParamDef, PrimitiveType, TypeRef};

const ERROR_NAME: &str = "ParsingError";
const ERROR_VARIANT: &str = "InvalidInput";
const TYPED_FUNCTION_NAME: &str = "parse_document";
const UNKNOWN_ERROR_FUNCTION_NAME: &str = "parse_with_unmapped_error";

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "test_lib"
"#,
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

/// One error type with one unit variant, and two free functions: `TYPED_FUNCTION_NAME` declares
/// `error_type: Some(ERROR_NAME)` (a converter exists), `UNKNOWN_ERROR_FUNCTION_NAME` declares an
/// error type with no matching `ErrorDef` in `api.errors` (no converter exists, and must fall
/// back to the generic conversion rather than referencing an undefined function name).
fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        errors: vec![ErrorDef {
            name: ERROR_NAME.to_string(),
            rust_path: format!("test_lib::{ERROR_NAME}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                error_code: Some(100),
                name: ERROR_VARIANT.to_string(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        functions: vec![
            FunctionDef {
                name: TYPED_FUNCTION_NAME.to_string(),
                rust_path: format!("test_lib::{TYPED_FUNCTION_NAME}"),
                params: vec![ParamDef {
                    name: "data".to_string(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                error_type: Some(ERROR_NAME.to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: UNKNOWN_ERROR_FUNCTION_NAME.to_string(),
                rust_path: format!("test_lib::{UNKNOWN_ERROR_FUNCTION_NAME}"),
                params: vec![ParamDef {
                    name: "data".to_string(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                error_type: Some("UnmappedError".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn lib_rs(config: &ResolvedCrateConfig) -> String {
    let files = crate::backends::pyo3::Pyo3Backend
        .generate_bindings(&surface(), config)
        .expect("binding generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
        .clone()
}

/// REPRODUCTION (alef #452): a plain free function whose declared error type has a matching
/// `{error}_to_py_err` converter must route its `.map_err(...)` through that converter, not
/// through the generic `PyRuntimeError` fallback every error kind used to collapse into.
#[test]
fn plain_free_function_routes_through_the_typed_error_converter() {
    let lib_rs = lib_rs(&config());

    assert!(
        lib_rs.contains("fn parsing_error_to_py_err(e: test_lib::ParsingError) -> pyo3::PyErr"),
        "the fixture must reach the case under test -- the typed converter must be emitted:\n{lib_rs}"
    );

    let fn_start = lib_rs
        .find(&format!("pub fn {TYPED_FUNCTION_NAME}"))
        .unwrap_or_else(|| panic!("{TYPED_FUNCTION_NAME} must be emitted:\n{lib_rs}"));
    let fn_end = lib_rs[fn_start..]
        .find("\n}\n")
        .map(|end| fn_start + end)
        .unwrap_or(lib_rs.len());
    let fn_body = &lib_rs[fn_start..fn_end];

    assert!(
        fn_body.contains(".map_err(parsing_error_to_py_err)"),
        "{TYPED_FUNCTION_NAME} must convert its error through the typed converter:\n{fn_body}"
    );
    assert!(
        !fn_body.contains("PyRuntimeError"),
        "{TYPED_FUNCTION_NAME} must not fall back to the generic untyped exception:\n{fn_body}"
    );
}

/// CONTROL: a free function whose declared error type has NO matching converter (no `ErrorDef`
/// in `api.errors` names it) must keep today's generic conversion -- proving the fallback branch
/// in `resolve_err_conv` still fires, rather than the fix silently emitting a call to an
/// undefined `{name}_to_py_err` function.
#[test]
fn plain_free_function_with_unmapped_error_type_keeps_the_generic_conversion() {
    let lib_rs = lib_rs(&config());

    let fn_start = lib_rs
        .find(&format!("pub fn {UNKNOWN_ERROR_FUNCTION_NAME}"))
        .unwrap_or_else(|| panic!("{UNKNOWN_ERROR_FUNCTION_NAME} must be emitted:\n{lib_rs}"));
    let fn_end = lib_rs[fn_start..]
        .find("\n}\n")
        .map(|end| fn_start + end)
        .unwrap_or(lib_rs.len());
    let fn_body = &lib_rs[fn_start..fn_end];

    assert!(
        fn_body.contains("pyo3::exceptions::PyRuntimeError::new_err(e.to_string())"),
        "with no matching converter, the fallback generic conversion must still be used:\n{fn_body}"
    );
    assert!(
        !fn_body.contains("_to_py_err"),
        "must not reference an undefined converter function:\n{fn_body}"
    );
}
