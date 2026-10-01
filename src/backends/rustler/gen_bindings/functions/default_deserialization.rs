use super::shared::{render_preamble, resolve_core_type_path};
use crate::backends::rustler::template_env;
use crate::core::ir::{ParamDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Build the deserialization preamble for `Option<String>` JSON params that
/// correspond to default-typed core types, and for `TypeRef::Json` params that
/// need String → serde_json::Value conversion. Returns an empty string when no
/// param needs JSON deserialization.
pub(super) fn build_default_deser_preamble(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    operation: &str,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    for parameter in params {
        lines.extend(deserialization_lines(
            parameter,
            opaque_types,
            default_types,
            core_import,
            operation,
            types_by_name,
        ));
    }
    render_preamble(&lines)
}

fn deserialization_lines(
    parameter: &ParamDef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    operation: &str,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> Vec<String> {
    match &parameter.ty {
        TypeRef::Named(name) if default_types.contains(name) => {
            named_deserialization_lines(parameter, name, core_import, operation, types_by_name)
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !opaque_types.contains(name) => {
                vec_deserialization_lines(parameter, name, core_import, operation, types_by_name)
            }
            _ => Vec::new(),
        },
        TypeRef::Json => json_deserialization_lines(parameter, operation),
        _ => Vec::new(),
    }
}

fn named_deserialization_lines(
    parameter: &ParamDef,
    name: &str,
    core_import: &str,
    operation: &str,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> Vec<String> {
    let core_type = resolve_core_type_path(name, types_by_name, core_import);
    let mut lines = vec![render_fallible_deser_line(
        &parameter.name,
        &format!("{}_core", parameter.name),
        &core_type,
        true,
        operation,
    )];
    if parameter.is_ref && parameter.is_mut {
        lines.push(render_let_binding(
            &format!("mut {}_mut", parameter.name),
            &core_type,
            &format!("{}_core.unwrap_or_default()", parameter.name),
        ));
    }
    lines
}

fn vec_deserialization_lines(
    parameter: &ParamDef,
    inner_name: &str,
    core_import: &str,
    operation: &str,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> Vec<String> {
    let inner_type = resolve_core_type_path(inner_name, types_by_name, core_import);
    let core_type = format!("Vec<{inner_type}>");
    let output_name = if parameter.optional {
        format!("{}_core", parameter.name)
    } else {
        format!("{}_core_option", parameter.name)
    };
    let mut lines = vec![render_fallible_deser_line(
        &parameter.name,
        &output_name,
        &core_type,
        true,
        operation,
    )];
    if !parameter.optional {
        let binding_name = if parameter.is_ref && parameter.is_mut {
            format!("mut {}_core", parameter.name)
        } else {
            format!("{}_core", parameter.name)
        };
        lines.push(render_let_binding(
            &binding_name,
            &core_type,
            &format!("{}_core_option.unwrap_or_default()", parameter.name),
        ));
    }
    lines
}

fn json_deserialization_lines(parameter: &ParamDef, operation: &str) -> Vec<String> {
    let output_name = if parameter.is_mut {
        format!("mut {}_json", parameter.name)
    } else {
        format!("{}_json", parameter.name)
    };
    vec![render_fallible_deser_line(
        &parameter.name,
        &output_name,
        "serde_json::Value",
        parameter.optional,
        operation,
    )]
}

fn render_let_binding(variable_name: &str, variable_type: &str, expression: &str) -> String {
    template_env::render(
        "rust_let_binding.jinja",
        minijinja::context! {
            var_name => variable_name,
            var_type => variable_type,
            expr => expression,
        },
    )
    .trim_end()
    .to_string()
}

pub(super) fn render_json_string_param(name: &str) -> String {
    template_env::render("rust_json_string_param.rs.jinja", minijinja::context! { name => name })
        .trim_end()
        .to_string()
}

pub(super) fn render_ok_expression(expression: &str) -> String {
    template_env::render(
        "rust_ok_expression.rs.jinja",
        minijinja::context! { expression => expression },
    )
    .trim_end()
    .to_string()
}

pub(super) fn render_fallible_deser_line(
    name: &str,
    output_name: &str,
    core_type: &str,
    input_optional: bool,
    operation: &str,
) -> String {
    template_env::render(
        "default_deser_without_error.rs.jinja",
        minijinja::context! {
            name => name,
            output_name => output_name,
            core_type => core_type,
            input_optional => input_optional,
            operation => operation,
        },
    )
    .trim_end()
    .to_string()
}
