use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{FieldDef, TypeRef};

use super::field_fragments::sanitized_vec_field_to_core_expr;
use super::{field_references_excluded_type, is_tuple_variant};

/// Generate a match arm for binding -> core direction.
/// Binding enums are always unit-variant-only. Core enums may have data variants.
/// For data variants: `BindingEnum::Variant => CoreEnum::Variant(Default::default(), ...)`
pub fn binding_to_core_match_arm(binding_prefix: &str, variant_name: &str, fields: &[FieldDef]) -> String {
    binding_to_core_match_arm_ext(binding_prefix, variant_name, fields, false)
}

/// Like `binding_to_core_match_arm` but `binding_has_data` controls whether the binding
/// enum has the variant's fields (true) or is unit-only (false, e.g. Rustler/Elixir).
/// `enum_serde_untagged` toggles the binding-side variant body shape for tuple variants:
/// untagged enums emit tuple-form `Variant(T)` in the binding (see Magnus template), so the
/// destructure pattern must match. Tagged/unit enums keep struct-form `Variant { _0 }`.
/// Generate match arm for binding->core conversion with config (handles type conversions).
pub fn binding_to_core_match_arm_ext_cfg(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    enum_serde_untagged: bool,
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
                        format!("serde_json::from_str(&{name}).unwrap_or_default()")
                    };
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                if !config.exclude_types.is_empty() && field_references_excluded_type(&f.ty, config.exclude_types) {
                    let expr = format!("serde_json::from_str(&{name}).unwrap_or_default()");
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                let conv = field_conversion_to_core_cfg(name, &f.ty, f.optional, config);
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{name}: ")) {
                    let expr = expr.replace(&format!("val.{name}"), name);
                    expr.to_string()
                } else {
                    conv
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        let pattern_syntax = if enum_serde_untagged {
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
                    return format!("{}: serde_json::from_str(&{}).unwrap_or_default()", f.name, f.name);
                }
                let conv = field_conversion_to_core_cfg(&f.name, &f.ty, f.optional, config);
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    format!("{}: {}", f.name, expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            core_fields.join(", ")
        )
    }
}

pub fn binding_to_core_match_arm_ext(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
) -> String {
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
                let expr = if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{name}.into()")
                } else if f.sanitized {
                    format!("serde_json::from_str(&{name}).unwrap_or_default()")
                } else {
                    name.clone()
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {binding_pattern} }} => Self::{variant_name}({}),",
            core_args.join(", ")
        )
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let core_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{}: {}.into()", f.name, f.name)
                } else if f.sanitized {
                    format!("{}: serde_json::from_str(&{}).unwrap_or_default()", f.name, f.name)
                } else {
                    format!("{0}: {0}", f.name)
                }
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
pub fn core_to_binding_match_arm(core_prefix: &str, variant_name: &str, fields: &[FieldDef]) -> String {
    core_to_binding_match_arm_ext(core_prefix, variant_name, fields, false)
}

/// Like `core_to_binding_match_arm` but `binding_has_data` controls whether the binding
/// enum has the variant's fields (true) or is unit-only (false).
/// `enum_serde_untagged` toggles the binding-side variant body shape for tuple variants:
/// untagged enums emit tuple-form `Variant(T)` in the binding (see Magnus template), so the
/// constructor must use tuple form too. Tagged/unit enums keep struct-form `Variant { _0 }`.
/// Generate match arm for core->binding conversion with config (handles type conversions).
pub fn core_to_binding_match_arm_ext_cfg(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    enum_serde_untagged: bool,
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
                        expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                    }
                    if enum_serde_untagged {
                        let string_move = format!("{}.to_string()", f.name);
                        if expr == string_move {
                            expr = f.name.clone();
                        }
                        expr
                    } else {
                        format!("{}: {}", f.name, expr)
                    }
                } else {
                    conv
                }
            })
            .collect();
        if enum_serde_untagged {
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
                        expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                    }
                    format!("{}: {}", f.name, expr)
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

pub fn core_to_binding_match_arm_ext(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
) -> String {
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
                let name = &f.name;
                let expr = if f.is_boxed && matches!(&f.ty, TypeRef::Named(_)) {
                    format!("(*{name}).into()")
                } else if f.is_boxed {
                    format!("*{name}")
                } else if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{name}.into()")
                } else if f.sanitized {
                    format!("serde_json::to_string(&{name}).unwrap_or_default()")
                } else {
                    name.clone()
                };
                format!("{name}: {expr}")
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{}: {}.into()", f.name, f.name)
                } else if f.sanitized {
                    format!("{}: serde_json::to_string(&{}).unwrap_or_default()", f.name, f.name)
                } else {
                    format!("{0}: {0}", f.name)
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    }
}
