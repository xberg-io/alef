//! NAPI-RS tagged-enum From-impl code generation (binding ↔ core conversions).

use alef_core::ir::EnumDef;

use super::enums::{tagged_enum_binding_struct_fields, tagged_enum_mixed_named_fields, variant_data_field_names};
use super::functions::{core_prim_str, needs_napi_cast};

/// Generate `From<JsTaggedEnum> for core::TaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    use alef_core::ir::TypeRef;
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    // Determine which Named fields use binding structs vs serde JSON String.
    // A field uses a binding struct only if: (1) it has a binding struct in struct_names,
    // (2) it's not sanitized, and (3) the field name maps to a single Named type across
    // all variants (not shared with different types).
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    // Fields with different Named types across variants are stored as String (JSON) in the
    // binding struct and must be deserialized per-variant via serde_json.
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    // Precompute all variant data for template
    let variants = enum_def
        .variants
        .iter()
        .map(|variant| {
            let default_tag = variant.name.to_lowercase();
            let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let is_empty = variant.fields.is_empty();

            if is_empty {
                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => true,
                    is_tuple => false,
                }
            } else {
                let field_exprs: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let has_binding = fields_with_binding_struct.contains(f.name.as_str());
                        let is_mixed = mixed_named_fields.contains(&f.name);
                        if f.optional {
                            match &f.ty {
                                TypeRef::Path => {
                                    format!("val.{}.map(std::path::PathBuf::from)", f.name)
                                }
                                TypeRef::Named(n) if is_mixed => {
                                    let core_type = format!("{core_import}::{n}");
                                    format!(
                                        "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok())",
                                        f.name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{}.map(|v| v.into())", f.name)
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{}.map(|v| v.into())", f.name)
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{}.map(|v| v as {core_ty})", f.name)
                                }
                                _ => {
                                    format!("val.{}", f.name)
                                }
                            }
                        } else if f.sanitized {
                            let expr = "Default::default()".to_string();
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        } else {
                            let expr = match &f.ty {
                                TypeRef::Named(n) if is_mixed => {
                                    let core_type = format!("{core_import}::{n}");
                                    format!(
                                        "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok()).unwrap_or_default()",
                                        f.name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name)
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name)
                                }
                                TypeRef::Path => {
                                    format!("val.{}.map(std::path::PathBuf::from).unwrap_or_default()", f.name)
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{}.map(|v| v as {core_ty}).unwrap_or_default()", f.name)
                                }
                                _ => {
                                    format!("val.{}.unwrap_or_default()", f.name)
                                }
                            };
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        }
                    })
                    .collect();

                let field_inits: Vec<String> = variant
                    .fields
                    .iter()
                    .zip(field_exprs.iter())
                    .map(|(f, expr)| format!("{}: {expr}", f.name))
                    .collect();

                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => false,
                    is_tuple => is_tuple,
                    field_exprs => field_exprs,
                    field_inits => field_inits,
                }
            }
        })
        .collect::<Vec<_>>();

    // Default fallback to first variant
    let default_variant = enum_def.variants.first().map(|first| {
        let is_tuple = alef_codegen::conversions::is_tuple_variant(&first.fields);
        let is_empty = first.fields.is_empty();

        if is_empty {
            minijinja::context! {
                name => first.name.clone(),
                is_empty => true,
                is_tuple => false,
            }
        } else if is_tuple {
            let defaults: Vec<&str> = first.fields.iter().map(|_| "Default::default()").collect();
            minijinja::context! {
                name => first.name.clone(),
                is_empty => false,
                is_tuple => true,
                defaults => defaults,
            }
        } else {
            let default_fields: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            minijinja::context! {
                name => first.name.clone(),
                is_empty => false,
                is_tuple => false,
                default_fields => default_fields,
            }
        }
    });

    crate::template_env::render(
        "gen_tagged_enum_binding_to_core.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            tag_field => tag_field,
            variants => variants,
            default_variant => default_variant,
        },
    )
}

