//! End-to-end regression coverage for the Magnus (Ruby) struct-declaration half of the
//! `candle_ocr`/Windows defect: a HOST-owned `TypeDef::cfg` (a struct gated behind a Cargo
//! feature the core crate itself declares, e.g. `#[cfg(feature = "candle-ocr")]` on a
//! candle-backend options struct) was never re-emitted onto the generated Ruby wrapper at all.
//! Every function and method already carries its own `cfg` forward via `prepend_cfg`
//! (`func.cfg`/`method.cfg` in `gen_bindings::mod`); the two loops over `api.types` that emit a
//! struct's own declaration and its `From` conversions never consulted `typ.cfg` the same way,
//! so a consumer whose own feature set disabled the gate still got an unconditional reference to
//! a type the core crate never compiled in -- 41 ungated `<core>::candle_ocr::*` references on
//! Ruby/Windows, the sole failure in a downstream crate's Publish Release dry run.
//!
//! `MagnusBackend::generate_bindings` is exercised end to end (not `classes::gen_struct` /
//! `classes::gen_from_binding_to_core_filtered` directly), since the defect was in the call
//! sites in `gen_bindings::mod`, not in those generators themselves.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn magnus_config() -> ResolvedCrateConfig {
    let toml_src = "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\n\
                     sources = [\"src/lib.rs\"]\n[crates.ruby]\ngem_name = \"test_lib\"\n";
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A HOST-owned (`rust_path` shares the crate's own `core_import`, "test_lib") struct gated
/// behind a Cargo feature, with a function taking it as a parameter and another returning it --
/// exercising both the binding->core and core->binding conversion loops, not just the
/// declaration. ~keep
fn gated_struct_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GatedOptions".to_string(),
            rust_path: "test_lib::GatedOptions".to_string(),
            cfg: Some(r#"feature = "candle-ocr""#.to_string()),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            is_clone: true,
            ..Default::default()
        }],
        functions: vec![
            FunctionDef {
                name: "make_options".to_string(),
                rust_path: "test_lib::make_options".to_string(),
                return_type: TypeRef::Named("GatedOptions".to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "use_options".to_string(),
                rust_path: "test_lib::use_options".to_string(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("GatedOptions".to_string()),
                    ..Default::default()
                }],
                return_type: TypeRef::Unit,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

#[test]
fn generate_bindings_gates_struct_declaration_behind_its_own_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\n#[derive(Clone"),
        "a HOST-owned type's own `#[cfg(...)]` must be re-emitted directly above its generated \
         Ruby struct declaration, or a consumer whose feature set disables the gate still \
         references a type the core crate never compiled in, got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_gates_binding_to_core_conversion_behind_the_type_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<GatedOptions> for test_lib::GatedOptions"),
        "the binding->core `From` impl for a cfg-gated type must carry the same `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_gates_core_to_binding_conversion_behind_the_type_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<test_lib::GatedOptions> for GatedOptions"),
        "the core->binding `From` impl for a cfg-gated type must carry the same `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

/// Positive control: an ungated type must never pick up a stray `#[cfg(...)]` -- proves the fix
/// reads `typ.cfg` per-type rather than gating every struct unconditionally.
#[test]
fn generate_bindings_never_gates_a_struct_with_no_cfg() {
    let mut api = gated_struct_api();
    api.types[0].cfg = None;
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("#[cfg("),
        "an ungated struct must not be wrapped in a `#[cfg(...)]` attribute, got:\n{lib_rs}"
    );
}
