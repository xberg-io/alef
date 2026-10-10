use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_java_name;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::HashSet;

use super::super::helpers::{is_bridge_param_java, render_nullable_type};
use super::super::marshal::{ffi_param_args, marshal_param_to_ffi, opaque_lease_resource};
use super::params_returns::return_type_name;
use super::visitor_bridge::VisitorFunctionBridge;

/// The three lookup sets used together to classify and marshal a visitor method's non-bridge
/// parameters. Threaded as a unit through the visitor-method emission helpers below. `pub(super)`
/// because `gen_convert_with_visitor_internal_method`'s call site in the parent module
/// (`ffi_class.rs`) also constructs one, to keep that function's own parameter count under the
/// lint limit.
pub(super) struct BridgeParamContext<'a> {
    pub(super) opaque_types: &'a AHashSet<String>,
    pub(super) bridge_param_names: &'a HashSet<String>,
    pub(super) bridge_type_aliases: &'a HashSet<String>,
}

fn effective_param_type(param: &crate::core::ir::ParamDef) -> TypeRef {
    if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
        TypeRef::Optional(Box::new(param.ty.clone()))
    } else {
        param.ty.clone()
    }
}

fn visitor_method_params(
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    func.params
        .iter()
        .filter(|param| !is_bridge_param_java(param, bridge_param_names, bridge_type_aliases))
        .map(|param| {
            let param_type = if param.optional {
                java_boxed_type(&param.ty)
            } else {
                java_type(&param.ty)
            };
            format!(
                "final {} {}",
                render_nullable_type(&param_type, param.optional),
                to_java_name(&param.name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_visitor_resources(
    out: &mut String,
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        if let Some(resource) =
            opaque_lease_resource(&to_java_name(&param.name), &effective_param_type(param), opaque_types)
        {
            out.push_str(&resource);
            out.push_str(";\n");
        }
    }
}

fn emit_visitor_param_marshalling(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        marshal_param_to_ffi(
            out,
            &to_java_name(&param.name),
            &effective_param_type(param),
            opaque_types,
            prefix,
            "nativeResources",
        );
    }
}

fn visitor_call_args(
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    func.params
        .iter()
        .flat_map(|param| {
            if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
                vec!["MemorySegment.NULL".to_owned()]
            } else {
                ffi_param_args(&to_java_name(&param.name), &effective_param_type(param), opaque_types)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_visitor_json_return(out: &mut String, func: &FunctionDef) {
    out.push_str("                String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
    if let Some(type_name) = return_type_name(&func.return_type) {
        if matches!(func.return_type, TypeRef::Optional(_)) {
            out.push_str("                return Optional.ofNullable(MAPPER.readValue(json, ");
        } else {
            out.push_str("                return MAPPER.readValue(json, ");
        }
        out.push_str(type_name);
        out.push_str(if matches!(func.return_type, TypeRef::Optional(_)) {
            ".class));\n"
        } else {
            ".class);\n"
        });
    } else {
        out.push_str("                return MAPPER.readValue(json, Object.class);\n");
    }
}

fn emit_visitor_handle_setup(
    out: &mut String,
    prefix_upper: &str,
    exception_class: &str,
    bridge: &VisitorFunctionBridge,
) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_visitor_create.jinja",
        crate::alef_context! { pu => prefix_upper },
    ));
    out.push_str("            if (visitorHandle.equals(MemorySegment.NULL)) {\n                if (!");
    out.push_str(&bridge.options_param_c);
    out.push_str(".equals(MemorySegment.NULL)) {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_free.jinja",
        crate::alef_context! { pu => prefix_upper, options_ptr => &bridge.options_param_c, options_type_handle => &bridge.options_type_handle },
    ));
    out.push_str("                }\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_throw_on_null.jinja",
        crate::alef_context! { exception_class },
    ));
    out.push_str("            }\n\n");
}

fn emit_visitor_result_conversion(out: &mut String, func: &FunctionDef, prefix_upper: &str) {
    out.push_str("                if (resultPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n                    return null;\n                }\n");
    let result_type_handle = return_type_name(&func.return_type)
        .map(|name| name.to_snake_case().to_uppercase())
        .unwrap_or_else(|| "OBJECT".to_owned());
    out.push_str("                nativeResources.register(resultPtr, handle -> NativeLib.");
    out.push_str(prefix_upper);
    out.push('_');
    out.push_str(&result_type_handle);
    out.push_str("_FREE.invoke(handle));\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_to_json.jinja",
        crate::alef_context! { pu => prefix_upper, result_type_handle },
    ));
    out.push_str("                // CPD-OFF\n");
    out.push_str("                if (jsonPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n                    return null;\n                }\n");
    out.push_str("                nativeResources.register(jsonPtr, handle -> NativeLib.");
    out.push_str(prefix_upper);
    out.push_str("_FREE_STRING.invoke(handle));\n");
    emit_visitor_json_return(out, func);
    out.push_str("                // CPD-ON\n");
}

fn emit_visitor_cleanup(out: &mut String, prefix_upper: &str, exception_class: &str) {
    super::error_catch::emit_visitor_operation_catch_chain(out, exception_class);
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_visitor_cleanup.jinja",
        crate::alef_context! { pu => prefix_upper },
    ));
    super::error_catch::emit_method_catch_chain(out, exception_class);
    out.push_str("    }\n");
}

