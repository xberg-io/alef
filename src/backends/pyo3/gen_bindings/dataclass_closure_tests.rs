//! Regression coverage for the nested-config public-identity defect: a native type with NO
//! core `Default` impl of its own (because one of its
//! fields is genuinely required, e.g. `CaptioningConfig { llm: LlmConfig, .. }`) but whose
//! required field's type DOES get a public `options.py` dataclass twin (because that field's
//! type, `LlmConfig`, has a core `Default` impl) must itself join the dataclass twin set --
//! otherwise its native `#[new]` demands a native `LlmConfig` instance while the public name
//! `LlmConfig` resolves to the unrelated dataclass, and `CaptioningConfig(llm=LlmConfig(...))`
//! raises `TypeError: 'LlmConfig' object is not an instance of 'LlmConfig'`.
//!
//! Every assertion here fails against the pre-fix code, where `options_dataclass_type_names`
//! (and the three independent `typ.has_default` gates in `gen_options_py`/`gen_init_py` this
//! test also exercises) considered only `has_default`, never a type's *closure* over a required
//! field of an already-dataclass-backed type.

use super::errors::gen_init_py;
use super::types::{gen_options_py, options_dataclass_type_names};
use crate::core::config::DtoConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

const LLM_CONFIG: &str = "LlmConfig";
const CAPTIONING_CONFIG: &str = "CaptioningConfig";

/// `LlmConfig`: has a core `Default` impl (the seed of the dataclass set) and one plain field.
/// `CaptioningConfig`: no core `Default` impl (mirrors the real type -- `llm` has no sensible
/// default), a required `llm: LlmConfig` field, and an optional `prompt: Option<String>` field
/// that DOES have a synthesizable default -- so the fix's "required fields with no default sort
/// first" reordering has something to actually reorder against.
fn captioning_config_api() -> ApiSurface {
    ApiSurface {
        types: vec![
            TypeDef {
                name: LLM_CONFIG.to_owned(),
                rust_path: format!("sample_core::{LLM_CONFIG}"),
                has_default: true,
                fields: vec![FieldDef {
                    name: "model".to_owned(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: CAPTIONING_CONFIG.to_owned(),
                rust_path: format!("sample_core::{CAPTIONING_CONFIG}"),
                has_default: false,
                fields: vec![
                    FieldDef {
                        name: "llm".to_owned(),
                        ty: TypeRef::Named(LLM_CONFIG.to_owned()),
                        optional: false,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "prompt".to_owned(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        optional: true,
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

/// `options_dataclass_type_names` must include the closure-added type, not just the
/// `has_default` seed -- this is the single source of truth every other emitter in this module
/// (and `e2e::codegen::python`) consults to decide a type's public spelling.
#[test]
fn options_dataclass_type_names_includes_a_required_field_of_a_dataclass_type() {
    let api = captioning_config_api();
    let names = options_dataclass_type_names(&api, &[]);
    assert!(names.contains(LLM_CONFIG), "the has_default seed must still be present");
    assert!(
        names.contains(CAPTIONING_CONFIG),
        "a native type with no Default of its own, but a required field whose type IS in the \
         dataclass set, must join the set too -- otherwise its constructor demands a native \
         instance of a type whose public name now resolves to the dataclass twin"
    );
}

/// `options.py` must actually render a `CaptioningConfig` dataclass body (not just report it in
/// the name set) -- and its genuinely required `llm` field must be emitted with NO Python
/// default (a real Rust `Default::default()` for `LlmConfig` was never asked for, so fabricating
/// one and letting `CaptioningConfig()` silently omit `llm` would be a second, quieter defect).
#[test]
fn gen_options_py_emits_captioning_config_with_a_required_llm_field() {
    let api = captioning_config_api();
    let options_py = gen_options_py(&api, "_rust", &DtoConfig::default(), &[]);

    assert!(
        options_py.contains("class CaptioningConfig:"),
        "options.py must define the CaptioningConfig dataclass:\n{options_py}"
    );
    assert!(
        options_py.contains("    llm: LlmConfig\n"),
        "the required `llm` field must have no default (bare `name: Type`, no `= ...`):\n{options_py}"
    );
    assert!(
        !options_py.contains("llm: LlmConfig ="),
        "the required `llm` field must not be given a fabricated default:\n{options_py}"
    );
    assert!(
        !options_py.contains("llm: LlmConfig | None"),
        "the required `llm` field must not be widened to Optional:\n{options_py}"
    );
}

/// `__init__.py` must route `CaptioningConfig` to `.options` (its public spelling is the
/// dataclass), not to the native extension module -- this is the actual surface the reported
/// `pkg.CaptioningConfig is pkg._native.CaptioningConfig -> True` bug lived on.
#[test]
fn gen_init_py_routes_captioning_config_to_options_not_native() {
    let api = captioning_config_api();
    let init_py = gen_init_py(
        &api,
        "_rust",
        "0.0.0",
        &DtoConfig::default(),
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        &std::collections::HashMap::new(),
        &[],
        &std::collections::HashMap::new(),
        &ahash::AHashSet::new(),
    );

    // The `.options` import renders either as one line (`from .options import A, B`) or, once
    // the joined names are long enough, as a parenthesized multi-line block -- read from the
    // opening line up to the next blank line so both shapes are covered.
    let options_import_start = init_py
        .find("from .options import")
        .unwrap_or_else(|| panic!("expected a `from .options import ...` statement:\n{init_py}"));
    let options_import_block = init_py[options_import_start..]
        .split("\n\n")
        .next()
        .unwrap_or(&init_py[options_import_start..]);
    assert!(
        options_import_block.contains(CAPTIONING_CONFIG),
        "CaptioningConfig must be imported from .options: {options_import_block}"
    );

    if let Some(native_import_start) = init_py.find("from ._rust import") {
        let native_import_block = init_py[native_import_start..]
            .split("\n\n")
            .next()
            .unwrap_or(&init_py[native_import_start..]);
        assert!(
            !native_import_block.contains(CAPTIONING_CONFIG),
            "CaptioningConfig must NOT also be imported from the native module: {native_import_block}"
        );
    }
}
