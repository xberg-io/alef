//! Regression coverage for the optional-converter-result defect: a bare `#[serde(default)]`
//! marker on a `Named`-typed field (`field.default == Some("/* serde(default) */")`,
//! `typed_default` unset) does NOT mean the *native* `#[new]` constructor grants that field a
//! default. `constructors::should_option_for_nested_default` requires the PARENT type's own
//! `has_default` as its very first condition, so on a closure-only type (`!typ.has_default`) the
//! field is always required at the native layer regardless of the marker -- only `field.optional`
//! (a real `Option<T>` in Rust) makes it omittable there.
//!
//! The pre-fix `options.py` emitter ignored the parent's `has_default` and fabricated `None` as
//! this field's public default purely because no Python literal was spellable for it, widening
//! its type hint to `T | None`. That is the real `OcrPipelineConfig::quality_thresholds` shape:
//! `stages` (required, no marker) plus a `quality_thresholds: OcrQualityThresholds` field with a
//! bare `#[serde(default)]`, on a struct with no `Default` impl of its own. The mismatch is what
//! made the paired `_to_rust_*` converter's `T | None` inferred return look valid for a
//! `None`-typed field while the native constructor still demanded a plain `T`. Every assertion
//! here fails against the pre-fix code.

use super::types::gen_options_py;
use crate::core::config::DtoConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

/// `Thresholds`: has a core `Default` impl (mirrors `OcrQualityThresholds`) -- present so
/// `Pipeline` (below) has a `default_types` member to defer to and so its own field, in the
/// CONTROL case, is a real seed of the dataclass set.
const THRESHOLDS: &str = "Thresholds";
/// `Pipeline`: no core `Default` impl of its own (mirrors `OcrPipelineConfig`) -- `stages` is a
/// genuinely required field with no default anywhere, `thresholds` carries a bare
/// `#[serde(default)]` marker but the struct's lack of `Default` means the native constructor
/// still requires it.
const PIPELINE: &str = "Pipeline";
/// `DefaultedPipeline`: same shape as `Pipeline` but WITH a core `Default` impl -- the control
/// proving a bare `#[serde(default)]` field still renders its existing `None` fallback when the
/// parent type genuinely has one.
const DEFAULTED_PIPELINE: &str = "DefaultedPipeline";

fn bare_serde_default_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_owned(),
        ty,
        optional: false,
        default: Some("/* serde(default) */".to_owned()),
        typed_default: None,
        ..FieldDef::default()
    }
}

fn api_surface() -> ApiSurface {
    let thresholds = TypeDef {
        name: THRESHOLDS.to_owned(),
        rust_path: format!("sample_core::{THRESHOLDS}"),
        has_default: true,
        fields: vec![FieldDef {
            name: "min_quality".to_owned(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let pipeline = TypeDef {
        name: PIPELINE.to_owned(),
        rust_path: format!("sample_core::{PIPELINE}"),
        has_default: false,
        fields: vec![
            FieldDef {
                name: "stages".to_owned(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
                optional: false,
                ..FieldDef::default()
            },
            bare_serde_default_field("thresholds", TypeRef::Named(THRESHOLDS.to_owned())),
        ],
        ..TypeDef::default()
    };
    let defaulted_pipeline = TypeDef {
        name: DEFAULTED_PIPELINE.to_owned(),
        rust_path: format!("sample_core::{DEFAULTED_PIPELINE}"),
        has_default: true,
        fields: vec![bare_serde_default_field(
            "thresholds",
            TypeRef::Named(THRESHOLDS.to_owned()),
        )],
        ..TypeDef::default()
    };

    ApiSurface {
        types: vec![thresholds, pipeline, defaulted_pipeline],
        ..ApiSurface::default()
    }
}

fn render() -> String {
    gen_options_py(&api_surface(), "_internal_bindings", &DtoConfig::default(), &[])
}

/// A bare `#[serde(default)]` field on a closure-only type (no core `Default`) must render as
/// required -- no `| None`, no `= None` -- because the native constructor never grants it a
/// default (`should_option_for_nested_default` needs `typ.has_default`, which is false here).
#[test]
fn bare_serde_default_field_on_closure_only_type_is_required() {
    let options_py = render();
    assert!(
        options_py.contains("class Pipeline:"),
        "options.py must define the Pipeline dataclass:\n{options_py}"
    );
    // ~keep Scoped to the Pipeline block, not the whole file. This same render also emits the
    // DefaultedPipeline control, whose `thresholds: Thresholds | None = None` is CORRECT -- a
    // file-wide negative assertion matches that instead and fails on the very case it exists to
    // leave alone.
    let pipeline_block = {
        let start = options_py
            .find("class Pipeline:")
            .expect("Pipeline dataclass must be rendered");
        let rest = &options_py[start..];
        let end = rest[1..].find("@dataclass").map_or(rest.len(), |offset| offset + 1);
        &rest[..end]
    };
    assert!(
        pipeline_block.contains("    thresholds: Thresholds\n"),
        "the field must render with no default (bare `name: Type`, no `= ...`):\n{pipeline_block}"
    );
    assert!(
        !pipeline_block.contains("thresholds: Thresholds | None"),
        "the field must not be widened to Optional -- the native constructor requires a real \
         instance:\n{pipeline_block}"
    );
    assert!(
        !pipeline_block.contains("thresholds: Thresholds = None"),
        "the field must not be given a fabricated `None` default:\n{pipeline_block}"
    );
}

/// CONTROL: the identical bare `#[serde(default)]` field, on a type that genuinely has a core
/// `Default` impl, must keep the pre-existing `None`-fallback behaviour -- the native constructor
/// really does grant this field a default in that case (`Self::default().thresholds`), so
/// `options.py` omitting it (via the `None` sentinel) is correct.
#[test]
fn bare_serde_default_field_on_has_default_type_stays_optional() {
    let options_py = render();
    assert!(
        options_py.contains("class DefaultedPipeline:"),
        "options.py must define the DefaultedPipeline dataclass:\n{options_py}"
    );
    assert!(
        options_py.contains("thresholds: Thresholds | None = None"),
        "on a `has_default` parent, the field must keep its `None`-fallback default:\n{options_py}"
    );
}
