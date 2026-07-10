use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_java_name;
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::super::helpers::{emit_javadoc_with_throws, is_bridge_param_java, render_nullable_type};
use super::super::marshal::{
    ffi_param_args, is_bytes_result, is_ffi_string_return, java_ffi_return_cast, java_ffi_return_expr,
    marshal_param_to_ffi,
};
use super::params_returns::public_arg_names;
use super::visitor_bridge::VisitorFunctionBridge;

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(super) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
    clear_fn_handles: &AHashMap<String, String>,
    capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
) {
    gen_sync_function_method_with_visitor(
        out,
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        has_visitor_bridge,
        clear_fn_handles,
        None,
        capsule_types,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_sync_function_method_with_visitor(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
    clear_fn_handles: &AHashMap<String, String>,
    visitor_bridge: Option<&VisitorFunctionBridge>,
    capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
) {
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

    if let Some(capsule_cfg) = capsule_return_config(func, capsule_types) {
        return gen_capsule_function_method(
            out,
            func,
            prefix,
            class_name,
            opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            capsule_cfg,
        );
    }

    let return_type = java_return_type(&func.return_type);
    let exception_class_name = format!("{}Exception", class_name);
    emit_javadoc_with_throws(out, &func.doc, "    ", &exception_class_name);
    let method_sig = crate::backends::java::template_env::render(
        "ffi_method_signature.jinja",
        minijinja::context! {
            return_type => return_type,
            method_name => to_java_name(&func.name),
            params => params.join(", "),
            exception_class => exception_class_name,
        },
    );
    out.push_str(&method_sig);

    if has_visitor_bridge && let Some(visitor_bridge) = visitor_bridge {
        out.push_str("        if (");
        out.push_str(&visitor_bridge.options_param_java);
        out.push_str(" != null && ");
        out.push_str(&visitor_bridge.options_param_java);
        out.push('.');
        out.push_str(&visitor_bridge.options_field_java);
        out.push_str("() != null) {\n");
        out.push_str("            return ");
        out.push_str(&visitor_bridge.internal_method_name);
        out.push('(');
        out.push_str(&public_arg_names(func, bridge_param_names, bridge_type_aliases).join(", "));
        out.push_str(");\n");
        out.push_str("        }\n");
        out.push('\n');
    }

    out.push_str(&crate::backends::java::template_env::render(
        "ffi_try_finally_block_start.jinja",
        minijinja::context! {},
    ));

    let ffi_ptr_params: Vec<(String, String)> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .filter_map(|p| {
            let inner_name = match &p.ty {
                TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if !opaque_types.contains(n.as_str()) {
                            Some(n.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            inner_name.map(|type_name| {
                let cname = "c".to_string() + &to_java_name(&p.name);
                let type_snake = type_name.to_snake_case();
                let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
                (cname, free_handle)
            })
        })
        .collect();

    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
            TypeRef::Optional(Box::new(param.ty.clone()))
        } else {
            param.ty.clone()
        };
        marshal_param_to_ffi(out, &to_java_name(&param.name), &effective_ty, opaque_types, prefix);
    }

    let ffi_handle = match clear_fn_handles.get(&func.name) {
        Some(handle) => format!("NativeLib.{}", handle),
        None => format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase()),
    };

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

    let emit_ffi_ptr_cleanup = |out: &mut String| {
        for (cname, free_handle) in &ffi_ptr_params {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_null_check_with_cleanup.jinja",
                minijinja::context! {
                    var => cname,
                    free_handle => free_handle,
                },
            ));
        }
    };

    let (is_optional_return, dispatch_return_type) = match &func.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    let is_clear_fn = clear_fn_handles.contains_key(&func.name);

    if matches!(dispatch_return_type, TypeRef::Unit) && !is_clear_fn {
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_invoke_void.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        if func.error_type.is_some() {
            out.push_str("            checkLastError();\n");
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if is_clear_fn {
        out.push_str("            var outErr = arena.allocate(ValueLayout.ADDRESS);\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_invoke_primitive_result.jinja",
            minijinja::context! {
                cast_type => "int",
                ffi_handle => &ffi_handle,
                call_args => {
                    let mut args = call_args.clone();
                    args.push("outErr".to_string());
                    args.join(", ")
                },
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str("            if (primitiveResult != 0) {\n");
        out.push_str("                MemorySegment errPtr = outErr.get(ValueLayout.ADDRESS, 0);\n");
        out.push_str("                String msg = errPtr.equals(MemorySegment.NULL) ? \"clear failed (rc=\" + primitiveResult + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
        out.push_str("                throw new ");
        out.push_str(&exception_class_name);
        out.push_str("(primitiveResult, msg);\n");
        out.push_str("            }\n");
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if is_ffi_string_return(&dispatch_return_type) {
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_result_ptr_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_null_check.jinja",
            minijinja::context! {
                var => "resultPtr",
                optional => is_optional_return,
            },
        ));
        let len_handle = format!("NativeLib.{}_{}_LEN", prefix.to_uppercase(), func.name.to_uppercase());
        out.push_str("            long resultLen = (long) ");
        out.push_str(&len_handle);
        out.push_str(".invoke(");
        out.push_str(&call_args.join(", "));
        out.push_str(");\n");
        out.push_str("            String str = readCString(resultPtr, resultLen);\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_invoke_free.jinja",
            minijinja::context! {
                free_handle => &free_handle,
                ptr => "resultPtr",
            },
        ));
        let return_expr = if matches!(dispatch_return_type, TypeRef::Path) {
            "java.nio.file.Path.of(str)"
        } else {
            "str"
        };
        if is_optional_return {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_optional_expr.jinja",
                minijinja::context! {
                    expr => return_expr,
                },
            ));
        } else {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_expr.jinja",
                minijinja::context! {
                    expr => return_expr,
                },
            ));
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Named(_)) {
        let return_type_name = match &dispatch_return_type {
            TypeRef::Named(name) => name,
            _ => unreachable!(),
        };
        let is_opaque = opaque_types.contains(return_type_name.as_str());

        out.push_str(&crate::backends::java::template_env::render(
            "ffi_result_ptr_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_null_check.jinja",
            minijinja::context! {
                var => "resultPtr",
                optional => is_optional_return,
            },
        ));

        if is_opaque {
            if is_optional_return {
                out.push_str(&crate::backends::java::template_env::render(
                    "ffi_return_new_handle.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::java::template_env::render(
                    "ffi_return_new_instance.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            }
        } else {
            // NOTE: _content only returns the markdown string field, not a full JSON object.
            let type_snake = return_type_name.to_snake_case();
            let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
            let to_json_handle = format!(
                "NativeLib.{}_{}_TO_JSON",
                prefix.to_uppercase(),
                type_snake.to_uppercase()
            );
            out.push_str("            // CPD-OFF\n");
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_invoke_json_ptr.jinja",
                minijinja::context! {
                    to_json_handle => &to_json_handle,
                },
            ));
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_invoke_free.jinja",
                minijinja::context! {
                    free_handle => &free_handle,
                    ptr => "resultPtr",
                },
            ));
            out.push_str("            if (jsonPtr.equals(MemorySegment.NULL)) {\n");
            out.push_str("                checkLastError();\n");
            if is_optional_return {
                out.push_str("                return Optional.empty();\n");
            } else {
                out.push_str("                throw new ");
                out.push_str(&exception_class_name);
                out.push_str("(\"");
                out.push_str(&to_java_name(&func.name));
                out.push_str(": failed to serialize response\", null);\n");
            }
            out.push_str("            }\n");
            out.push_str("            String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_invoke_free_string.jinja",
                minijinja::context! {
                    prefix => prefix.to_uppercase(),
                },
            ));
            if is_optional_return {
                out.push_str(&crate::backends::java::template_env::render(
                    "ffi_return_mapper_read_optional.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::java::template_env::render(
                    "ffi_return_mapper_read.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            }
            out.push_str("            // CPD-ON\n");
        }

        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Vec(_)) {
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_result_ptr_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        let element_type = match &dispatch_return_type {
            TypeRef::Vec(inner) => java_boxed_type(inner),
            _ => unreachable!(),
        };
        let type_ref = format!(
            "new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }}",
            element_type
        );
        if is_optional_return {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_read_json_list_optional.jinja",
                minijinja::context! {
                    type_ref => &type_ref,
                },
            ));
        } else {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_read_json_list_plain.jinja",
                minijinja::context! {
                    type_ref => &type_ref,
                },
            ));
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Bytes) && is_bytes_result(func) {
        let free_bytes_handle = format!("NativeLib.{}_FREE_BYTES", prefix.to_uppercase());
        let args_with_sep = if call_args.is_empty() {
            String::new()
        } else {
            format!("{}, ", call_args.join(", "))
        };
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::backends::java::template_env::render(
            "bytes_result_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => &args_with_sep,
                free_bytes_handle => &free_bytes_handle,
                optional => is_optional_return,
            },
        ));
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else {
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_invoke_primitive_result.jinja",
            minijinja::context! {
                cast_type => java_ffi_return_cast(&dispatch_return_type),
                ffi_handle => &ffi_handle,
                call_args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        if func.error_type.is_some() {
            out.push_str("            checkLastError();\n");
        }
        if is_optional_return {
            let return_expr = java_ffi_return_expr(&dispatch_return_type, "primitiveResult");
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_primitive_result.jinja",
                minijinja::context! {
                    return_expr => format!("Optional.of({return_expr})"),
                },
            ));
        } else {
            let return_expr = java_ffi_return_expr(&dispatch_return_type, "primitiveResult");
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_return_primitive_result.jinja",
                minijinja::context! {
                    return_expr => return_expr,
                },
            ));
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::backends::java::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    }

    out.push_str("    }\n");
}

