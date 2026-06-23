//! Rust-side JSON bridge shims for generated swift-bridge crates.

use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, TypeDef};
use heck::AsSnakeCase;

pub(super) fn emit_from_json_extern_decl(out: &mut String, snake_name: &str, wrapper_name: &str) {
    use heck::ToLowerCamelCase;

    let fn_name = format!("{snake_name}_from_json");
    out.push_str(&crate::backends::swift::template_env::render(
        "rust_from_json_extern_decl.rs.jinja",
        minijinja::context! {
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
        minijinja::context! {
            fn_name => fn_name,
            wrapper_name => wrapper_name,
            source_path => source_path,
            map_expr => map_expr,
        },
    ));
}

/// Collect serde-enabled, non-opaque types from `visible_types` that appear as
/// parameters in either free functions or type methods, excluding those already
/// covered by static e2e shims (`already_covered`).
///
/// These types need `{type_snake}_from_json` shims so Swift e2e tests can
/// deserialise fixture JSON into the strongly-typed request objects required by
/// swift-bridge wrappers.
pub(super) fn collect_serde_param_types<'a>(
    api: &'a ApiSurface,
    visible_types: &[&'a TypeDef],
    visible_functions: &[&FunctionDef],
    already_covered: &[&str],
) -> Vec<&'a TypeDef> {
    let covered: std::collections::HashSet<&str> = already_covered.iter().copied().collect();

    /// Return true if any param in `params` references the type named `name`.
    fn param_uses_type(params: &[crate::core::ir::ParamDef], name: &str) -> bool {
        params.iter().any(|p| p.ty.references_named(name))
    }

    visible_types
        .iter()
        .copied()
        .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
        .filter(|ty| !covered.contains(ty.name.as_str()))
        .filter(|ty| {
            let name = ty.name.as_str();
            let in_free_fn = visible_functions.iter().any(|f| param_uses_type(&f.params, name));
            let in_method = api
                .types
                .iter()
                .any(|t| t.methods.iter().any(|m| param_uses_type(&m.params, name)));
            in_free_fn || in_method
        })
        .collect()
}
