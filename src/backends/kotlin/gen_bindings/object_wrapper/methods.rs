use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use std::collections::BTreeSet;

use super::types::kotlin_type_with_string_imports;
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::to_lower_camel;

pub(crate) fn emit_function(
    f: &FunctionDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    _java_package: &str,
    client_type_names: &std::collections::HashSet<&str>,
) {
    emit_cleaned_kdoc(out, &f.doc, "    ");
    let params: Vec<String> = f.params.iter().map(|p| format_param_with_imports(p, imports)).collect();
    let return_ty = kotlin_type_with_string_imports(&f.return_type, false, imports);
    let async_kw = if f.is_async { "suspend " } else { "" };
    let func_name_camel = to_lower_camel(&f.name);
    let call_args = f
        .params
        .iter()
        .map(|p| to_lower_camel(&p.name))
        .collect::<Vec<_>>()
        .join(", ");

    let returns_client_type = match &f.return_type {
        TypeRef::Named(n) => client_type_names.contains(n.as_str()),
        _ => false,
    };

    out.push_str(&crate::backends::kotlin::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            name => func_name_camel,
            params => params.join(", "),
            return_type => return_ty,
        },
    ));
    out.push('\n');

    let optional_suffix = if matches!(f.return_type, TypeRef::Optional(_)) && !returns_client_type {
        ".orElse(null)"
    } else {
        ""
    };

    if f.is_async {
        if returns_client_type {
            let wrapper = return_ty.trim_end_matches('?');
            out.push_str(&crate::backends::kotlin::template_env::render(
                "async_bridge_client_return.jinja",
                minijinja::context! {
                    wrapper => wrapper,
                    name => func_name_camel,
                    args => call_args,
                },
            ));
        } else {
            out.push_str(&crate::backends::kotlin::template_env::render(
                "async_bridge_return.jinja",
                minijinja::context! {
                    name => func_name_camel,
                    args => call_args,
                    optional_suffix => optional_suffix,
                },
            ));
        }
    } else if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "bridge_call_unit.jinja",
            minijinja::context! {
                name => func_name_camel,
                args => call_args,
            },
        ));
        out.push('\n');
    } else if returns_client_type {
        let wrapper = return_ty.trim_end_matches('?');
        out.push_str(&crate::backends::kotlin::template_env::render(
            "bridge_client_return.jinja",
            minijinja::context! {
                wrapper => wrapper,
                name => func_name_camel,
                args => call_args,
            },
        ));
    } else {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "bridge_return.jinja",
            minijinja::context! {
                name => func_name_camel,
                args => call_args,
                optional_suffix => optional_suffix,
            },
        ));
    }
    out.push_str("    }\n");
}

pub(crate) fn format_param_with_imports(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    let ty_str = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
    let default = if p.optional { " = null" } else { "" };
    format!("{}: {}{}", to_lower_camel(&p.name), ty_str, default)
}
