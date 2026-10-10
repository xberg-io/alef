use std::collections::HashMap;

use crate::backends::kotlin::{to_lower_camel, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, TypeRef};

use super::jni_param_type_str;

pub(super) fn get_capsule_config<'a>(
    function: &FunctionDef,
    capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
) -> Option<&'a HostCapsuleTypeConfig> {
    let (name, _) = capsule_return_type(&function.return_type)?;
    capsule_types.get(name)
}

fn capsule_return_type(return_type: &TypeRef) -> Option<(&str, bool)> {
    let (name, optional) = match return_type {
        TypeRef::Named(name) => (name, false),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => (name, true),
            _ => return None,
        },
        _ => return None,
    };
    Some((name, optional))
}

pub(super) fn emit_capsule_function_wrapper(
    body: &mut String,
    function: &FunctionDef,
    bridge_name: &str,
    capsule: &HostCapsuleTypeConfig,
) {
    match capsule_function_projection(function, bridge_name, capsule) {
        Ok(projection) => body.push_str(&template_env::render(
            "capsule_function_wrapper.jinja",
            crate::alef_context! {
                method_name => projection.method_name,
                params => projection.params,
                host_type => projection.host_type,
                bridge_call => projection.bridge_call,
                exception_type => projection.exception_type,
                error_message => projection.error_message,
                construct_expr => projection.construct_expr,
                optional => projection.optional,
            },
        )),
        Err(error) => body.push_str(&template_env::render(
            "generation_error.jinja",
            crate::alef_context! { error => error.to_string() },
        )),
    }
}

struct CapsuleFunctionProjection {
    method_name: String,
    params: String,
    host_type: String,
    bridge_call: String,
    exception_type: String,
    error_message: &'static str,
    construct_expr: String,
    optional: bool,
}

fn capsule_function_projection(
    function: &FunctionDef,
    bridge_name: &str,
    capsule: &HostCapsuleTypeConfig,
) -> anyhow::Result<CapsuleFunctionProjection> {
    let Some((type_name, optional)) = capsule_return_type(&function.return_type) else {
        anyhow::bail!("capsule function `{}` must return a named type", function.name);
    };
    let params = function
        .params
        .iter()
        .map(|param| format!("{}: {}", to_lower_camel(&param.name), jni_param_type_str(&param.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let bridge_args = function
        .params
        .iter()
        .map(|param| to_lower_camel(&param.name))
        .collect::<Vec<_>>()
        .join(", ");
    let native_name = format!("native{}", to_pascal_case(&function.name));
    let (exception_type, error_message) = capsule_exception(function, bridge_name);
    let host_type = capsule.required_host_type(type_name, "kotlin_android")?;
    Ok(CapsuleFunctionProjection {
        method_name: to_lower_camel(&function.name),
        params,
        host_type: if optional {
            format!("{}?", host_type.trim_end_matches('?'))
        } else {
            host_type.to_string()
        },
        bridge_call: format!("{bridge_name}.{native_name}({bridge_args})"),
        exception_type,
        error_message,
        construct_expr: capsule.construct_required("capsulePtr", type_name, "kotlin_android")?,
        optional,
    })
}

fn capsule_exception(function: &FunctionDef, bridge_name: &str) -> (String, &'static str) {
    if function.error_type.is_some() {
        (format!("{bridge_name}Exception"), "\"Function failed\"")
    } else {
        (
            "IllegalArgumentException".to_string(),
            "\"Unexpected null return from native function\"",
        )
    }
}
