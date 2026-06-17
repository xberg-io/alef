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
    // First, find which enum-tainted types directly reference data-variant enums
    let mut unconvertible: AHashSet<String> = AHashSet::new();
    for typ in types {
        if !enum_tainted.contains(&typ.name) {
            continue;
        }
        for field in binding_fields(&typ.fields) {
            if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
                if let Some(enum_def) = enums.iter().find(|e| e.name == enum_name) {
                    if enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
                        unconvertible.insert(typ.name.clone());
                    }
                }
            }
        }
    }
    // Transitively exclude types that reference unconvertible types
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
    // Return the set of enum-tainted types that CAN be converted
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

    // Types with lifetime parameters (e.g. NodeContext<'a>) have private fields that
    // forbid struct-literal construction AND require `<'_>` in the impl header.
    // Delegate to gen_from_lifetime_type_constructor which locates the correct static
    // constructor (with_owned_*, etc.) and emits a well-formed From impl.
    if typ.has_lifetime_params {
        if let Some(code) =
            crate::codegen::conversions::gen_from_lifetime_type_constructor(typ, &core_path, &typ.name, config)
        {
            return code;
        }
    }

    let mut out = String::with_capacity(512);
    out.push_str(&crate::backends::php::template_env::render(
        "php_impl_from_begin.jinja",
        context! {
            binding_type => &typ.name,
            core_type => &core_path,
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
        },
    ));
    let has_binding_excluded_fields = typ.fields.iter().any(|f| f.binding_excluded);
    for field in &typ.fields {
        if field.binding_excluded {
            if !typ.has_default {
                // The core type does not derive Default, so the trailing
                // `..Default::default()` spread would fail with E0277. Emit
                // `<field>: Default::default()` explicitly for each binding-excluded
                // field. This loses any custom core-level Default behaviour for
                // these fields, but is the only way to construct the struct literal
                // when the core type lacks a Default impl.
                out.push_str(&crate::backends::php::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => field.name.as_str(),
                        field_expr => "Default::default()",
                    },
                ));
                continue;
            }
            // Skip binding_excluded fields entirely; the trailing `..Default::default()`
            // spread fills them with the core type's Default impl. Emitting
            // `<field>: Default::default()` would shadow custom defaults.
            continue;
        }
        // cfg-gated fields are absent from the binding struct and must not appear in the
        // From impl field list — they are filled by the ..Default::default() spread.
        // Exception: fields in `never_skip_cfg_field_names` (trait-bridge options-field
        // attachments like `visitor`) are emitted unconditionally on the binding side,
        // so they must appear in the From impl too — otherwise the handle is silently dropped.
        if field.cfg.is_some() && !config.never_skip_cfg_field_names.contains(&field.name) {
            continue;
        }
        let name = &field.name;
        // Bridge type alias fields (e.g. VisitorHandle) are NOT sanitized but are in
        // from_binding_skip_types, so field_conversion_to_core_cfg would emit Default::default().
        // Handle them here first: the PHP struct wraps opaque Named types in Option<T> even
        // when field.optional=false, so always use the map(|v| (*v.inner).clone()) form.
        let is_bridge_named = match &field.ty {
            crate::core::ir::TypeRef::Named(n) => bridge_type_aliases.contains(n.as_str()),
            crate::core::ir::TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n.as_str()))
            }
            _ => false,
        };
        if is_bridge_named {
            // PHP opaque structs wrap the core handle in Arc<VisitorHandle>; extract via deref.
            // The PHP binding struct stores the field as Option<T> (opaque naming convention),
            // so map over the option rather than direct deref.
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &format!("val.{name}.map(|v| (*v.inner).clone())"),
                },
            ));
        } else if field.sanitized {
            // Sanitized fields (e.g. Duration→u64, Vec<T>→Vec<String>) use Default::default()
            // since they can't be round-tripped from the PHP binding representation.
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => "Default::default()",
                },
            ));
        } else if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
            // Direct enum-Named field: generate string->enum match
            let conversion =
                gen_string_to_enum_expr(&format!("val.{name}"), &enum_name, field.optional, enums, core_import);
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
            // Vec<Enum-Named> field: element-wise string->enum parsing
            let elem_conversion = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import);
            if field.optional {
                let conversion = format!("val.{name}.map(|v| v.into_iter().map(|s| {elem_conversion}).collect())");
                out.push_str(&crate::backends::php::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => name.as_str(),
                        field_expr => &conversion,
                    },
                ));
            } else {
                let conversion = format!("val.{name}.into_iter().map(|s| {elem_conversion}).collect()");
                out.push_str(&crate::backends::php::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => name.as_str(),
                        field_expr => &conversion,
                    },
                ));
            }
        } else if !field.optional
            && matches!(field.ty, TypeRef::Duration)
            && config.option_duration_on_defaults
            && typ.has_default
        {
            // Non-optional Duration stored as Option<i64> (option_duration_on_defaults).
            // field_conversion_to_core_cfg doesn't know about this optionalization and would
            // generate `val.{name} as u64` which fails to compile on Option<i64>.
            // Use the core type's default when None to preserve intended defaults (e.g. 30s timeout).
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
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else if matches!(field.ty, TypeRef::Bytes)
            || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
        {
            // PHP binding Bytes fields are Vec<u8>. Convert via .into() to core Bytes type.
            let conversion = if field.optional {
                format!("val.{name}.map(|v| v.into())")
            } else {
                format!("val.{name}.into()")
            };
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else {
            // Non-enum field (may reference other tainted types, which have their own From)
            let conversion =
                crate::codegen::conversions::field_conversion_to_core_cfg(name, &field.ty, field.optional, config);
            // Newtype wrapping: when field was resolved from a newtype (e.g. NodeIndex → String),
            // wrap the binding value back into the newtype for the core struct.
            let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
                if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                    match &field.ty {
                        TypeRef::Optional(_) => format!("{name}: ({expr}).map({newtype_path})"),
                        TypeRef::Vec(_) => {
                            // When the inner expr already ends with .collect() (e.g. primitive Vec),
                            // the compiler cannot infer the intermediate Vec type without an explicit
                            // type annotation. Use collect::<Vec<_>>() to make it unambiguous.
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
            // Box<T> fields: wrap the converted value in Box::new().
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
            // Apply core wrapper handling (Cow/Arc/Bytes; vec_inner_core_wrapper for Vec<Arc<T>>)
            let conversion = crate::codegen::conversions::apply_core_wrapper_to_core(
                &conversion,
                name,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            );
            // field_conversion_to_core_cfg returns "name: expr" (with the field name prefix).
            // php_struct_field_assignment.jinja already adds "{{ field_name }}: " so we strip
            // the prefix here to avoid "name: name: expr" duplication.
            let field_expr = conversion.strip_prefix(&format!("{name}: ")).unwrap_or(&conversion);
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => field_expr,
                },
            ));
        }
    }
    // Only emit the trailing `..Default::default()` spread when the core type
    // derives Default — otherwise E0277 blocks compilation. For binding-excluded
    // fields the loop above already emitted explicit `field: Default::default()`
    // assignments when `!typ.has_default`.
    let emit_default_spread = typ.has_default && (typ.has_stripped_cfg_fields || has_binding_excluded_fields);
    out.push_str(&crate::backends::php::template_env::render(
        "php_impl_from_end.jinja",
        context! {
            has_stripped_cfg_fields => emit_default_spread,
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
/// Falls back to the first variant if no match found.
/// Data variants (with fields) use `Default::default()` for each field.
pub(super) fn gen_string_to_enum_expr(
    val_expr: &str,
    enum_name: &str,
    optional: bool,
    enums: &[EnumDef],
    core_import: &str,
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
        variant_expr(&core_enum_path, &enum_def.variants[0])
    };
    let mut match_arms = String::new();
    for variant in &enum_def.variants {
        let expr = variant_expr(&core_enum_path, variant);
        // The wire value the PHP user supplies (in JSON or via the binding's String
        // mirror of a Rust enum) follows the core enum's serde rename strategy. Match
        // against `#[serde(rename)]` first, then `#[serde(rename_all = "...")]`, then
        // the variant's raw Rust name as a fallback.
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        // Accept both the serde-renamed wire form (e.g. "Angle") and its lowercase
        // variant (e.g. "angle"). Some core enums implement Serialize/Deserialize
        // manually via a token normaliser (see UrlEscapeStyle), so the wire form on
        // the JSON boundary may be lowercase even when alef's IR sees the raw
        // PascalCase variant name. Matching both keeps the binding robust against
        // either convention without forcing the core to add `#[serde(rename_all)]`.
        let variant_lower = wire_name.to_lowercase();
        match_arms.push_str(&crate::backends::php::template_env::render(
            "php_enum_string_match_arm.jinja",
            context! {
                variant_name => &wire_name,
                variant_name_lower => &variant_lower,
                expr => &expr,
            },
        ));
    }
    match_arms.push_str(&crate::backends::php::template_env::render(
        "php_enum_string_match_fallback_arm.jinja",
        context! {
            fallback_expr => &fallback_expr,
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
    fn enum_tainted_no_excluded_fields_no_spread() {
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
            !out.contains("..Default::default()"),
            "spread must not appear when there are no excluded/stripped fields; got:\n{out}"
        );
    }
}
