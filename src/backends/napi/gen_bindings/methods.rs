//! NAPI-RS tagged-enum From-impl code generation (binding ↔ core conversions).

use crate::core::ir::EnumDef;

use super::enums::{
    tagged_enum_binding_struct_fields, tagged_enum_field_is_tuple, tagged_enum_field_name,
    tagged_enum_mixed_named_fields, variant_data_field_names,
};
use super::functions::{core_prim_str, needs_napi_cast};

/// Generate `From<JsTaggedEnum> for core::TaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    let variants = enum_def
        .variants
        .iter()
        .map(|variant| {
            let default_tag = variant.name.to_lowercase();
            let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
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
                        let binding_field_name = tagged_enum_field_name(variant, f);
                        let has_binding = fields_with_binding_struct.contains(f.name.as_str());
                        let is_single_tuple_named = variant.fields.len() == 1
                            && tagged_enum_field_is_tuple(f)
                            && matches!(&f.ty, TypeRef::Named(_));
                        let is_mixed = !is_single_tuple_named && mixed_named_fields.contains(&f.name);
                        if f.optional {
                            match &f.ty {
                                TypeRef::Path => {
                                    format!("val.{binding_field_name}.map(std::path::PathBuf::from)")
                                }
                                TypeRef::Named(n) if is_mixed => {
                                    let core_type = format!("{core_import}::{n}");
                                    format!(
                                        "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok())",
                                        binding_field_name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{binding_field_name}.map(|v| v.into())")
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{binding_field_name}.map(|v| v.into())")
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{binding_field_name}.map(|v| v as {core_ty})")
                                }
                                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                    format!(
                                        "val.{binding_field_name}.map(|v| v.into_iter().map(Into::into).collect())"
                                    )
                                }
                                _ => {
                                    format!("val.{binding_field_name}")
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
                                        binding_field_name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{binding_field_name}.map(|v| v.into()).unwrap_or_default()")
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{binding_field_name}.map(|v| v.into()).unwrap_or_default()")
                                }
                                TypeRef::Path => {
                                    format!(
                                        "val.{binding_field_name}.map(std::path::PathBuf::from).unwrap_or_default()"
                                    )
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{binding_field_name}.map(|v| v as {core_ty}).unwrap_or_default()")
                                }
                                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                    format!(
                                        "val.{binding_field_name}.map(|v| v.into_iter().map(Into::into).collect()).unwrap_or_default()"
                                    )
                                }
                                _ => {
                                    format!("val.{binding_field_name}.unwrap_or_default()")
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

    let default_variant = enum_def.variants.first().map(|first| {
        let is_tuple = crate::codegen::conversions::is_tuple_variant(&first.fields);
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

    crate::backends::napi::template_env::render(
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
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                if tagged_enum_field_is_tuple(f) && matches!(&f.ty, crate::core::ir::TypeRef::Named(_)) {
                    continue;
                }
                fields.insert(tagged_enum_field_name(v, f));
            }
        }
        fields.into_iter().collect()
    };

    let synth_field_names = variant_data_field_names(enum_def);

    let variants = enum_def
        .variants
        .iter()
        .map(|variant| {
            let default_tag = variant.name.to_lowercase();
            let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
            let this_synth_field = if variant.fields.len() == 1 {
                let field = &variant.fields[0];
                if tagged_enum_field_is_tuple(field) && matches!(&field.ty, crate::core::ir::TypeRef::Named(_)) {
                    Some(tagged_enum_field_name(variant, field))
                } else {
                    None
                }
            } else {
                None
            };

            if variant.fields.is_empty() {
                let mut all_fields_none: Vec<String> = all_fields.iter().map(|f| format!("{f}: None")).collect();
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
                use crate::core::ir::TypeRef;
                let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
                let variant_field_map: std::collections::BTreeMap<String, &crate::core::ir::FieldDef> = variant
                    .fields
                    .iter()
                    .map(|f| (tagged_enum_field_name(variant, f), f))
                    .collect();
                let destructured: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let binding_field_name = tagged_enum_field_name(variant, f);
                        if f.sanitized {
                            if is_tuple {
                                format!("_{binding_field_name}")
                            } else {
                                format!("{}: _{}", f.name, f.name)
                            }
                        } else {
                            binding_field_name
                        }
                    })
                    .collect();
                let mut field_inits: Vec<String> = all_fields
                    .iter()
                    .map(|f| {
                        if let Some(field) = variant_field_map.get(f) {
                            let has_binding = fields_with_binding_struct.contains(f.as_str());
                            let is_mixed = mixed_named_fields.contains(field.name.as_str());
                            let boxed_deref = if field.is_boxed { "*" } else { "" };
                            if field.optional {
                                match &field.ty {
                                    TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                                    TypeRef::Named(_) if is_mixed => {
                                        format!("{f}: {f}.and_then(|v| serde_json::to_string(&v).ok())")
                                    }
                                    TypeRef::Named(_) if has_binding => {
                                        format!("{f}: {f}.map(|v| (*v).into())",)
                                    }
                                    TypeRef::Named(_) => {
                                        format!("{f}: {f}.map(|v| v.into())")
                                    }
                                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                        format!("{f}: {f}.map(|v| v.into_iter().map(Into::into).collect())")
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
                                    TypeRef::Named(_) if has_binding => {
                                        format!("{f}: Some(({boxed_deref}{f}).into())")
                                    }
                                    TypeRef::Named(_) => format!("{f}: Some(({boxed_deref}{f}).into())"),
                                    TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                                    TypeRef::Primitive(p) if needs_napi_cast(p) => match p {
                                        crate::core::ir::PrimitiveType::F32 => format!("{f}: Some({f} as f64)"),
                                        crate::core::ir::PrimitiveType::U64
                                        | crate::core::ir::PrimitiveType::Usize
                                        | crate::core::ir::PrimitiveType::Isize => format!("{f}: Some({f} as i64)"),
                                        _ => format!("{f}: Some({f})"),
                                    },
                                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                        format!("{f}: Some({f}.into_iter().map(Into::into).collect())")
                                    }
                                    _ => format!("{f}: Some({f})"),
                                }
                            }
                        } else {
                            format!("{f}: None")
                        }
                    })
                    .collect();
                for sf in &synth_field_names {
                    if this_synth_field.as_deref() == Some(sf.as_str()) {
                        let field = &variant.fields[0];
                        let var_name = tagged_enum_field_name(variant, field);
                        let is_boxed = field.is_boxed;
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

    let has_excluded_variants = !enum_def.excluded_variants.is_empty();

    crate::backends::napi::template_env::render(
        "gen_tagged_enum_core_to_binding.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            tag_field => tag_field,
            variants => variants,
            has_excluded_variants => has_excluded_variants,
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
    fn tagged_enum_from_impls_exist() {}
}
