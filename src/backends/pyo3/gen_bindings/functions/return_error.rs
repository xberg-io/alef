use crate::core::ir::TypeRef;

pub(super) fn emit_function_return_call(
    out: &mut String,
    return_type: &TypeRef,
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    return_prefix: &str,
    name: &str,
    kwargs: &[String],
) {
    let is_void_return = matches!(return_type, TypeRef::Unit);

    if is_void_return {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "function_call_statement.jinja",
            minijinja::context! {
                return_prefix => return_prefix,
                name => name,
                kwargs => kwargs.join(", "),
            },
        ));
    } else if match return_type {
        crate::core::ir::TypeRef::Named(n) => capsule_types.contains_key(n),
        crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
            crate::core::ir::TypeRef::Named(n) => capsule_types.contains_key(n),
            _ => false,
        },
        _ => false,
    } {
        let cast_target = match return_type {
            crate::core::ir::TypeRef::Named(n) => n.clone(),
            crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                crate::core::ir::TypeRef::Named(n) => format!("{n} | None"),
                _ => crate::backends::pyo3::type_map::python_type(return_type),
            },
            _ => crate::backends::pyo3::type_map::python_type(return_type),
        };
        out.push_str(&crate::backends::pyo3::template_env::render(
            "function_cast_return.jinja",
            minijinja::context! {
                cast_target => cast_target,
                return_prefix => return_prefix,
                name => name,
                kwargs => kwargs.join(", "),
            },
        ));
    } else {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "function_call.jinja",
            minijinja::context! {
                return_prefix => return_prefix,
                name => name,
                kwargs => kwargs.join(", "),
            },
        ));
    }
}
