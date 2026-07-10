use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_java_name;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::HashSet;

use super::super::helpers::{is_bridge_param_java, render_nullable_type};
use super::super::marshal::{ffi_param_args, marshal_param_to_ffi};
use super::params_returns::return_type_name;
use super::visitor_bridge::VisitorFunctionBridge;

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_convert_with_visitor_internal_method(
    func: &FunctionDef,
    class_name: &str,
    prefix: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
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
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty)
            } else {
                java_type(&p.ty)
            };
            let annotated = render_nullable_type(&ptype, p.optional);
            format!("final {annotated} {}", to_java_name(&p.name))
        })
        .collect();
    let return_type = java_return_type(&func.return_type);

    out.push_str(&crate::backends::java::template_env::render(
        "convert_with_visitor_signature.jinja",
        minijinja::context! {
            return_type => &return_type,
            method_name => &visitor_bridge.internal_method_name,
            params => params.join(", "),
            exception_class => &exc,
        },
    ));
    out.push_str("        try (var arena = Arena.ofShared();\n");
    out.push_str("             var bridge = new VisitorBridge(");
    out.push_str(&visitor_bridge.options_param_java);
    out.push('.');
    out.push_str(&visitor_bridge.options_field_java);
    out.push_str("())) {\n");
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
            TypeRef::Optional(Box::new(param.ty.clone()))
        } else {
            param.ty.clone()
        };
        marshal_param_to_ffi(
            &mut out,
            &to_java_name(&param.name),
            &effective_ty,
            opaque_types,
            prefix,
        );
    }
    out.push('\n');
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_visitor_create.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("            if (visitorHandle.equals(MemorySegment.NULL)) {\n");
    out.push_str("                if (!");
    out.push_str(&visitor_bridge.options_param_c);
    out.push_str(".equals(MemorySegment.NULL)) {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_free.jinja",
        minijinja::context! {
            pu => &pu,
            options_ptr => &visitor_bridge.options_param_c,
            options_type_handle => &visitor_bridge.options_type_handle,
        },
    ));
    out.push_str("                }\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_throw_on_null.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            }\n");
    out.push('\n');
    out.push_str("            try {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_set_visitor.jinja",
        minijinja::context! {
            handle_name => &options_set_handle,
            options_ptr => &visitor_bridge.options_param_c,
        },
    ));
    let call_args: Vec<String> = func
        .params
        .iter()
        .flat_map(|p| {
            if is_bridge_param_java(p, bridge_param_names, bridge_type_aliases) {
                vec!["MemorySegment.NULL".to_string()]
            } else {
                let effective_ty = if p.optional && !matches!(p.ty, TypeRef::Optional(_)) {
                    TypeRef::Optional(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                };
                ffi_param_args(&to_java_name(&p.name), &effective_ty, opaque_types)
            }
        })
        .collect();
    let ffi_handle = format!("NativeLib.{}_{}", pu, func.name.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_ptr_call.jinja",
        minijinja::context! {
            ffi_handle => &ffi_handle,
            args => call_args.join(", "),
        },
    ));
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_options_free_conditional.jinja",
        minijinja::context! {
            pu => &pu,
            options_ptr => &visitor_bridge.options_param_c,
            options_type_handle => &visitor_bridge.options_type_handle,
        },
    ));
    out.push_str("                if (resultPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n");
    out.push_str("                    return null;\n");
    out.push_str("                }\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_to_json.jinja",
        minijinja::context! {
            pu => &pu,
            result_type_handle => return_type_name(&func.return_type)
                .map(|name| name.to_snake_case().to_uppercase())
                .unwrap_or_else(|| "OBJECT".to_string()),
        },
    ));
    out.push_str("                // CPD-OFF\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_free.jinja",
        minijinja::context! {
            pu => &pu,
            result_type_handle => return_type_name(&func.return_type)
                .map(|name| name.to_snake_case().to_uppercase())
                .unwrap_or_else(|| "OBJECT".to_string()),
        },
    ));
    out.push_str("                if (jsonPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n");
    out.push_str("                    return null;\n");
    out.push_str("                }\n");
    out.push_str("                String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_free_string.jinja",
        minijinja::context! {
            prefix => &pu,
        },
    ));
    if let Some(return_type_name) = return_type_name(&func.return_type) {
        if matches!(func.return_type, TypeRef::Optional(_)) {
            out.push_str("                return Optional.ofNullable(MAPPER.readValue(json, ");
            out.push_str(return_type_name);
            out.push_str(".class));\n");
        } else {
            out.push_str("                return MAPPER.readValue(json, ");
            out.push_str(return_type_name);
            out.push_str(".class);\n");
        }
    } else {
        out.push_str("                return MAPPER.readValue(json, Object.class);\n");
    }
    out.push_str("                // CPD-ON\n");
    out.push_str("            } catch (Throwable e) {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_throw_inner.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            } finally {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_visitor_free.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("                bridge.rethrowVisitorError();\n");
    out.push_str("            }\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_catch_exception.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            throw e;\n");
    out.push_str("        } catch (Throwable e) {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_throw_outer.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("        }\n");
    out.push_str("    }\n");

    out
}
