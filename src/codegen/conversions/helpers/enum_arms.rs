use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{FieldDef, TypeRef};

use super::field_fragments::sanitized_vec_field_to_core_expr;
use super::{field_references_excluded_type, is_tuple_variant};

/// Emit a named-field initializer, collapsing `field_name: field_name` to the shorthand
/// `field_name` when the conversion expression is just the field itself (no conversion
/// needed). Avoids `clippy::redundant_field_names` in generated struct/variant literals.
fn field_init(field_name: &str, expr: &str) -> String {
    if expr == field_name {
        field_name.to_string()
    } else {
        format!("{field_name}: {expr}")
    }
}

/// Wrap a sanitized field's JSON deserialize so a value that fails to parse emits a
/// `tracing::warn!` before falling back to `Default::default()`, instead of swallowing the
/// parse error with no diagnostic. Only reachable for `binding_enums_have_data: true` backends
/// (rustler, magnus), where the field's JSON text comes from a host-language caller. Stays
/// infallible on purpose: this expression sits inside `impl From<Binding> for Core`, and every
/// backend call site uses `.into()`, so returning `Result` here would require a coordinated
/// `TryFrom` migration across backends outside this module's territory. ~keep
fn sanitized_field_parse_or_warn(access: &str, variant_name: &str, field_name: &str) -> String {
    let context = format!("variant = \"{variant_name}\", field = \"{field_name}\"");
    crate::codegen::template_env::render(
        "conversions/sanitized_json_parse_or_warn",
        minijinja::context! {
            access => access,
            context => context,
            message => "binding provided unparseable JSON for enum variant field; substituting default",
        },
    )
    .trim_end()
    .to_string()
}

/// Generate a match arm for binding -> core direction.
/// Binding enums are always unit-variant-only; core enums may have data variants.
/// `binding_has_data` controls whether the binding enum has the variant's fields (true) or is
/// unit-only (false, e.g. Rustler/Elixir).
/// `binding_uses_tuple_form` records the binding-side variant body shape for tuple variants,
/// so the destructure pattern matches the declaration emitted by the backend template.
/// Generate match arm for binding->core conversion with config (handles type conversions).
pub fn binding_to_core_match_arm_ext_cfg(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    binding_uses_tuple_form: bool,
) -> String {
    use crate::codegen::conversions::field_conversion_to_core_cfg;

    if fields.is_empty() {
        format!("{binding_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        if is_tuple_variant(fields) {
            let defaults: Vec<&str> = fields.iter().map(|_| "Default::default()").collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name}({}),",
                defaults.join(", ")
            )
        } else {
            let defaults: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name} {{ {} }},",
                defaults.join(", ")
            )
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let binding_pattern = field_names.join(", ");
        let core_args: Vec<String> = fields
            .iter()
            .map(|f| {
                let name = &f.name;
                if f.sanitized {
                    let expr = if let TypeRef::Vec(_) = &f.ty {
                        sanitized_vec_field_to_core_expr(name, &f.ty)
                    } else {
                        sanitized_field_parse_or_warn(name, variant_name, name)
                    };
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                if !config.exclude_types.is_empty() && field_references_excluded_type(&f.ty, config.exclude_types) {
                    let expr = sanitized_field_parse_or_warn(name, variant_name, name);
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                let conv = field_conversion_to_core_cfg(name, &f.ty, f.optional, config);
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{name}: ")) {
                    let expr = expr.replace(&format!("val.{name}"), name);
                    expr.to_string()
                } else {
                    conv
                };
                let expr = if config.binding_enum_fields_are_boxed && f.is_boxed && matches!(&f.ty, TypeRef::Named(_)) {
                    if f.optional {
                        expr.replace(
                            &format!("{name}.map(Into::into)"),
                            &format!("{name}.map(|v| (*v).into())"),
                        )
                    } else {
                        expr.replace(&format!("{name}.into()"), &format!("(*{name}).into()"))
                    }
                } else {
                    expr
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        let pattern_syntax = if binding_uses_tuple_form {
            format!("{binding_prefix}::{variant_name}({binding_pattern})")
        } else {
            format!("{binding_prefix}::{variant_name} {{ {binding_pattern} }}")
        };
        format!("{pattern_syntax} => Self::{variant_name}({}),", core_args.join(", "))
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let core_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if f.sanitized {
                    if let TypeRef::Vec(_) = &f.ty {
                        let expr = sanitized_vec_field_to_core_expr(&f.name, &f.ty);
                        return format!("{}: {expr}", f.name);
                    }
                    let expr = sanitized_field_parse_or_warn(&f.name, variant_name, &f.name);
                    return format!("{}: {expr}", f.name);
                }
                let conv = field_conversion_to_core_cfg(&f.name, &f.ty, f.optional, config);
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    expr.replace(&format!("val.{}", f.name), &f.name)
                } else {
                    conv.strip_prefix(&format!("{}: ", f.name)).unwrap_or(&conv).to_string()
                };
                let binding_field_is_boxed =
                    config.binding_enum_fields_are_boxed && f.is_boxed && matches!(&f.ty, TypeRef::Named(_));
                let expr = if binding_field_is_boxed {
                    // The binding field is already a `Box<T>`: unbox, convert, and rebox.
                    if f.optional {
                        format!("{}.map(|v| Box::new((*v).into()))", f.name)
                    } else {
                        format!("Box::new((*{}).into())", f.name)
                    }
                } else if f.is_boxed {
                    if f.optional {
                        format!("{expr}.map(Box::new)")
                    } else {
                        format!("Box::new({expr})")
                    }
                } else {
                    expr
                };
                field_init(&f.name, &expr)
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            core_fields.join(", ")
        )
    }
}

