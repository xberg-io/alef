use crate::codegen::conversions::ConversionConfig;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{EnumDef, TypeDef, TypeRef};
use ahash::AHashSet;
use minijinja::context;

use super::params::references_named_type;

/// Compute the set of enum-tainted types for which binding->core From CAN be generated.
/// A type is excluded if it references (directly or transitively) an enum with data variants,
/// because data-variant fields may reference types that don't implement Default.
#[allow(dead_code)]
pub(crate) fn gen_convertible_enum_tainted(
    types: &[TypeDef],
    enum_tainted: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> AHashSet<String> {
    let mut unconvertible: AHashSet<String> = AHashSet::new();
    for typ in types {
        if !enum_tainted.contains(&typ.name) {
            continue;
        }
        for field in binding_fields(&typ.fields) {
            if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names)
                && let Some(enum_def) = enums.iter().find(|e| e.name == enum_name)
                && enum_def.variants.iter().any(|v| !v.fields.is_empty())
            {
                unconvertible.insert(typ.name.clone());
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for typ in types {
            if !enum_tainted.contains(&typ.name) || unconvertible.contains(&typ.name) {
                continue;
            }
            if binding_fields(&typ.fields).any(|f| references_named_type(&f.ty, &unconvertible)) {
                unconvertible.insert(typ.name.clone());
                changed = true;
            }
        }
    }
    enum_tainted
        .iter()
        .filter(|name| !unconvertible.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Generate `impl From<BindingType> for core::Type` for enum-tainted types.
/// Enum-Named fields use string->enum parsing (match on variant names, first variant as fallback).
/// Fields referencing other enum-tainted struct types use `.into()` (their own From is also generated).
/// Non-enum fields use the normal conversion with i64 casts.
pub(crate) fn gen_enum_tainted_from_binding_to_core(
    typ: &TypeDef,
    core_import: &str,
    enum_names: &AHashSet<String>,
    _enum_tainted: &AHashSet<String>,
    config: &ConversionConfig,
    enums: &[EnumDef],
    bridge_type_aliases: &AHashSet<String>,
) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);

    if typ.has_lifetime_params
        && let Some(code) = crate::codegen::conversions::gen_from_lifetime_type_constructor(
            typ,
            &core_path,
            &typ.name,
            core_import,
            config,
        )
    {
        return code;
    }

    let mut fields: Vec<(&str, String)> = Vec::new();
    for field in &typ.fields {
        if field.binding_excluded {
            if !typ.has_default {
                fields.push((field.name.as_str(), "Default::default()".to_string()));
                continue;
            }
            continue;
        }
        if field.cfg.is_some()
            && !config.never_skip_cfg_field_names.contains(&field.name)
            && config.strip_cfg_fields_from_binding_struct
        {
            continue;
        }
        let name = &field.name;
        let is_bridge_named = match &field.ty {
            crate::core::ir::TypeRef::Named(n) => bridge_type_aliases.contains(n.as_str()),
            crate::core::ir::TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n.as_str()))
            }
            _ => false,
        };
        if is_bridge_named {
            fields.push((name.as_str(), format!("val.{name}.map(|v| (*v.inner).clone())")));
        } else if field.sanitized {
            fields.push((name.as_str(), "Default::default()".to_string()));
        } else if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
            let conversion = gen_string_to_enum_expr(
                &format!("val.{name}"),
                &enum_name,
                field.optional,
                enums,
                core_import,
                name,
            );
            fields.push((name.as_str(), conversion));
        } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
            let elem_conversion = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import, name);
            let conversion = if field.optional {
                format!("val.{name}.map(|v| v.into_iter().map(|s| {elem_conversion}).collect())")
            } else {
                format!("val.{name}.into_iter().map(|s| {elem_conversion}).collect()")
            };
            fields.push((name.as_str(), conversion));
        } else if !field.optional
            && matches!(field.ty, TypeRef::Duration)
            && config.option_duration_on_defaults
            && typ.has_default
        {
            let cast = if config.cast_large_ints_to_i64 { " as u64" } else { "" };
            let conversion = crate::backends::php::template_env::render(
                "php_duration_default_expr.jinja",
                context! {
                    value_expr => &format!("val.{name}"),
                    cast => cast,
                    core_type => &core_path,
                    field_name => name.as_str(),
                },
            );
            fields.push((name.as_str(), conversion));
        } else if matches!(field.ty, TypeRef::Bytes)
            || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
        {
            let conversion = if field.optional {
                format!("val.{name}.map(|v| v.into())")
            } else {
                format!("val.{name}.into()")
            };
            fields.push((name.as_str(), conversion));
        } else {
            let conversion =
                crate::codegen::conversions::field_conversion_to_core_cfg(name, &field.ty, field.optional, config);
            let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
                if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                    match &field.ty {
                        TypeRef::Optional(_) => format!("{name}: ({expr}).map({newtype_path})"),
                        TypeRef::Vec(_) => {
                            let inner_expr = if let Some(prefix) = expr.strip_suffix(".collect()") {
                                format!("{prefix}.collect::<Vec<_>>()")
                            } else {
                                expr.to_string()
                            };
                            format!("{name}: ({inner_expr}).into_iter().map({newtype_path}).collect()")
                        }
                        _ if field.optional => format!("{name}: ({expr}).map({newtype_path})"),
                        _ => format!("{name}: {newtype_path}({expr})"),
                    }
                } else {
                    conversion
                }
            } else {
                conversion
            };
            let conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
                if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                    if field.optional {
                        format!("{name}: {expr}.map(Box::new)")
                    } else {
                        format!("{name}: Box::new({expr})")
                    }
                } else {
                    conversion
                }
            } else {
                conversion
            };
            let conversion = crate::codegen::conversions::apply_core_wrapper_to_core(
                &conversion,
                name,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            );
            let field_expr = conversion.strip_prefix(&format!("{name}: ")).unwrap_or(&conversion);
            fields.push((name.as_str(), field_expr.to_string()));
        }
    }
    if typ.has_private_fields {
        let assignments: Vec<_> = fields
            .into_iter()
            .filter(|(_, expr)| expr != "Default::default()")
            .map(
                |(core_field, expr)| crate::codegen::conversions::construction::FieldAssign {
                    core_field: core_field.to_string(),
                    expr,
                },
            )
            .collect();
        return crate::codegen::conversions::construction::gen_private_field_from_impl(
            &crate::codegen::conversions::construction::PrivateFieldImpl {
                core_path: &core_path,
                binding_name: &typ.name,
                param: "val",
                has_default: typ.has_default,
                assignments: &assignments,
                allow_attrs: &["clippy::useless_conversion"],
            },
        );
    }

    let emit_default_spread = typ.has_default;
    let mut out = crate::backends::php::template_env::render(
        "php_impl_from_begin.jinja",
        context! {
            binding_type => &typ.name,
            core_type => &core_path,
            emit_spread => emit_default_spread,
        },
    );
    for &(field_name, ref field_expr) in &fields {
        out.push_str(&crate::backends::php::template_env::render(
            "php_struct_field_assignment.jinja",
            context! {
                field_name => field_name,
                field_expr => field_expr,
            },
        ));
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_impl_from_end.jinja",
        context! {
            emit_spread => emit_default_spread,
        },
    ));
    out
}