/// Returns the capsule config for a function's return type if it is a capsule type,
/// otherwise returns None.
fn capsule_return_config<'a>(
    func: &FunctionDef,
    capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
) -> Option<&'a HostCapsuleTypeConfig> {
    if let TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Generate a Java wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer.
/// The wrapper converts parameters, calls the C function, and constructs the host `Language`
/// from the raw pointer — never an opaque alef handle.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_capsule_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    cfg: &HostCapsuleTypeConfig,
) {
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

    let return_type = match cfg.required_host_type("Language", "java") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("    // ALEF ERROR: {e}\n"));
            return;
        }
    };

    let exception_class_name = format!("{}Exception", class_name);
    emit_javadoc_with_throws(out, &func.doc, "    ", &exception_class_name);
    let method_sig = crate::backends::java::template_env::render(
        "ffi_method_signature.jinja",
        minijinja::context! {
            return_type => &return_type,
            method_name => to_java_name(&func.name),
            params => params.join(", "),
            exception_class => exception_class_name,
        },
    );
    out.push_str(&method_sig);

    out.push_str(&crate::backends::java::template_env::render(
        "ffi_try_finally_block_start.jinja",
        minijinja::context! {},
    ));

    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
            TypeRef::Optional(Box::new(param.ty.clone()))
        } else {
            param.ty.clone()
        };
        marshal_param_to_ffi(out, &to_java_name(&param.name), &effective_ty, opaque_types, prefix);
    }

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

    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_ptr_call.jinja",
        minijinja::context! {
            ffi_handle => &ffi_handle,
            args => call_args.join(", "),
        },
    ));

    out.push_str(&crate::backends::java::template_env::render(
        "ffi_null_check.jinja",
        minijinja::context! {
            var => "resultPtr",
            optional => false,
        },
    ));

    let construct = match cfg.construct_required("resultPtr", "Language", "java") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("            // ALEF ERROR: {e}\n"));
            out.push_str("        } catch (Throwable e) {\n");
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_throw_exception.jinja",
                minijinja::context! {
                    exception_class => format!("{}Exception", class_name),
                },
            ));
            out.push_str("        }\n");
            out.push_str("    }\n");
            return;
        }
    };
    out.push_str(&format!("            return {construct};\n"));

    out.push_str("        } catch (Throwable e) {\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_throw_exception.jinja",
        minijinja::context! {
            exception_class => format!("{}Exception", class_name),
        },
    ));
    out.push_str("        }\n");
    out.push_str("    }\n");
}