/// Generate a match arm for core -> binding direction.
/// When the binding also has data variants, destructure and forward fields.
/// When the binding is unit-variant-only, discard core data with `..`.
/// `binding_has_data` controls whether the binding enum has the variant's fields (true) or is
/// unit-only (false).
/// `binding_uses_tuple_form` records the binding-side variant body shape for tuple variants,
/// so the constructor matches the declaration emitted by the backend template.
/// Generate match arm for core->binding conversion with config (handles type conversions).
pub fn core_to_binding_match_arm_ext_cfg(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    binding_uses_tuple_form: bool,
) -> String {
    use crate::codegen::conversions::field_conversion_from_core_cfg;
    use ahash::AHashSet;

    if fields.is_empty() {
        format!("{core_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        if is_tuple_variant(fields) {
            format!("{core_prefix}::{variant_name}(..) => Self::{variant_name},")
        } else {
            format!("{core_prefix}::{variant_name} {{ .. }} => Self::{variant_name},")
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let core_pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let mut expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    if f.is_boxed {
                        if config.binding_enum_fields_are_boxed {
                            if f.optional {
                                expr = expr.replace(
                                    &format!("{}.map(Into::into)", f.name),
                                    &format!("{}.map(|v| Box::new((*v).into()))", f.name),
                                );
                            } else {
                                expr = expr.replace(
                                    &format!("{}.into()", f.name),
                                    &format!("Box::new((*{}).into())", f.name),
                                );
                            }
                        } else {
                            expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                        }
                    }
                    if binding_uses_tuple_form {
                        let string_move = format!("{}.to_string()", f.name);
                        if expr == string_move {
                            expr = f.name.clone();
                        }
                        expr
                    } else {
                        field_init(&f.name, &expr)
                    }
                } else {
                    conv
                }
            })
            .collect();
        if binding_uses_tuple_form {
            format!(
                "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name}({}),",
                binding_fields.join(", ")
            )
        } else {
            format!(
                "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name} {{ {} }},",
                binding_fields.join(", ")
            )
        }
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let mut expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    if f.is_boxed {
                        if config.binding_enum_fields_are_boxed {
                            if f.optional {
                                expr = expr.replace(
                                    &format!("{}.map(Into::into)", f.name),
                                    &format!("{}.map(|v| Box::new((*v).into()))", f.name),
                                );
                            } else {
                                expr = expr.replace(
                                    &format!("{}.into()", f.name),
                                    &format!("Box::new((*{}).into())", f.name),
                                );
                            }
                        } else {
                            expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                        }
                    }
                    field_init(&f.name, &expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::PrimitiveType;

    /// Regression coverage for task #511: struct-form named field initializers used to emit
    /// `field: field` unconditionally, tripping `clippy::redundant_field_names` on every
    /// no-op field in generated enum conversions. Table-driven over the collapse rule itself.
    ///
    /// Task #548 re-verified this same helper against every non-collapsible shape a naive
    /// substring/prefix match could get wrong: a method call (`.clone()`), a function call
    /// wrapping the identifier, a deref, a differently-named binding, and a raw identifier field
    /// paired with either its own bare form (must collapse -- `Foo { r#type }` is valid
    /// shorthand) or a differing expression (must not). All pass unchanged because `field_init`
    /// only ever compares `expr` to `field_name` by exact string equality, never a substring or
    /// prefix match. ~keep
    #[test]
    fn field_init_collapses_only_when_expr_is_exactly_the_field_name() {
        let cases: &[(&str, &str, &str)] = &[
            // No conversion needed: expr is exactly the field name -> collapses to shorthand.
            ("url", "url", "url"),
            // Conversion needed: expr differs (a real transform) -> must NOT collapse. This is
            // the more important half: a false-positive collapse here would drop data silently.
            ("url", "val.url.to_string()", "url: val.url.to_string()"),
            // Target name differs from source name: expr names a different identifier, not a
            // conversion of the same field -> must NOT collapse (exact-equality boundary, not
            // a prefix/substring match).
            ("id", "identifier", "id: identifier"),
            // Method call on the field's own binding -> not a bare identifier -> must NOT
            // collapse, even though `bar` is a prefix of `bar.clone()`.
            ("bar", "bar.clone()", "bar: bar.clone()"),
            // Function call wrapping the identifier -> must NOT collapse.
            ("bar", "into_x(bar)", "bar: into_x(bar)"),
            // Deref of the identifier -> must NOT collapse.
            ("bar", "*bar", "bar: *bar"),
            // Raw identifier field with an identical raw-identifier expr -> collapses. `Foo {
            // r#type }` is valid shorthand syntax, so this is a real no-op field like any other.
            ("r#type", "r#type", "r#type"),
            // Raw identifier field whose expr is NOT the same raw identifier -> must NOT
            // collapse.
            ("r#type", "r#type.to_string()", "r#type: r#type.to_string()"),
        ];
        for (field_name, expr, expected) in cases {
            assert_eq!(
                field_init(field_name, expr),
                *expected,
                "field_init({field_name:?}, {expr:?}) mismatch"
            );
        }
    }

    fn string_field(name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }
    }

    /// End-to-end reproduction of the reported defect shape (`Self::Link { url: url, title:
    /// title }`) at the public match-arm generator: one field that needs no conversion must
    /// collapse to shorthand, one field with a real conversion must keep its `name: expr` form.
    #[test]
    fn core_to_binding_match_arm_collapses_no_op_fields_only() {
        let fields = vec![
            FieldDef {
                name: "id".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::I32),
                ..Default::default()
            },
            string_field("url"),
        ];
        let config = ConversionConfig::default();

        let arm = core_to_binding_match_arm_ext_cfg("Core", "Link", &fields, true, &config, false);

        assert_eq!(
            arm, "Core::Link { id, url } => Self::Link { id, url: url.to_string() },",
            "no-op field `id` must collapse to shorthand while converted field `url` keeps its \
             `name: expr` form, got:\n{arm}"
        );
    }

    /// Same reproduction for the binding->core direction, where a bare `String` field needs no
    /// conversion at all: both fields must collapse (this is the exact `url: url, title: title`
    /// shape from the reported clippy findings).
    #[test]
    fn binding_to_core_match_arm_collapses_all_no_op_fields() {
        let fields = vec![string_field("url"), string_field("title")];
        let config = ConversionConfig::default();

        let arm = binding_to_core_match_arm_ext_cfg("Binding", "Link", &fields, true, &config, false);

        assert_eq!(
            arm, "Binding::Link { url, title } => Self::Link { url, title },",
            "both no-op fields must collapse to shorthand, got:\n{arm}"
        );
    }
}
