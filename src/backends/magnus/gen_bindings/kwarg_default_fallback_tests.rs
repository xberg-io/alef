//! A kwargs constructor's `None => Default::default()` fallback resolves against the *generated*
//! re-declaration of the field's type, not the core one.
//!
//! A data enum whose core `impl Default` carries `#[cfg_attr(alef, alef(skip))]` reaches the
//! backend with `has_default == false`: `extract_impl_block` drops a skipped impl, so the mirrored
//! enum is emitted without one. Meanwhile the owning struct's `#[derive(Default)]` still seeds
//! `DefaultValue::Empty` onto every one of its fields — including that enum field, which has no
//! default of its own. That pairing put `Default::default()` on a type the generated crate cannot
//! satisfy, an `error[E0277]` per affected field in a downstream consumer's generated crate.
//!
//! These run the REAL `MagnusBackend::generate_bindings` path rather than calling
//! `gen_magnus_kwargs_constructor` directly, because the defect is as much in *which* set of
//! default-bearing type names the backend hands the constructor as in the constructor's own guard.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn magnus_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.ruby]\ngem_name = \"test_lib\"\n",
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

/// Every field carries `DefaultValue::Empty`: that is exactly what `extract::extractor::types`
/// seeds onto each field of a `#[derive(Default)]` struct, and it is the seeding — not any
/// per-field serde attribute — that routes these fields into the `use_unwrap_or_default` arm. ~keep
fn derived_default_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    }
}

/// `EmbeddingInput`-shaped: an untagged enum with data variants and `has_default: false`, the state
/// `alef(skip)` on the core `impl Default` leaves behind. `enum_magnus.rs.jinja` gates a data
/// enum's `impl Default` on `has_default`, so this one is emitted without it.
fn undefaultable_untagged_enum() -> EnumDef {
    EnumDef {
        name: "EmbeddingInput".to_string(),
        rust_path: "test_lib::EmbeddingInput".to_string(),
        serde_untagged: true,
        has_default: false,
        variants: vec![
            EnumVariant {
                name: "Single".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Multiple".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `EmbeddingFormat`-shaped: a unit enum, for which `enum_magnus.rs.jinja` emits an
/// *unconditional* `impl Default`. This is the control that keeps the guard a per-type lookup
/// instead of a blanket refusal to default any `Named` field.
fn defaultable_unit_enum() -> EnumDef {
    EnumDef {
        name: "EmbeddingFormat".to_string(),
        rust_path: "test_lib::EmbeddingFormat".to_string(),
        has_default: false,
        variants: vec![
            EnumVariant {
                name: "Float".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Base64".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn embedding_request_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![undefaultable_untagged_enum(), defaultable_unit_enum()],
        types: vec![TypeDef {
            name: "EmbeddingRequest".to_string(),
            rust_path: "test_lib::EmbeddingRequest".to_string(),
            has_default: true,
            has_serde: true,
            is_clone: true,
            fields: vec![
                derived_default_field("model", TypeRef::String),
                derived_default_field("input", TypeRef::Named("EmbeddingInput".to_string())),
                derived_default_field("encoding_format", TypeRef::Named("EmbeddingFormat".to_string())),
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated_lib_rs() -> String {
    let files = MagnusBackend
        .generate_bindings(&embedding_request_api(), &magnus_config())
        .expect("magnus bindings generate");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
        .clone()
}

/// The kwargs constructor emits one `match kwargs.get(ruby.to_symbol("<field>"))` per field; this
/// returns that field's arm alone so an assertion cannot accidentally read a sibling field's. ~keep
fn field_initializer<'a>(lib_rs: &'a str, field: &str) -> &'a str {
    let needle = format!("match kwargs.get(ruby.to_symbol(\"{field}\"))");
    let start = lib_rs
        .find(&needle)
        .unwrap_or_else(|| panic!("the kwargs constructor must extract `{field}`:\n{lib_rs}"));
    let end = lib_rs[start..]
        .find("},\n")
        .map(|i| start + i + 1)
        .unwrap_or_else(|| panic!("`{field}`'s match arm must close:\n{}", &lib_rs[start..]));
    &lib_rs[start..end]
}

/// Guards the premise of every other test here: the generated crate really does re-declare
/// `EmbeddingInput` without a `Default` impl. If alef ever started emitting one, the other
/// assertions would pass for the wrong reason. ~keep
#[test]
fn generated_untagged_enum_has_no_default_impl() {
    let lib_rs = generated_lib_rs();

    assert!(
        lib_rs.contains("pub enum EmbeddingInput {"),
        "the generated crate must re-declare the untagged enum:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("impl Default for EmbeddingInput"),
        "fixture premise broken: the generated crate now has a Default impl for EmbeddingInput, \
         so `Default::default()` on it would compile and this module tests nothing:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("impl Default for EmbeddingFormat"),
        "fixture premise broken: the unit-enum control must have a Default impl, or the positive \
         control below proves nothing:\n{lib_rs}"
    );
}

/// The fix: a `Named` field whose generated type has no `Default` impl must raise a Ruby
/// `ArgumentError` naming the missing keyword, never fabricate a value. Reverting the
/// `!named_lacks_generated_default` guard in `codegen::config_gen::magnus` puts
/// `None => Default::default()` back here and fails both assertions.
#[test]
fn undefaultable_untagged_enum_field_raises_instead_of_defaulting() {
    let lib_rs = generated_lib_rs();
    let input = field_initializer(&lib_rs, "input");

    assert!(
        !input.contains("Default::default()"),
        "`input`'s type has no generated Default impl, so this fallback is an E0277:\n{input}"
    );
    assert!(
        input.contains("exception_arg_error()") && input.contains("missing required field: input"),
        "an absent keyword for a genuinely required field must raise a Ruby ArgumentError naming \
         it:\n{input}"
    );
}

/// Negative control #1 — a non-`Named` field. `String`'s `Default` is structural, never generated,
/// so the guard must not touch it. This is what keeps the overwhelming majority of
/// `None => Default::default()` sites in a real generated crate compiling unchanged; an over-broad
/// guard that keyed on `use_unwrap_or_default` alone would turn `model` into a required keyword and
/// fail here.
#[test]
fn string_field_still_defaults() {
    let lib_rs = generated_lib_rs();
    let model = field_initializer(&lib_rs, "model");

    assert!(
        model.contains("None => Default::default()"),
        "a field whose Default is structural must keep its fallback:\n{model}"
    );
    assert!(
        !model.contains("missing required field"),
        "`model` must not become a required keyword:\n{model}"
    );
}

/// Negative control #2 — a `Named` field whose type *does* get a generated `Default` impl. Proves
/// the guard is a per-type membership test and not "never default a `Named` field": widening it to
/// every `Named` type, or handing the constructor a set built only from `api.types` (which omits
/// enums entirely), turns this optional Ruby keyword into a required one and fails here.
#[test]
fn defaultable_unit_enum_field_still_defaults() {
    let lib_rs = generated_lib_rs();
    let encoding_format = field_initializer(&lib_rs, "encoding_format");

    assert!(
        encoding_format.contains("None => Default::default()"),
        "a Named field whose generated type has a Default impl must keep its fallback:\n{encoding_format}"
    );
    assert!(
        !encoding_format.contains("missing required field"),
        "`encoding_format` must not become a required keyword:\n{encoding_format}"
    );
}
