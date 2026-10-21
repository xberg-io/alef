//! An optional string argument of a generated Ruby function raises `TypeError` for a value that is
//! neither nil nor a string. It never turns into an empty string.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn magnus_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.ruby]\ngem_name = \"test_lib\"\n",
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

fn lib_rs_for_optional_string_function() -> String {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![FunctionDef {
            name: "list_models".to_string(),
            rust_path: "test_lib::list_models".to_string(),
            original_rust_path: "test_lib::list_models".to_string(),
            params: vec![ParamDef {
                name: "provider".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    };
    MagnusBackend
        .generate_bindings(&api, &magnus_config())
        .expect("magnus bindings generate")
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
        .clone()
}

/// Positive control: the optional string argument is read through the nil check at all. If the
/// shape of the generated line changes, the next test would pass for the wrong reason. ~keep
#[test]
fn optional_string_argument_is_read_with_a_nil_check() {
    let lib_rs = lib_rs_for_optional_string_function();
    assert!(
        lib_rs.contains("let provider: Option<String>") && lib_rs.contains("is_nil()"),
        "the optional string argument must be converted from a Ruby value that may be nil:\n{lib_rs}"
    );
}

#[test]
fn wrong_type_optional_string_argument_raises_instead_of_becoming_empty() {
    let lib_rs = lib_rs_for_optional_string_function();
    assert!(
        !lib_rs.contains("String::try_convert(v).unwrap_or_default()"),
        "a failed string conversion must not be replaced with an empty string:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("String::try_convert(v)?"),
        "a failed string conversion must raise the conversion error:\n{lib_rs}"
    );
}