fn emit_visitor_method_open(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    exception_class: &str,
    params: &BridgeParamContext<'_>,
    bridge: &VisitorFunctionBridge,
) {
    out.push_str(&crate::backends::java::template_env::render(
        "convert_with_visitor_signature.jinja",
        crate::alef_context! {
            return_type => java_return_type(&func.return_type),
            method_name => &bridge.internal_method_name,
            params => visitor_method_params(func, params.bridge_param_names, params.bridge_type_aliases),
            exception_class,
        },
    ));
    out.push_str("        try (var arena = Arena.ofShared();\n");
    out.push_str("             var nativeResources = new NativeResources();\n");
    emit_visitor_resources(
        out,
        func,
        params.opaque_types,
        params.bridge_param_names,
        params.bridge_type_aliases,
    );
    out.push_str("             var bridge = new VisitorBridge(");
    out.push_str(&bridge.options_param_java);
    out.push('.');
    out.push_str(&bridge.options_field_java);
    out.push_str("())) {\n");
    emit_visitor_param_marshalling(
        out,
        func,
        prefix,
        params.opaque_types,
        params.bridge_param_names,
        params.bridge_type_aliases,
    );
}

fn emit_visitor_operation(
    out: &mut String,
    func: &FunctionDef,
    prefix_upper: &str,
    options_set_handle: &str,
    params: &BridgeParamContext<'_>,
    bridge: &VisitorFunctionBridge,
) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_set_visitor.jinja",
        crate::alef_context! { handle_name => options_set_handle, options_ptr => &bridge.options_param_c },
    ));
    let ffi_handle = format!("NativeLib.{}_{}", prefix_upper, func.name.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_ptr_call.jinja",
        crate::alef_context! {
            ffi_handle,
            args => visitor_call_args(func, params.opaque_types, params.bridge_param_names, params.bridge_type_aliases),
        },
    ));
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_free_conditional.jinja",
        crate::alef_context! { pu => prefix_upper, options_ptr => &bridge.options_param_c, options_type_handle => &bridge.options_type_handle },
    ));
    emit_visitor_result_conversion(out, func, prefix_upper);
}

pub(super) fn gen_convert_with_visitor_internal_method(
    func: &FunctionDef,
    class_name: &str,
    prefix: &str,
    params: &BridgeParamContext<'_>,
    visitor_bridge: &VisitorFunctionBridge,
) -> String {
    let mut out = String::with_capacity(2048);
    let pu = prefix.to_uppercase();
    let options_set_handle = format!(
        "{}_OPTIONS_SET_{}",
        pu,
        visitor_bridge.options_field_native.to_uppercase()
    );
    let exc = format!("{class_name}Exception");
    emit_visitor_method_open(&mut out, func, prefix, &exc, params, visitor_bridge);
    out.push('\n');
    emit_visitor_handle_setup(&mut out, &pu, &exc, visitor_bridge);
    super::error_catch::emit_visitor_operation_open(&mut out, &exc);
    emit_visitor_operation(&mut out, func, &pu, &options_set_handle, params, visitor_bridge);
    emit_visitor_cleanup(&mut out, &pu, &exc);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitor_cleanup_preserves_primary_and_attempts_every_release() {
        let func = FunctionDef {
            name: "convert".into(),
            return_type: TypeRef::Named("ResultRecord".into()),
            ..Default::default()
        };
        let bridge = VisitorFunctionBridge {
            options_param_java: "options".into(),
            options_param_c: "cOptions".into(),
            options_type_handle: "OPTIONS".into(),
            options_field_java: "visitor".into(),
            options_field_native: "visitor".into(),
            internal_method_name: "convertWithVisitorInternal".into(),
        };
        let params = BridgeParamContext {
            opaque_types: &AHashSet::new(),
            bridge_param_names: &HashSet::new(),
            bridge_type_aliases: &HashSet::new(),
        };
        let generated = gen_convert_with_visitor_internal_method(&func, "SampleRs", "sample", &params, &bridge);

        let visitor_free = generated.find("SAMPLE_VISITOR_FREE.invoke").unwrap();
        let bridge_error = generated.find("bridge.rethrowVisitorError()").unwrap();
        assert!(visitor_free < bridge_error, "{generated}");
        assert!(
            generated.contains("operationFailure.addSuppressed(aggregate)"),
            "{generated}"
        );
    }
}
