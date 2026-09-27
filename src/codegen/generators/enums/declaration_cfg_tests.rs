//! Regression coverage for `gen_enum`'s declaration-side cfg filtering.
//!
//! `gen_enum` is the shared fieldless-enum declaration emitter for BOTH PyO3 and extendr (see
//! `backends::pyo3::gen_bindings::mod::generate_bindings` and
//! `backends::extendr::gen_bindings::mod::generate_bindings`, the only two callers). Before this
//! fix it declared every variant unconditionally, while the `From` impls elsewhere (via
//! `codegen::conversions::enums::gen_enum_from_binding_to_core_cfg`/
//! `gen_enum_from_core_to_binding_cfg`) already dropped a foreign cfg-gated variant's conversion
//! arm whenever this binding's own configured feature set proved it unreachable -- a Python or R
//! caller could construct the wrapper value the compiled dependency can never actually produce.
//! `gen_enum` now asks the same `codegen::conversions::enums::enum_variant_declaration` authority
//! every other Rust-emitting backend's own wrapper declaration already consults, mirroring
//! `backends::rustler::gen_bindings::types::gen_enum`'s own fix for the identical gap.
//!
//! Assertions parse the exact set of declared variant lines (`"    Name = N,\n"`) rather than a
//! bare substring `.contains` check, since a variant name can be a substring of a longer
//! identifier (and `Extra`, for instance, is also a substring of nothing here, but the discipline
//! matters generally).