/// If the TypeRef is a Named type referencing an enum, return the enum name.
/// Handles Named(enum) and Optional(Named(enum)).
pub(super) fn get_direct_enum_named(ty: &TypeRef, enum_names: &AHashSet<String>) -> Option<String> {
    match ty {
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(name.clone()),
        TypeRef::Optional(inner) => get_direct_enum_named(inner, enum_names),
        _ => None,
    }
}

/// If the TypeRef is a Vec<Named(enum)>, return the enum name.
/// Handles Vec(Named(enum)) and Optional(Vec(Named(enum))).
pub(super) fn get_vec_enum_named(ty: &TypeRef, enum_names: &AHashSet<String>) -> Option<String> {
    match ty {
        TypeRef::Vec(inner) => get_direct_enum_named(inner, enum_names),
        TypeRef::Optional(inner) => get_vec_enum_named(inner, enum_names),
        _ => None,
    }
}

/// Generate an expression that converts a String to a core enum type via matching.
///
/// An unrecognised string throws a real PHP exception naming the offending value and the
/// field, via `PhpException::throw()`, before still evaluating to the first-variant/Default
/// fallback. `From::from` cannot return `Result` here without breaking every generated call
/// site that isn't itself `Result`-returning (many aren't — see `enum_defaults.rs` module
/// tests), so the fallback value keeps the surrounding `From` impl infallible and type-correct.
/// `PhpException::throw()` sets the pending Zend exception directly; the PHP VM discards
/// whatever the native call eventually returns and unwinds to PHP-land once the native
/// function returns to it, so the fallback expression is never actually observed by PHP
/// callers even though Rust must still produce a well-typed value here. ~keep
/// Data variants (with fields) use `Default::default()` for each field.
pub(super) fn gen_string_to_enum_expr(
    val_expr: &str,
    enum_name: &str,
    optional: bool,
    enums: &[EnumDef],
    core_import: &str,
    field_name: &str,
) -> String {
    let enum_def = match enums.iter().find(|e| e.name == enum_name) {
        Some(e) => e,
        None => return "Default::default()".to_string(),
    };
    let core_enum_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);

    if enum_def.variants.is_empty() {
        return "Default::default()".to_string();
    }

    /// Build the variant constructor expression, filling data variant fields with defaults.
    fn variant_expr(core_path: &str, variant: &crate::core::ir::EnumVariant) -> String {
        if variant.fields.is_empty() {
            crate::backends::php::template_env::render(
                "php_enum_variant_unit_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                },
            )
        } else if crate::codegen::conversions::is_tuple_variant(&variant.fields) {
            let defaults: Vec<&str> = variant.fields.iter().map(|_| "Default::default()").collect();
            crate::backends::php::template_env::render(
                "php_enum_variant_tuple_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                    defaults => defaults.join(", "),
                },
            )
        } else {
            let fields: Vec<String> = variant
                .fields
                .iter()
                .map(|field| {
                    crate::backends::php::template_env::render(
                        "php_enum_variant_default_field_expr.jinja",
                        context! {
                            field_name => &field.name,
                        },
                    )
                })
                .collect();
            crate::backends::php::template_env::render(
                "php_enum_variant_struct_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                    fields => fields.join(", "),
                },
            )
        }
    }

    let has_default_variant = enum_def.variants.iter().any(|v| v.is_default);
    let fallback_expr = if has_default_variant {
        "Default::default()".to_string()
    } else {
        // A cfg-gated first variant may not exist in a build that doesn't satisfy its
        // condition (E0599); prefer the first ungated variant so the fallback expression
        // itself never names a variant this build might not have. If every variant is
        // gated, there is no safe unconditional fallback left to pick -- keep the previous
        // behavior (first variant) rather than silently changing shape for that edge case.
        let fallback_variant = enum_def
            .variants
            .iter()
            .find(|v| v.cfg.is_none())
            .unwrap_or(&enum_def.variants[0]);
        variant_expr(&core_enum_path, fallback_variant)
    };
    let mut match_arms = String::new();
    let mut valid_variants: Vec<String> = Vec::new();
    for variant in &enum_def.variants {
        let expr = variant_expr(&core_enum_path, variant);
        // against `#[serde(rename)]` first, then `#[serde(rename_all = "...")]`, then
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        // either convention without forcing the core to add `#[serde(rename_all)]`.
        let variant_lower = wire_name.to_lowercase();
        // A variant behind `#[cfg(...)]` (e.g. `#[cfg(any(test, feature = "testkit"))]`) does
        // not exist in a build that doesn't satisfy that condition; an unconditional match arm
        // naming it is a hard E0599. The surrounding match already carries a catch-all `other
        // =>` fallback arm (see php_enum_string_match_fallback_arm.jinja), so unlike an
        // exhaustive Rust-enum match, dropping this arm needs no additional wildcard.
        //
        // Which of the two we do depends on who owns the enum. A *host*-owned gated variant
        // keeps its arm under the same `#[cfg]`, so the arm goes live whenever the feature is
        // on. A *foreign*-owned one (a `[[crates.source_crates]]` enum) must have its arm
        // dropped: the feature name belongs to that crate, not to the generated one, so
        // forwarding the gate emits `unexpected cfg condition value` -- and because
        // `cfg(test)` is satisfied under `cargo clippy --all-targets`, the arm still compiles
        // and fails E0599 on a variant the foreign crate was not built with. ~keep
        if variant.cfg.is_some() && !crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path) {
            tracing::debug!(
                enum_name = %enum_def.name,
                variant = %variant.name,
                "gen_bindings(php): drop string-match arm for a foreign crate's cfg-gated variant"
            );
            continue;
        }
        valid_variants.push(wire_name.clone());
        // A variant behind `#[cfg(...)]` (e.g. `#[cfg(any(test, feature = "testkit"))]`) does
        // not exist in a build that doesn't satisfy that condition; an unconditional match arm
        // naming it is a hard E0599. The surrounding match already carries a catch-all `other
        // =>` fallback arm (see php_enum_string_match_fallback_arm.jinja), so unlike an
        // exhaustive Rust-enum match, dropping this arm from the string match needs no
        // additional wildcard -- gating it with `#[cfg(...)]` is sufficient and keeps the
        // arm live whenever the feature is actually on. ~keep
        match_arms.push_str(&crate::backends::php::template_env::render(
            "php_enum_string_match_arm.jinja",
            context! {
                variant_name => &wire_name,
                variant_name_lower => &variant_lower,
                expr => &expr,
                cfg => variant.cfg.as_deref(),
            },
        ));
    }
    match_arms.push_str(&crate::backends::php::template_env::render(
        "php_enum_string_match_fallback_arm.jinja",
        context! {
            fallback_expr => &fallback_expr,
            field_name => field_name,
            enum_name => enum_name,
            valid_variants => &valid_variants.join(", "),
        },
    ));

    if optional {
        crate::backends::php::template_env::render(
            "php_enum_string_optional_match_expr.jinja",
            context! {
                val_expr => val_expr,
                match_arms => &match_arms,
            },
        )
    } else {
        crate::backends::php::template_env::render(
            "php_enum_string_match_expr.jinja",
            context! {
                val_expr => val_expr,
                match_arms => &match_arms,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, TypeRef};

    fn field(name: &str, binding_excluded: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            binding_excluded,
            ..Default::default()
        }
    }

    fn typ(name: &str, has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("crate::{name}"),
            fields,
            is_clone: true,
            has_default,
            ..Default::default()
        }
    }

    #[test]
    fn enum_tainted_binding_excluded_with_default_uses_spread() {
        let typ = typ(
            "DefaultedWithExcluded",
            true,
            vec![field("name", false), field("attrs", true)],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &AHashSet::new(),
            &AHashSet::new(),
            &cfg,
            &[],
            &AHashSet::new(),
        );

        assert!(
            out.contains("..Default::default()"),
            "spread should be emitted when has_default is true; got:\n{out}"
        );
        assert!(
            !out.contains("attrs: Default::default()"),
            "binding-excluded field should NOT be explicitly emitted when has_default is true; got:\n{out}"
        );
    }

    #[test]
    fn enum_tainted_binding_excluded_without_default_emits_explicit_default() {
        let typ = typ(
            "NoDefaultEnumTainted",
            false,
            vec![field("name", false), field("attrs", true)],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &AHashSet::new(),
            &AHashSet::new(),
            &cfg,
            &[],
            &AHashSet::new(),
        );

        assert!(
            !out.contains("..Default::default()"),
            "spread must NOT be emitted when has_default is false; got:\n{out}"
        );
        assert!(
            out.contains("attrs: Default::default()"),
            "binding-excluded field must be explicitly defaulted when has_default is false; got:\n{out}"
        );
    }

    #[test]
    fn enum_tainted_fully_mirrored_with_default_still_emits_spread() {
        let typ = typ(
            "PlainEnumTainted",
            true,
            vec![field("name", false), field("value", false)],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &AHashSet::new(),
            &AHashSet::new(),
            &cfg,
            &[],
            &AHashSet::new(),
        );

        assert!(
            out.contains("..Default::default()"),
            "has_default core type must always get the spread trailer; got:\n{out}"
        );
        assert!(
            out.contains("#[allow(clippy::needless_update)]"),
            "the spread over a fully-mirrored literal needs the needless_update allow; got:\n{out}"
        );
    }

    #[test]
    fn enum_tainted_fully_mirrored_without_default_keeps_exhaustive_literal() {
        let typ = typ(
            "NoDefaultPlain",
            false,
            vec![field("name", false), field("value", false)],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &AHashSet::new(),
            &AHashSet::new(),
            &cfg,
            &[],
            &AHashSet::new(),
        );

        assert!(
            !out.contains("..Default::default()"),
            "the spread trailer must not be emitted when the core type has no Default \
             impl — it would fail to compile (E0277); got:\n{out}"
        );
    }

    fn unit_variant(name: &str, is_default: bool) -> crate::core::ir::EnumVariant {
        crate::core::ir::EnumVariant {
            name: name.to_string(),
            is_default,
            ..Default::default()
        }
    }

    fn redaction_strategy_enum() -> EnumDef {
        EnumDef {
            name: "RedactionStrategy".to_string(),
            rust_path: "crate::RedactionStrategy".to_string(),
            variants: vec![unit_variant("Mask", true), unit_variant("Hash", false)],
            ..Default::default()
        }
    }

    fn enum_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// Field evidence regression: a struct field naming a core enum by string must still map a
    /// recognised wire value to the matching core variant — the fallback-arm rewrite below must
    /// not disturb the happy path. Asserts on the rendered `impl From` text emitted for the
    /// pipeline's real `gen_enum_tainted_from_binding_to_core` entry point, not on
    /// `gen_string_to_enum_expr`'s return value in isolation. ~keep
    #[test]
    fn enum_tainted_string_field_valid_value_maps_to_correct_core_variant() {
        let enums = vec![redaction_strategy_enum()];
        let enum_names: AHashSet<String> = ["RedactionStrategy".to_string()].into_iter().collect();
        let typ = typ(
            "RedactionConfig",
            false,
            vec![enum_field(
                "strategy",
                TypeRef::Named("RedactionStrategy".to_string()),
                false,
            )],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &enum_names,
            &enum_names,
            &cfg,
            &enums,
            &AHashSet::new(),
        );

        assert!(
            out.contains(r#""Mask" | "mask" => crate::RedactionStrategy::Mask,"#),
            "a recognised wire value must still map to its exact core variant; got:\n{out}"
        );
        assert!(
            out.contains(r#""Hash" | "hash" => crate::RedactionStrategy::Hash,"#),
            "a recognised wire value must still map to its exact core variant; got:\n{out}"
        );
    }

    /// The core defect under test: an unrecognised wire value must never silently fall through
    /// to a default core variant. It must throw a real PHP exception instead. Checks the
    /// rendered fallback arm binds the offending string (`other`, not `_`) and calls
    /// `PhpException::throw()` rather than only evaluating to a bare fallback expression.
    #[test]
    fn enum_tainted_string_field_unknown_value_throws_instead_of_silently_defaulting() {
        let enums = vec![redaction_strategy_enum()];
        let enum_names: AHashSet<String> = ["RedactionStrategy".to_string()].into_iter().collect();
        let typ = typ(
            "RedactionConfig",
            false,
            vec![enum_field(
                "strategy",
                TypeRef::Named("RedactionStrategy".to_string()),
                false,
            )],
        );
        let cfg = ConversionConfig::default();
        let out = gen_enum_tainted_from_binding_to_core(
            &typ,
            "crate",
            &enum_names,
            &enum_names,
            &cfg,
            &enums,
            &AHashSet::new(),
        );

        assert!(
            out.contains("other =>"),
            "the fallback arm must bind the offending string instead of discarding it with `_`; got:\n{out}"
        );
        assert!(
            !out.contains("_ => crate::RedactionStrategy::Mask"),
            "the old silent-default fallback arm shape must be gone; got:\n{out}"
        );
        assert!(
            out.contains("ext_php_rs::exception::PhpException::default(format!("),
            "an unrecognised value must construct a PhpException; got:\n{out}"
        );
        assert!(
            out.contains(".throw()"),
            "an unrecognised value must actually throw the PhpException, not just build it; got:\n{out}"
        );
    }

    /// The error must name both the offending value AND the field, per the ticket's explicit
    /// requirement — "Invalid enum value" with no value in it is barely better than silence.
    /// `other` is a runtime binding (the interpolation happens when the generated crate
    /// executes, not at alef codegen time), so this asserts the two static pieces that make the
    /// runtime message correct: the field-name literal is baked into the format string, and the
    /// runtime value is captured via `{other:?}` rather than discarded. See
    /// `enum_tainted_string_field_thrown_message_is_executed_and_contains_value_and_field` below
    /// for a test that actually compiles and runs this exact generated fragment.
    #[test]
    fn enum_tainted_string_field_error_message_names_field_and_captures_value() {
        let enums = vec![redaction_strategy_enum()];
        let expr = gen_string_to_enum_expr("val.strategy", "RedactionStrategy", false, &enums, "crate", "strategy");

        assert!(
            expr.contains("field 'strategy'"),
            "the field name must be named in the thrown message; got:\n{expr}"
        );
        assert!(
            expr.contains("{other:?}"),
            "the offending runtime value must be captured into the thrown message; got:\n{expr}"
        );
        assert!(
            expr.contains("RedactionStrategy"),
            "the enum name must be named in the thrown message for context; got:\n{expr}"
        );
    }

    /// The regression this task fixes: a variant behind `#[cfg(...)]` (e.g.
    /// `#[cfg(any(test, feature = "testkit"))]`) does not exist in a build that doesn't satisfy
    /// that condition; before the fix, `gen_string_to_enum_expr` never read `variant.cfg` at
    /// all, so its match arm named the variant unconditionally -- a hard E0599 in the real
    /// failure this fixes. An ungated sibling variant in the same enum must be unaffected.
    #[test]
    fn cfg_gated_variant_carries_cfg_guard_on_its_match_arm() {
        let enums = vec![EnumDef {
            name: "RedactionStrategy".to_string(),
            rust_path: "crate::RedactionStrategy".to_string(),
            variants: vec![
                unit_variant("Mask", true),
                crate::core::ir::EnumVariant {
                    cfg: Some(r#"feature = "testkit""#.to_string()),
                    ..unit_variant("Hash", false)
                },
            ],
            ..Default::default()
        }];
        let expr = gen_string_to_enum_expr("val.strategy", "RedactionStrategy", false, &enums, "crate", "strategy");

        assert_eq!(
            expr.matches("#[cfg(feature = \"testkit\")]").count(),
            1,
            "the cfg-gated variant's match arm must carry the guard exactly once, got:\n{expr}"
        );
        assert!(
            expr.contains(r#""Mask" | "mask" => crate::RedactionStrategy::Mask,"#),
            "the ungated sibling variant's arm must stay unconditional, got:\n{expr}"
        );
    }

    /// The host-owned test above keeps its arm under a `#[cfg]`. A *foreign*-owned enum -- one
    /// from a `[[crates.source_crates]]` entry -- must have the arm dropped instead. Forwarding
    /// the gate emits `unexpected cfg condition value` (the feature belongs to that crate, not
    /// the generated one), and because `cfg(test)` is satisfied under `cargo clippy
    /// --all-targets` the arm still compiles and fails E0599 on a variant the foreign crate was
    /// not built with. Both symptoms were observed in a consumer's generated PHP crate. ~keep
    #[test]
    fn foreign_crate_cfg_gated_variant_drops_its_match_arm_instead_of_gating_it() {
        let enums = vec![EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "foreign_lib::TierStrategy".to_string(),
            variants: vec![
                unit_variant("Auto", true),
                crate::core::ir::EnumVariant {
                    cfg: Some(r#"any(test, feature = "testkit")"#.to_string()),
                    ..unit_variant("Tier1", false)
                },
            ],
            ..Default::default()
        }];
        let expr = gen_string_to_enum_expr("val.tier", "TierStrategy", false, &enums, "hostlib", "tier");

        assert!(
            !expr.contains("testkit"),
            "a foreign crate's feature name must not be forwarded into the generated crate, got:\n{expr}"
        );
        assert!(
            !expr.contains("TierStrategy::Tier1"),
            "the dropped arm must not name a variant this build may not have, got:\n{expr}"
        );
        assert!(
            !expr.contains("\"Tier1\""),
            "a dropped arm must not still be advertised as an accepted value, got:\n{expr}"
        );
        assert!(
            expr.contains("foreign_lib::TierStrategy::Auto"),
            "the ungated sibling must survive, got:\n{expr}"
        );
    }

    /// Negative control: when no variant carries a cfg (`redaction_strategy_enum()` as-is),
    /// the generated match must contain no `#[cfg(...)]` at all.
    #[test]
    fn ungated_enum_emits_no_cfg_in_string_match() {
        let enums = vec![redaction_strategy_enum()];
        let expr = gen_string_to_enum_expr("val.strategy", "RedactionStrategy", false, &enums, "crate", "strategy");

        assert!(
            !expr.contains("#[cfg("),
            "an ungated enum must not emit #[cfg(...)] anywhere in the match, got:\n{expr}"
        );
    }

    /// When the enum has no `is_default` variant, the unrecognised-value fallback expression
    /// falls back to constructing `enum_def.variants[0]` -- but if that first variant is itself
    /// cfg-gated, blindly picking it would name a possibly-nonexistent variant unconditionally
    /// in the fallback branch too. The fix must skip cfg-gated variants when choosing a
    /// fallback and prefer the first ungated one instead.
    ///
    /// Discriminating assertion: with no `is_default` variant, `Mask`'s own match arm and the
    /// fallback expression are the only two places `RedactionStrategy::Mask` can appear, so a
    /// correct fallback pick (skipping the gated `Hash`) produces exactly 2 occurrences. A
    /// broken fix that still picks the gated first variant would produce only 1 (arm only),
    /// with `RedactionStrategy::Hash` appearing twice instead.
    #[test]
    fn fallback_expr_skips_cfg_gated_first_variant_when_no_default_exists() {
        let enums = vec![EnumDef {
            name: "RedactionStrategy".to_string(),
            rust_path: "crate::RedactionStrategy".to_string(),
            variants: vec![
                crate::core::ir::EnumVariant {
                    cfg: Some(r#"feature = "testkit""#.to_string()),
                    ..unit_variant("Hash", false)
                },
                unit_variant("Mask", false),
            ],
            ..Default::default()
        }];
        let expr = gen_string_to_enum_expr("val.strategy", "RedactionStrategy", false, &enums, "crate", "strategy");

        assert_eq!(
            expr.matches("crate::RedactionStrategy::Mask").count(),
            2,
            "the fallback expression must construct the first UNGATED variant (Mask), not the \
             gated first variant (Hash), got:\n{expr}"
        );
    }

    /// Executes the exact generated match expression (not a hand-written stand-in) against a
    /// stub `ext_php_rs::exception` module, proving — not inferring — that the thrown message
    /// contains both the offending value and the field name at runtime. Prevents a codegen
    /// change from silently reordering/renaming the captured identifiers in a way that string
    /// assertions on the template text alone could miss. ~keep
    #[test]
    fn enum_tainted_string_field_thrown_message_is_executed_and_contains_value_and_field() {
        let enums = vec![redaction_strategy_enum()];
        let expr = gen_string_to_enum_expr("val.strategy", "RedactionStrategy", false, &enums, "crate", "strategy");

        let harness = format!(
            r#"
mod ext_php_rs {{
    pub mod exception {{
        pub struct PhpException(String);
        impl PhpException {{
            pub fn default(message: String) -> Self {{ Self(message) }}
            pub fn throw(self) -> Result<(), ()> {{
                println!("THROWN:{{}}", self.0);
                Ok(())
            }}
        }}
    }}
}}

#[derive(Debug, Default)]
enum RedactionStrategy {{ #[default] Mask, Hash }}

struct Val {{ strategy: String }}

fn main() {{
    let val = Val {{ strategy: "bogus".to_string() }};
    let _result: RedactionStrategy = {expr};
}}
"#
        );

        let dir = std::env::temp_dir().join(format!(
            "alef_php_enum_throw_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir for harness");
        let src_path = dir.join("harness.rs");
        let bin_path = dir.join("harness_bin");
        std::fs::write(&src_path, &harness).expect("write harness source");

        let compile = std::process::Command::new("rustc")
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path)
            .arg("--edition=2021")
            .output()
            .expect("invoke rustc");
        assert!(
            compile.status.success(),
            "harness failed to compile — the generated fragment is not valid Rust:\nstdout:\n{}\nstderr:\n{}\nfragment:\n{expr}",
            String::from_utf8_lossy(&compile.stdout),
            String::from_utf8_lossy(&compile.stderr),
        );

        let run = std::process::Command::new(&bin_path)
            .output()
            .expect("run harness binary");
        let stdout = String::from_utf8_lossy(&run.stdout);

        assert!(
            stdout.contains("THROWN:"),
            "the harness must actually reach PhpException::throw(); stdout:\n{stdout}"
        );
        assert!(
            stdout.contains("bogus"),
            "the thrown message must contain the offending runtime value; stdout:\n{stdout}"
        );
        assert!(
            stdout.contains("strategy"),
            "the thrown message must contain the field name; stdout:\n{stdout}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[path = "enum_defaults/forced_cfg_tests.rs"]
mod forced_cfg_tests;