#[cfg(test)]
mod capsule_tests {
    use super::*;
    use crate::core::ir::ParamDef;

    fn get_language_fn() -> FunctionDef {
        FunctionDef {
            name: "get_language".to_string(),
            rust_path: "sample::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_cfg(host_type: &str, construct_expr: &str) -> HostCapsuleTypeConfig {
        HostCapsuleTypeConfig {
            host_type: host_type.to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: construct_expr.to_string(),
        }
    }

    #[test]
    fn capsule_method_emits_configured_host_type_and_construct_expr() {
        let func = get_language_fn();
        let cfg = make_cfg(
            "io.github.example.jtreesitter.Language",
            "new io.github.example.jtreesitter.Language({ptr})",
        );
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            &func,
            "tsp",
            "LanguagePack",
            &AHashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &cfg,
        );
        assert!(
            out.contains("io.github.example.jtreesitter.Language"),
            "must use configured host_type. Got:\n{out}"
        );
        assert!(
            out.contains("new io.github.example.jtreesitter.Language(resultPtr)"),
            "must use configured construct_expr with ptr substituted. Got:\n{out}"
        );
    }

    #[test]
    fn capsule_method_errors_when_host_type_empty() {
        let func = get_language_fn();
        let cfg = make_cfg("", "new MyLanguage({ptr})");
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            &func,
            "tsp",
            "LanguagePack",
            &AHashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &cfg,
        );
        assert!(
            out.contains("ALEF ERROR"),
            "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
        );
        assert!(
            out.contains("host_type"),
            "error must mention the missing field. Got:\n{out}"
        );
    }

    #[test]
    fn capsule_method_errors_when_construct_expr_empty() {
        let func = get_language_fn();
        let cfg = make_cfg("io.github.example.jtreesitter.Language", "");
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            &func,
            "tsp",
            "LanguagePack",
            &AHashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &cfg,
        );
        assert!(
            out.contains("ALEF ERROR"),
            "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
        );
        assert!(
            out.contains("construct_expr"),
            "error must mention the missing field. Got:\n{out}"
        );
    }
}