use super::gen_enum;
use crate::codegen::generators::{AsyncPattern, RustBindingConfig};
use crate::core::ir::{EnumDef, EnumVariant};
use std::collections::BTreeSet;

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn routing_strategy(rust_path: &str) -> EnumDef {
    EnumDef {
        name: "RoutingStrategy".to_string(),
        rust_path: rust_path.to_string(),
        variants: vec![
            unit_variant("Primary", None),
            unit_variant("Secondary", None),
            unit_variant("Extra", Some(r#"feature = "extra-tier""#)),
        ],
        ..Default::default()
    }
}

/// extendr-shaped config: no PyO3 attrs, so the rendered enum carries no `__new__`/string-method
/// block, keeping the parsed declaration lines unambiguous.
fn extendr_style_config(core_import: &'static str) -> RustBindingConfig<'static> {
    RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &[],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "",
        enum_attrs: &[],
        enum_derives: &["Clone", "PartialEq"],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import,
        async_pattern: AsyncPattern::None,
        has_serde: true,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
        source_crate_remaps: &[],
        emit_delegating_default_for_types: None,
        delegate_deserialize_to_core_for_types: None,
        error_converters: None,
    }
}

/// Parse the exact set of variant names declared in the `pub enum { ... }` body -- every line
/// shaped `    Name = N,`, ignoring attribute lines (`#[...]`).
fn declared_variant_names(rendered: &str) -> BTreeSet<String> {
    // ~keep Split on `,` rather than on lines: the renderer emits the whole declaration on one
    // line in some configurations, and a line-based parse then silently yields an EMPTY set --
    // which compares unequal to any expected set, so the test fails for the wrong reason and its
    // message blames the generator. Attributes such as `#[default]` may precede a name inside a
    // fragment, so take the text after the last `]`.
    rendered
        .split(',')
        .filter_map(|fragment| {
            let fragment = fragment.rsplit(']').next()?;
            let (name, rest) = fragment.trim().split_once(" = ")?;
            let name = name.trim();
            let rest: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '}').collect();
            if name.is_empty() || rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// The reported gap: a FOREIGN enum with one cfg-excluded variant and two retained ones. The
/// excluded variant must be entirely absent from the declared set; the positive control (same
/// shape, feature configured) proves the drop is conditional on the proof, not a blanket
/// foreign-owned rule.
#[test]
fn foreign_variant_proven_unreachable_dropped_declared_set_exact_control_kept_when_active() {
    let cfg = extendr_style_config("mylib");

    let excluded = gen_enum(&routing_strategy("dep_crate::RoutingStrategy"), &cfg, Some(&[]));
    assert_eq!(
        declared_variant_names(&excluded),
        set(&["Primary", "Secondary"]),
        "the declared set must be exactly the two retained variants, got:\n{excluded}"
    );

    let active_features = vec!["extra-tier".to_string()];
    let active = gen_enum(
        &routing_strategy("dep_crate::RoutingStrategy"),
        &cfg,
        Some(&active_features),
    );
    assert_eq!(
        declared_variant_names(&active),
        set(&["Primary", "Secondary", "Extra"]),
        "with \"extra-tier\" configured, the declared set must include the retained foreign \
         variant, got:\n{active}"
    );
}

/// Host-owned enums are unaffected: `enum_variant_declaration` never resolves a host-owned gate
/// to `Drop`, so the declaration keeps a host-owned cfg-gated variant regardless of
/// `configured_features` -- matching every other backend's declaration surface.
#[test]
fn host_owned_cfg_variant_is_never_dropped_from_declared_set() {
    let cfg = extendr_style_config("mylib");
    let out = gen_enum(&routing_strategy("mylib::RoutingStrategy"), &cfg, Some(&[]));
    assert_eq!(
        declared_variant_names(&out),
        set(&["Primary", "Secondary", "Extra"]),
        "a host-owned cfg-gated variant must stay declared even with no features configured, \
         got:\n{out}"
    );
}

/// Edge case: the variant marked `#[default]` is itself the one a foreign cfg proves
/// unreachable. `impl Default` (via the `#[default]` attribute on one declared variant) must fall
/// back to another declared variant instead of marking a variant the enum no longer declares,
/// which would not compile.
#[test]
fn default_variant_selection_skips_a_variant_dropped_from_the_declaration() {
    let cfg = extendr_style_config("mylib");
    let en = EnumDef {
        name: "SyncMode".to_string(),
        rust_path: "dep_crate::SyncMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Testkit".to_string(),
                is_default: true,
                cfg: Some(r#"feature = "testkit""#.to_string()),
                ..Default::default()
            },
            unit_variant("Manual", None),
        ],
        ..Default::default()
    };

    let out = gen_enum(&en, &cfg, Some(&[]));
    assert_eq!(
        declared_variant_names(&out),
        set(&["Manual"]),
        "the dropped default variant must not appear in the declared set, got:\n{out}"
    );
    // ~keep Pin the fact -- `#[default]` names a variant the declaration kept -- not the
    // whitespace between them. This renderer emits the declaration on one line, so an assertion
    // spelling `#[default]\n    Manual` fails on formatting while the generator is correct.
    let after_default = out
        .split_once("#[default]")
        .map(|(_, tail)| tail.trim_start().to_string())
        .expect("the declaration must carry a #[default] attribute");
    assert!(
        after_default.starts_with("Manual"),
        "#[default] must fall back to a variant the declaration actually keeps, but it precedes \
         {after_default:?} in:\n{out}"
    );
}

/// Negative control: an ungated enum's declared set is unaffected by this fix regardless of
/// `configured_features` -- the common case for both PyO3 and extendr, and the shape most at risk
/// of an accidental regression from touching a function shared by both backends.
#[test]
fn ungated_enum_declared_set_unaffected_by_configured_features() {
    let cfg = extendr_style_config("mylib");
    let en = EnumDef {
        name: "Simple".to_string(),
        rust_path: "mylib::Simple".to_string(),
        variants: vec![unit_variant("On", None), unit_variant("Off", None)],
        ..Default::default()
    };

    let no_features = gen_enum(&en, &cfg, None);
    let with_features = gen_enum(&en, &cfg, Some(&["unrelated".to_string()]));
    assert_eq!(declared_variant_names(&no_features), set(&["On", "Off"]));
    assert_eq!(declared_variant_names(&with_features), set(&["On", "Off"]));
    assert_eq!(
        no_features, with_features,
        "an ungated enum's declaration must be byte-identical regardless of configured_features"
    );
}