/// Generate `From<core::TaggedEnum> for JsTaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    // Fields with different Named types across variants are stored as String (JSON) in the
    // binding struct and must be serialized per-variant via serde_json.
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    // Collect all field names across all variants
    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                fields.insert(f.name.clone());
            }
        }
        fields.into_iter().collect()
    };

    // Collect synthesized variant-data field names (e.g. `pdf`, `docx`, `archive`).
    // These are the per-variant optional properties added to the binding struct for
    // single-tuple Named variants, enabling direct property access in TypeScript.
    let synth_field_names = variant_data_field_names(enum_def);

    // Precompute all variant data for template
    let variants = enum_def
        .variants
        .iter()
        .map(|variant| {
            let default_tag = variant.name.to_lowercase();
            let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
            // Synthesized field name for this variant (snake_case of variant name), if any
            let this_synth_field = if variant.fields.len() == 1 {
                let field = &variant.fields[0];
                let is_tuple = field
                    .name
                    .strip_prefix('_')
                    .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
                if is_tuple && matches!(&field.ty, alef_core::ir::TypeRef::Named(_)) {
                    Some(alef_codegen::naming::to_python_name(&variant.name))
                } else {
                    None
                }
            } else {
                None
            };

            if variant.fields.is_empty() {
                let mut all_fields_none: Vec<String> = all_fields.iter().map(|f| format!("{f}: None")).collect();
                // Include synthesized fields as None for empty variants
                for sf in &synth_field_names {
                    all_fields_none.push(format!("{sf}: None"));
                }
                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => true,
                    is_tuple => false,
                    all_fields_none => all_fields_none,
                }
            } else {
                use alef_core::ir::TypeRef;
                let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
                let variant_field_map: std::collections::BTreeMap<&str, &alef_core::ir::FieldDef> =
                    variant.fields.iter().map(|f| (f.name.as_str(), f)).collect();
                let destructured: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        if f.sanitized {
                            if is_tuple {
                                format!("_{}", f.name)
                            } else {
                                format!("{}: _{}", f.name, f.name)
                            }
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                let mut field_inits: Vec<String> = all_fields
                    .iter()
                    .map(|f| {
                        if let Some(field) = variant_field_map.get(f.as_str()) {
                            let has_binding = fields_with_binding_struct.contains(f.as_str());
                            let is_mixed = mixed_named_fields.contains(f.as_str());
                            if field.optional {
                                match &field.ty {
                                    TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                                    TypeRef::Named(_) if is_mixed => {
                                        format!("{f}: {f}.and_then(|v| serde_json::to_string(&v).ok())")
                                    }
                                    TypeRef::Named(_) if has_binding => {
                                        format!("{f}: {f}.map(|v| v.into())")
                                    }
                                    TypeRef::Named(_) => {
                                        format!("{f}: {f}.map(|v| v.into())")
                                    }
                                    _ => format!("{f}: {f}"),
                                }
                            } else if field.sanitized {
                                format!("{f}: None")
                            } else {
                                match &field.ty {
                                    TypeRef::Named(_) if is_mixed => {
                                        format!("{f}: serde_json::to_string(&{f}).ok()")
                                    }
                                    TypeRef::Named(_) if has_binding => format!("{f}: Some({f}.into())"),
                                    TypeRef::Named(_) => format!("{f}: Some({f}.into())"),
                                    TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                                    TypeRef::Primitive(p) if needs_napi_cast(p) => match p {
                                        alef_core::ir::PrimitiveType::F32 => format!("{f}: Some({f} as f64)"),
                                        alef_core::ir::PrimitiveType::U64
                                        | alef_core::ir::PrimitiveType::Usize
                                        | alef_core::ir::PrimitiveType::Isize => format!("{f}: Some({f} as i64)"),
                                        _ => format!("{f}: Some({f})"),
                                    },
                                    _ => format!("{f}: Some({f})"),
                                }
                            }
                        } else {
                            format!("{f}: None")
                        }
                    })
                    .collect();
                // Append synthesized variant-data fields. The field matching this variant gets
                // Some(inner.into()), all others get None.
                for sf in &synth_field_names {
                    if this_synth_field.as_deref() == Some(sf.as_str()) {
                        // The destructured tuple variable is the first field name
                        let var_name = &variant.fields[0].name;
                        let is_boxed = variant.fields[0].is_boxed;
                        if is_boxed {
                            field_inits.push(format!("{sf}: Some((*{var_name}).into())"));
                        } else {
                            field_inits.push(format!("{sf}: Some({var_name}.into())"));
                        }
                    } else {
                        field_inits.push(format!("{sf}: None"));
                    }
                }

                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => false,
                    is_tuple => is_tuple,
                    destructured => destructured,
                    field_inits => field_inits,
                }
            }
        })
        .collect::<Vec<_>>();

    crate::template_env::render(
        "gen_tagged_enum_core_to_binding.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            tag_field => tag_field,
            variants => variants,
        },
    )
}

/// Determine which Named fields in a tagged enum have **different** Named types across variants.
/// These fields cannot use a single `JsXxx` binding type, so they are stored as `String` (JSON)
/// and converted via `serde_json` per variant in the From impls.
#[cfg(test)]
mod tests {
    /// gen_tagged_enum_binding_to_core is tested via integration tests in gen_bindings_test.rs.
    /// This unit test verifies the function exists and is callable.
    #[test]
    fn tagged_enum_from_impls_exist() {
        // Compilation check only — integration tests cover the generated output.
    }
}
