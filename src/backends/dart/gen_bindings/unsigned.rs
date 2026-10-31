use std::collections::{BTreeSet, HashSet};

use crate::backends::dart::{ident::dart_safe_ident, template_env};
use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;

fn contains_u64(ty: &TypeRef, types: &[TypeDef], enums: &[EnumDef], seen: &mut HashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(PrimitiveType::U64) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_u64(inner, types, enums, seen),
        TypeRef::Map(key, value) => contains_u64(key, types, enums, seen) || contains_u64(value, types, enums, seen),
        TypeRef::Named(name) if seen.insert(name.clone()) => {
            types
                .iter()
                .find(|item| item.name == *name && !item.is_opaque && !item.binding_excluded)
                .is_some_and(|item| {
                    binding_fields(&item.fields).any(|field| contains_u64(&field.ty, types, enums, seen))
                })
                || enums
                    .iter()
                    .find(|item| item.name == *name && !item.binding_excluded)
                    .is_some_and(|item| {
                        item.variants
                            .iter()
                            .flat_map(|variant| &variant.fields)
                            .any(|field| contains_u64(&field.ty, types, enums, seen))
                    })
        }
        _ => false,
    }
}

fn shape(ty: &TypeRef, types: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    if !contains_u64(ty, types, enums, &mut HashSet::new()) {
        return None;
    }
    Some(match ty {
        TypeRef::Optional(inner) => return shape(inner, types, enums),
        TypeRef::Vec(inner) => format!("['list', {}]", shape(inner, types, enums)?),
        TypeRef::Map(key, value) => format!(
            "['map', {}, {}]",
            shape(key, types, enums).unwrap_or_else(|| "null".into()),
            shape(value, types, enums).unwrap_or_else(|| "null".into())
        ),
        TypeRef::Primitive(PrimitiveType::U64) => "'u64'".into(),
        _ => "'dto'".into(),
    })
}

pub(super) fn emit_checks(f: &FunctionDef, types: &[TypeDef], enums: &[EnumDef], out: &mut String) {
    for param in &f.params {
        if let Some(shape) = shape(&param.ty, types, enums) {
            out.push_str(&template_env::render(
                "dart_u64_argument_check.jinja",
                crate::alef_context! {
                    name => dart_safe_ident(&param.name.to_lower_camel_case()),
                    shape => shape,
                },
            ));
        }
    }
}

pub(super) fn emit_helpers(api: &ApiSurface, excluded: &std::collections::HashSet<&str>, out: &mut String) {
    if !out.contains("_alefCheckU64Argument(") {
        return;
    }
    let select_fields = |fields: &[crate::core::ir::FieldDef], tuple: bool| {
        binding_fields(fields).enumerate().filter(|(_, field)| contains_u64(&field.ty, &api.types, &api.enums, &mut HashSet::new()))
            .map(|(index, field)| serde_json::json!({
                "name": if tuple { format!("field{index}") } else { dart_safe_ident(&field.name.to_lower_camel_case()) },
                "shape": shape(&field.ty, &api.types, &api.enums).expect("selected unsigned field"),
            }))
            .collect::<Vec<_>>()
    };
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();
    for ty in &api.types {
        if ty.is_opaque || ty.is_trait || ty.binding_excluded || excluded.contains(ty.name.as_str()) {
            continue;
        }
        let fields = select_fields(&ty.fields, false);
        if !fields.is_empty() && seen.insert(ty.name.clone()) {
            owners.push(serde_json::json!({ "name": ty.name, "fields": fields }));
        }
    }
    for en in &api.enums {
        if en.binding_excluded || excluded.contains(en.name.as_str()) {
            continue;
        }
        for variant in en.variants.iter().filter(|variant| !variant.binding_excluded) {
            let fields = select_fields(&variant.fields, variant.is_tuple);
            if !fields.is_empty() {
                owners.push(serde_json::json!({ "name": format!("{}_{}", en.name, variant.name), "fields": fields }));
            }
        }
    }
    out.push_str(&template_env::render(
        "dart_u64_argument_guard.jinja",
        crate::alef_context! { owners => owners },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_container_masks_preserve_signed_siblings() {
        let unsigned = TypeRef::Primitive(PrimitiveType::U64);
        let signed = TypeRef::Primitive(PrimitiveType::I64);
        assert_eq!(
            shape(
                &TypeRef::Map(Box::new(unsigned.clone()), Box::new(signed.clone())),
                &[],
                &[]
            )
            .as_deref(),
            Some("['map', 'u64', null]")
        );
        assert_eq!(
            shape(&TypeRef::Map(Box::new(signed), Box::new(unsigned)), &[], &[]).as_deref(),
            Some("['map', null, 'u64']")
        );
    }
}
