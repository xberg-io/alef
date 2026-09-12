//! Regression coverage for the dataclass-twin capability-loss defect: `options.py`'s public
//! `@dataclass` twin of a native `#[pyclass]` must not lose native-only capability (starting
//! with `from_json`) just because its public name shadows the native class. Every assertion
//! here fails against the pre-fix code, where `gen_options_py` never consulted
//! `type_has_from_json` at all and a consumer's `from pkg import Widget` gave a class
//! with no `from_json`.

use super::types::gen_options_py;
use crate::core::config::DtoConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

const WIDGET: &str = "Widget";
const MODULE_NAME: &str = "_rust";

/// A `has_default`, `has_serde` type with one plain field -- the minimal shape that makes it
/// both a dataclass twin (`options_dataclass_type_names`) and `from_json`-eligible
/// (`type_has_from_json`).
fn widget_api(type_has_serde: bool) -> ApiSurface {
    ApiSurface {
        types: vec![TypeDef {
            name: WIDGET.to_owned(),
            rust_path: format!("sample_core::{WIDGET}"),
            has_default: true,
            has_serde: type_has_serde,
            fields: vec![FieldDef {
                name: "label".to_owned(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

/// The headline fix: a `from_json`-eligible dataclass twin gets a `from_json` staticmethod that
/// delegates to the native class and lifts the result back into the dataclass via the existing
/// `_from_native_*` converter -- so `from pkg import Widget; Widget.from_json(...)` works
/// exactly like `from pkg._native import Widget; Widget.from_json(...)` does today.
#[test]
fn gen_options_py_emits_from_json_delegating_to_native_class() {
    let api = widget_api(true);
    let options_py = gen_options_py(&api, MODULE_NAME, &DtoConfig::default(), &[], true);

    assert!(
        options_py.contains("def from_json(json_str: str) -> Widget:"),
        "future annotations require an unquoted Widget return annotation:\n{options_py}"
    );
    assert!(
        options_py.contains(&format!(
            "_from_native_widget({MODULE_NAME}.Widget.from_json(json_str))"
        )),
        "from_json must call the native class's from_json and lift the result back through \
         the existing _from_native_widget converter, not construct Widget directly:\n{options_py}"
    );
    assert!(
        options_py.contains(&format!("from . import {MODULE_NAME}\n")),
        "the native module must be imported by name so from_json can reach {MODULE_NAME}.Widget:\n{options_py}"
    );
}

#[test]
fn gen_options_py_from_json_docstring_uses_an_before_a_vowel_class_name() {
    let mut api = widget_api(true);
    api.types[0].name = "AccelerationConfig".to_owned();
    api.types[0].rust_path = "sample_core::AccelerationConfig".to_owned();
    let options_py = gen_options_py(&api, MODULE_NAME, &DtoConfig::default(), &[], true);

    assert!(
        options_py.contains("Build an AccelerationConfig from a JSON string, via the native binding."),
        "a vowel-starting class name must take the article an:\n{options_py}"
    );
    assert!(
        options_py.contains("def from_json(json_str: str) -> AccelerationConfig:"),
        "the generated return annotation must remain unquoted:\n{options_py}"
    );
    let consonant = gen_options_py(&widget_api(true), MODULE_NAME, &DtoConfig::default(), &[], true);
    assert!(
        consonant.contains("Build a Widget from a JSON string, via the native binding."),
        "a consonant-starting class name must retain the article a:\n{consonant}"
    );
}

/// Negative control: a type with NO core `Deserialize` (`has_serde: false` on the `TypeDef`
/// itself) must NOT get a `from_json` delegate, even though the crate as a whole has serde
/// available -- `type_has_from_json` requires both. Proves the from_json gate is a real
/// condition, not an unconditional emission.
#[test]
fn gen_options_py_omits_from_json_when_type_has_no_serde() {
    let api = widget_api(false);
    let options_py = gen_options_py(&api, MODULE_NAME, &DtoConfig::default(), &[], true);

    assert!(
        !options_py.contains("def from_json("),
        "a type without core Deserialize must not get a from_json delegate:\n{options_py}"
    );
    assert!(
        !options_py.contains(&format!("from . import {MODULE_NAME}\n")),
        "no from_json delegate anywhere in the file means the native-module-alias import must \
         not be emitted either -- it would be dead code:\n{options_py}"
    );
}

/// Negative control: even a `from_json`-eligible type gets no delegate when the crate itself has
/// no serde support (`has_serde: false` passed to `gen_options_py`). Proves the crate-level half
/// of the gate is consulted too, matching the native `#[pymethods]` injection's own condition.
#[test]
fn gen_options_py_omits_from_json_when_crate_has_no_serde() {
    let api = widget_api(true);
    let options_py = gen_options_py(&api, MODULE_NAME, &DtoConfig::default(), &[], false);

    assert!(
        !options_py.contains("def from_json("),
        "no from_json delegate anywhere when the crate has no serde support:\n{options_py}"
    );
}
