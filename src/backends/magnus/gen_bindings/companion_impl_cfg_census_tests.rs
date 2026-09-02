//! Structural census regression for the `candle_ocr`/Windows defect's second half: `typ.cfg` was
//! re-emitted onto a cfg-gated type's own declaration (see `type_cfg_gate_tests`), but
//! `struct_def.rs.jinja`/`opaque_struct.rs.jinja` render the declaration together with its
//! companion impls (`IntoValueFromNative`, `magnus::TryConvert`, `TryConvertOwned`, and a
//! delegating `Deserialize`) as ONE multi-item string, and a `#[cfg(...)]` attribute only binds
//! to the single item immediately following it. Prepending `#[cfg(...)]` once to the front of
//! that whole blob therefore gated only the struct declaration -- xberg's generated
//! `packages/ruby/ext/xberg_rb/src/lib.rs` carried 67 gated type declarations and 259 ungated
//! companion impls, the sole cause of 827 compile errors and the only failing job
//! (`Build Ruby gem (windows-x86_64)`) in xberg's publish run.
//!
//! This is a CENSUS, not a spot check: it parses the real generated `lib.rs` with `syn` and
//! asserts, over every `impl` item in the file, that none of them targets a type whose
//! declaration carries `#[cfg(...)]` while the impl itself does not.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

fn magnus_config() -> ResolvedCrateConfig {
    let toml_src = "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\n\
                     sources = [\"src/lib.rs\"]\n[crates.ruby]\ngem_name = \"test_lib\"\n";
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// One gated (host-owned) struct and one ungated struct, each with a field and a function that
/// both consumes and returns it, so both directions of `From` and both structs' companion impls
/// are actually emitted this run -- a census over a file with only one shape would not prove the
/// gate is applied selectively rather than universally (see the positive control in
/// `type_cfg_gate_tests::generate_bindings_never_gates_a_struct_with_no_cfg`).
fn mixed_gating_api() -> ApiSurface {
    let make_type = |name: &str, cfg: Option<&str>| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        cfg: cfg.map(str::to_string),
        fields: vec![FieldDef {
            name: "value".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        is_clone: true,
        ..Default::default()
    };
    let make_fns = |type_name: &str| {
        vec![
            FunctionDef {
                name: format!("make_{}", type_name.to_lowercase()),
                rust_path: format!("test_lib::make_{}", type_name.to_lowercase()),
                return_type: TypeRef::Named(type_name.to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: format!("use_{}", type_name.to_lowercase()),
                rust_path: format!("test_lib::use_{}", type_name.to_lowercase()),
                params: vec![ParamDef {
                    name: "value".to_string(),
                    ty: TypeRef::Named(type_name.to_string()),
                    ..Default::default()
                }],
                return_type: TypeRef::Unit,
                ..Default::default()
            },
        ]
    };

    let mut functions = make_fns("GatedOptions");
    functions.extend(make_fns("PlainOptions"));

    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            make_type("GatedOptions", Some(r#"feature = "candle-ocr""#)),
            make_type("PlainOptions", None),
        ],
        functions,
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

/// The type names (struct or enum) whose own declaration item carries a `#[cfg(...)]` attribute.
fn cfg_gated_type_names(file: &syn::File) -> HashSet<String> {
    file.items
        .iter()
        .filter_map(|item| {
            let (ident, attrs) = match item {
                syn::Item::Struct(s) => (&s.ident, &s.attrs),
                syn::Item::Enum(e) => (&e.ident, &e.attrs),
                _ => return None,
            };
            attrs
                .iter()
                .any(|a| a.path().is_ident("cfg"))
                .then(|| ident.to_string())
        })
        .collect()
}

/// The bare type name an `impl` block targets, e.g. `Point` for both `impl Point` and
/// `impl magnus::TryConvert for Point` and `impl<'de> serde::Deserialize<'de> for Point`.
fn impl_self_type_name(item_impl: &syn::ItemImpl) -> Option<String> {
    match item_impl.self_ty.as_ref() {
        syn::Type::Path(type_path) => type_path.path.segments.last().map(|seg| seg.ident.to_string()),
        _ => None,
    }
}

/// Every `impl` item in the file, paired with the type it targets and whether the impl itself
/// carries a `#[cfg(...)]` attribute.
fn impl_cfg_census(file: &syn::File) -> Vec<(String, bool)> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item_impl) => {
                let type_name = impl_self_type_name(item_impl)?;
                let is_gated = item_impl.attrs.iter().any(|a| a.path().is_ident("cfg"));
                Some((type_name, is_gated))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn no_ungated_companion_impl_exists_for_a_cfg_gated_type() {
    let api = mixed_gating_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let file = syn::parse_file(lib_rs).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{lib_rs}"));

    let gated_types = cfg_gated_type_names(&file);
    assert!(
        gated_types.contains("GatedOptions"),
        "fixture must actually produce a cfg-gated declaration to exercise this census, got gated set {gated_types:?} in:\n{lib_rs}"
    );

    let census = impl_cfg_census(&file);
    let offenders: Vec<&(String, bool)> = census
        .iter()
        .filter(|(type_name, is_gated)| gated_types.contains(type_name) && !is_gated)
        .collect();

    assert_eq!(
        offenders.len(),
        0,
        "every companion impl of a cfg-gated type must carry the same #[cfg(...)], but found {} \
         ungated impl(s) targeting a gated type ({:?}), got:\n{lib_rs}",
        offenders.len(),
        offenders
    );

    // Positive control: `PlainOptions` is ungated, so its companion impls (and the file overall)
    // must still contain at least one impl targeting it -- proves the census walked real impls,
    // not an empty item list.
    let plain_impls = census
        .iter()
        .filter(|(type_name, _)| type_name == "PlainOptions")
        .count();
    assert!(
        plain_impls > 0,
        "fixture must also produce impls for the ungated type to prove the census inspects real \
         impl items, got:\n{lib_rs}"
    );
}
