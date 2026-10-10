//! Rust-side JSON bridge shims for generated swift-bridge crates.

use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use heck::AsSnakeCase;

pub(super) fn emit_from_json_extern_decl(out: &mut String, snake_name: &str, wrapper_name: &str) {
    use heck::ToLowerCamelCase;

    let fn_name = format!("{snake_name}_from_json");
    out.push_str(&crate::backends::swift::template_env::render(
        "rust_from_json_extern_decl.rs.jinja",
        crate::alef_context! {
            swift_name => fn_name.to_lower_camel_case(),
            fn_name => fn_name,
            wrapper_name => wrapper_name,
        },
    ));
}

pub(super) fn emit_type_from_json_extern_block(out: &mut String, types: &[&TypeDef]) {
    if types.is_empty() {
        return;
    }
    out.push_str("    extern \"Rust\" {\n");
    for ty in types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        emit_from_json_extern_decl(out, &type_snake, &ty.name);
    }
    out.push_str("    }\n");
}

pub(super) fn emit_enum_from_json_extern_block(out: &mut String, enums: &[&EnumDef]) {
    if enums.is_empty() {
        return;
    }
    out.push_str("    extern \"Rust\" {\n");
    for en in enums {
        let enum_snake = AsSnakeCase(en.name.as_str()).to_string();
        emit_from_json_extern_decl(out, &enum_snake, &en.name);
    }
    out.push_str("    }\n");
}

pub(super) fn emit_from_json_shim(
    out: &mut String,
    snake_name: &str,
    wrapper_name: &str,
    source_path: &str,
    map_expr: &str,
) {
    let fn_name = format!("{snake_name}_from_json");
    out.push_str(&crate::backends::swift::template_env::render(
        "rust_from_json_shim.rs.jinja",
        crate::alef_context! {
            fn_name => fn_name,
            wrapper_name => wrapper_name,
            source_path => source_path,
            map_expr => map_expr,
        },
    ));
}

pub(super) fn collect_signature_serde_types<'a>(
    visible_types: &[&'a TypeDef],
    visible_functions: &[&FunctionDef],
    already_covered: &[&str],
) -> Vec<&'a TypeDef> {
    let covered: std::collections::HashSet<&str> = already_covered.iter().copied().collect();

    visible_types
        .iter()
        .copied()
        .filter(|ty| ty.has_serde && !ty.is_trait)
        .filter(|ty| !covered.contains(ty.name.as_str()))
        .filter(|ty| {
            crate::backends::swift::signatures_reference_named(
                visible_types.iter().copied(),
                visible_functions.iter().copied(),
                &ty.name,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, FieldDef, TypeRef};

    #[test]
    fn collects_opaque_serde_types_referenced_by_fields() {
        let credential = TypeDef {
            name: "CredentialConfig".to_owned(),
            is_opaque: true,
            has_serde: true,
            ..TypeDef::default()
        };
        let unused = TypeDef {
            name: "UnusedConfig".to_owned(),
            is_opaque: true,
            has_serde: true,
            ..TypeDef::default()
        };
        let options = TypeDef {
            name: "RequestOptions".to_owned(),
            fields: vec![FieldDef {
                name: "credential".to_owned(),
                ty: TypeRef::Named("CredentialConfig".to_owned()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        };
        let api = ApiSurface {
            types: vec![credential, unused, options],
            ..ApiSurface::default()
        };
        let visible_types: Vec<_> = api.types.iter().collect();

        let collected = collect_signature_serde_types(&visible_types, &[], &[]);
        let names: Vec<_> = collected.iter().map(|ty| ty.name.as_str()).collect();

        assert_eq!(names, vec!["CredentialConfig"]);
    }
}
