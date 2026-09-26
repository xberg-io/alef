use crate::core::ir::TypeRef;

/// Inputs for [`emit_function_return_call`], grouped to keep the function under the
/// too-many-parameters limit without changing any of the values passed through. `out` stays a
/// separate argument since it is the mutable output buffer, not a data input.
pub(super) struct FunctionReturnCallInputs<'a> {
    pub return_type: &'a TypeRef,
    pub capsule_types: &'a std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    pub return_prefix: &'a str,
    pub name: &'a str,
    pub kwargs: &'a [String],
    pub return_converter: Option<&'a str>,
}

pub(super) fn emit_function_return_call(out: &mut String, inputs: FunctionReturnCallInputs<'_>) {
    let FunctionReturnCallInputs {
        return_type,
        capsule_types,
        return_prefix,
        name,
        kwargs,
        return_converter,
    } = inputs;
    let is_void_return = matches!(return_type, TypeRef::Unit);

    // The wrapper's declared return type is the name `options.py` publishes; the extension
    // module hands back its own `#[pyclass]` under that same name. Converting here is what makes
    // the annotation true, exactly as `emit_adapter_wrapper` already does for an adapter whose
    // return type is a public `options` type. ~keep
    if let Some(converter) = return_converter
        && !is_void_return
    {
        let template = if matches!(return_type, TypeRef::Optional(_)) {
            "function_convert_optional_return.jinja"
        } else {
            "function_convert_return.jinja"
        };
        out.push_str(&crate::backends::pyo3::template_env::render(
            template,
            minijinja::context! {
                converter => converter,
                return_prefix => return_prefix,
                name => name,
                kwargs => kwargs.join(", "),
            },
        ));
        return;
    }

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
