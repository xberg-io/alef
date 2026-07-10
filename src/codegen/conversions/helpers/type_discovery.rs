use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;

/// Collect all Named type names that appear in the API surface — both as
/// function/method input parameters AND as function/method return types.
/// These are types that need binding→core `From` impls.
///
/// Return types need binding→core From impls because:
/// - Users may construct binding types and convert them to core types
/// - Generated code may use `.into()` on nested Named fields in From impls
/// - Round-trip conversion completeness ensures the API is fully usable
///
/// The result includes transitive dependencies: if `ConversionResult` is a
/// return type and it has a field `metadata: HtmlMetadata`, then `HtmlMetadata`
/// is also included.
pub fn input_type_names(surface: &ApiSurface) -> AHashSet<String> {
    let mut names = AHashSet::new();

    for func in &surface.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut names);
        }
    }
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            for param in &method.params {
                collect_named_types(&param.ty, &mut names);
            }
        }
    }
    for func in &surface.functions {
        collect_named_types(&func.return_type, &mut names);
    }
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            collect_named_types(&method.return_type, &mut names);
        }
    }
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        if !typ.is_opaque && !typ.methods.is_empty() {
            for field in binding_fields(&typ.fields) {
                if !field.sanitized {
                    collect_named_types(&field.ty, &mut names);
                }
            }
        }
    }

    for e in &surface.enums {
        let is_data_enum = e.variants.iter().any(|v| !v.fields.is_empty());
        if names.contains(&e.name) || is_data_enum {
            for variant in &e.variants {
                if variant.binding_excluded {
                    continue;
                }
                for field in &variant.fields {
                    if field.sanitized || field.binding_excluded {
                        continue;
                    }
                    collect_named_types(&field.ty, &mut names);
                }
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = names.iter().cloned().collect();
        for name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *name) {
                for field in binding_fields(&typ.fields) {
                    let mut field_names = AHashSet::new();
                    collect_named_types(&field.ty, &mut field_names);
                    for n in field_names {
                        if names.insert(n) {
                            changed = true;
                        }
                    }
                }
            }
            if let Some(e) = surface.enums.iter().find(|e| e.name == *name) {
                for variant in &e.variants {
                    if variant.binding_excluded {
                        continue;
                    }
                    for field in &variant.fields {
                        if field.sanitized || field.binding_excluded {
                            continue;
                        }
                        let mut field_names = AHashSet::new();
                        collect_named_types(&field.ty, &mut field_names);
                        for n in field_names {
                            if names.insert(n) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    names
}

/// Recursively collect all `Named(name)` from a TypeRef.
fn collect_named_types(ty: &TypeRef, out: &mut AHashSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Check if a TypeRef references a Named type that is in the exclude list.
/// Used to skip fields whose types were excluded from binding generation,
/// preventing references to non-existent wrapper types (e.g. Js* in WASM).
pub fn field_references_excluded_type(ty: &TypeRef, exclude_types: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => exclude_types.iter().any(|e| e == name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_excluded_type(inner, exclude_types),
        TypeRef::Map(k, v) => {
            field_references_excluded_type(k, exclude_types) || field_references_excluded_type(v, exclude_types)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..Default::default()
        }
    }

    fn data_enum(name: &str, variant_name: &str, fields: Vec<FieldDef>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("pkg::{name}"),
            variants: vec![EnumVariant {
                name: variant_name.to_string(),
                fields,
                ..Default::default()
            }],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn data_enum_variant_field_types_are_inputs_even_when_enum_is_return_only() {
        let mut surface = ApiSurface::default();
        surface.enums.push(data_enum(
            "EnrichStatus",
            "Completed",
            vec![field("result", TypeRef::Named("EnrichResult".to_string()))],
        ));

        let names = input_type_names(&surface);
        assert!(names.contains("EnrichResult"), "{names:?}");
    }

    #[test]
    fn sanitized_and_excluded_variant_fields_are_not_inputs() {
        let mut sanitized = field("points", TypeRef::Named("QuadPoints".to_string()));
        sanitized.sanitized = true;
        let mut excluded = field("entries", TypeRef::Named("MetaEntries".to_string()));
        excluded.binding_excluded = true;

        let mut surface = ApiSurface::default();
        surface.enums.push(data_enum("Geo", "Quad", vec![sanitized]));
        surface.enums.push(data_enum("Node", "Block", vec![excluded]));

        let names = input_type_names(&surface);
        assert!(!names.contains("QuadPoints"), "{names:?}");
        assert!(!names.contains("MetaEntries"), "{names:?}");
    }
}
