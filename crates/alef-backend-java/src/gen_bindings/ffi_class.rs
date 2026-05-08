use crate::type_map::{java_boxed_type, java_return_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_java_name;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

use super::helpers::is_bridge_param_java;
use super::marshal::{
    ffi_param_name, gen_helper_methods, is_bytes_result, is_ffi_string_return, java_ffi_return_cast,
    marshal_param_to_ffi,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_main_class(
    api: &ApiSurface,
    _config: &ResolvedCrateConfig,
    package: &str,
    class_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
) -> String {
    // Build the set of opaque type names so we can distinguish opaque handles from records
    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(4096);

    let header_out = crate::template_env::render(
        "ffi_main_class_header.jinja",
        minijinja::context! { class_name => class_name },
    );
    body.push_str(&header_out);
    body.push('\n');

    // Generate static methods for free functions
    for func in &api.functions {
        // Always generate sync method (bridge params stripped from signature)
        gen_sync_function_method(
            &mut body,
            func,
            prefix,
            class_name,
            &opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_bridge,
        );
        body.push('\n');

        // Also generate async wrapper if marked as async
        if func.is_async {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            body.push('\n');
        }
    }

    // Add internal convertWithVisitor helper when visitor bridge is configured
    if has_visitor_bridge {
        body.push_str(&gen_convert_with_visitor_internal_method(class_name, prefix));
        body.push('\n');
    }

    // Add helper methods only if they are referenced in the body
    gen_helper_methods(&mut body, prefix, class_name);

    let footer_out = crate::template_env::render("ffi_main_class_footer.jinja", minijinja::context! {});
    body.push_str(&footer_out);

    // Now assemble the file with only the imports that are actually used in the body.
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::template_env::render(
        "ffi_imports.jinja",
        minijinja::context! {
            header => header,
            package => package,
            needs_arena => body.contains("Arena"),
            needs_function_descriptor => body.contains("FunctionDescriptor"),
            needs_linker => body.contains("Linker"),
            needs_memory_segment => body.contains("MemorySegment"),
            needs_symbol_lookup => body.contains("SymbolLookup"),
            needs_value_layout => body.contains("ValueLayout"),
            needs_list => body.contains("List<"),
            needs_map => body.contains("Map<"),
            needs_optional => body.contains("Optional<"),
            needs_hash_map => body.contains("HashMap<") || body.contains("new HashMap"),
            needs_completable_future => body.contains("CompletableFuture"),
            needs_completion_exception => body.contains("CompletionException"),
            needs_object_mapper => body.contains(" ObjectMapper"),
        },
    );

    out.push_str(&body);

    out
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
) {
    // Exclude bridge params from the public Java signature. Optional params
    // take the boxed Java type (Integer/Long/Boolean/...) so callers can pass
    // `null` to skip them.
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
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = java_return_type(&func.return_type);
    let exception_class_name = format!("{}Exception", class_name);
    let method_sig = crate::template_env::render(
        "ffi_method_signature.jinja",
        minijinja::context! {
            return_type => return_type,
            method_name => to_java_name(&func.name),
            params => params.join(", "),
            exception_class => exception_class_name,
        },
    );
    out.push_str(&method_sig);

    // Check if this is the convert function with visitor support
    let is_convert_with_visitor_support = has_visitor_bridge
        && func.name == "convert"
        && func
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(n) if n == "ConversionOptions"));

    // For convert with visitor, handle delegation at the top level
    if is_convert_with_visitor_support {
        out.push_str("        if (options != null && options.visitor() != null) {\n");
        out.push_str("            return convertWithVisitorInternal(html, options);\n");
        out.push_str("        }\n");
        out.push('\n');
    }

    out.push_str(&crate::template_env::render(
        "ffi_try_finally_block_start.jinja",
        minijinja::context! {},
    ));

    // Collect non-opaque Named params that need FFI pointer cleanup after the call.
    // These are Rust-allocated by _from_json and must be freed with _free.
    // Bridge params are excluded — they are passed as NULL.
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

    // Marshal non-bridge parameters (use camelCase Java names)
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        // When a parameter is optional (Option<T> in Rust), wrap the TypeRef so that
        // marshal_param_to_ffi generates a null-safe allocation path.
        let effective_ty = if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
            TypeRef::Optional(Box::new(param.ty.clone()))
        } else {
            param.ty.clone()
        };
        marshal_param_to_ffi(out, &to_java_name(&param.name), &effective_ty, opaque_types, prefix);
    }

    // Call FFI
    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

    // Build call args: bridge params get MemorySegment.NULL, others are marshalled normally.
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if is_bridge_param_java(p, bridge_param_names, bridge_type_aliases) {
                "MemorySegment.NULL".to_string()
            } else {
                // Apply the same optional-wrapping logic used when marshalling.
                let effective_ty = if p.optional && !matches!(p.ty, TypeRef::Optional(_)) {
                    TypeRef::Optional(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                };
                ffi_param_name(&to_java_name(&p.name), &effective_ty, opaque_types)
            }
        })
        .collect();

    // Emit a helper closure to free FFI-allocated param pointers (e.g. options created by _from_json)
    let emit_ffi_ptr_cleanup = |out: &mut String| {
        for (cname, free_handle) in &ffi_ptr_params {
            out.push_str(&crate::template_env::render(
                "ffi_null_check_with_cleanup.jinja",
                minijinja::context! {
                    var => cname,
                    free_handle => free_handle,
                },
            ));
        }
    };

    // Unwrap Optional<T> to determine the actual dispatch type and whether we're optional.
    let (is_optional_return, dispatch_return_type) = match &func.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    if matches!(dispatch_return_type, TypeRef::Unit) {
        out.push_str(&crate::template_env::render(
            "ffi_invoke_void.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if is_ffi_string_return(&dispatch_return_type) {
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        out.push_str(&crate::template_env::render(
            "ffi_result_ptr_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::template_env::render(
            "ffi_null_check.jinja",
            minijinja::context! {
                var => "resultPtr",
                optional => is_optional_return,
            },
        ));
        out.push_str("            String str = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
        out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
                "ffi_return_optional_expr.jinja",
                minijinja::context! {
                    expr => return_expr,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "ffi_return_expr.jinja",
                minijinja::context! {
                    expr => return_expr,
                },
            ));
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Named(_)) {
        // Named return types: FFI returns a struct pointer.
        let return_type_name = match &dispatch_return_type {
            TypeRef::Named(name) => name,
            _ => unreachable!(),
        };
        let is_opaque = opaque_types.contains(return_type_name.as_str());

        out.push_str(&crate::template_env::render(
            "ffi_result_ptr_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::template_env::render(
            "ffi_null_check.jinja",
            minijinja::context! {
                var => "resultPtr",
                optional => is_optional_return,
            },
        ));

        if is_opaque {
            // Opaque handles: wrap the raw pointer directly, caller owns and will close()
            if is_optional_return {
                out.push_str(&crate::template_env::render(
                    "ffi_return_new_handle.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "ffi_return_new_instance.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            }
        } else {
            // Record types: use _to_json to serialize the full struct to JSON, then deserialize.
            // NOTE: _content only returns the markdown string field, not a full JSON object.
            let type_snake = return_type_name.to_snake_case();
            let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
            let to_json_handle = format!(
                "NativeLib.{}_{}_TO_JSON",
                prefix.to_uppercase(),
                type_snake.to_uppercase()
            );
            out.push_str(&crate::template_env::render(
                "ffi_invoke_json_ptr.jinja",
                minijinja::context! {
                    to_json_handle => &to_json_handle,
                },
            ));
            out.push_str(&crate::template_env::render(
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
                out.push_str("                return null;\n");
            }
            out.push_str("            }\n");
            out.push_str("            String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
            out.push_str(&crate::template_env::render(
                "ffi_invoke_free_string.jinja",
                minijinja::context! {
                    prefix => prefix.to_uppercase(),
                },
            ));
            if is_optional_return {
                out.push_str(&crate::template_env::render(
                    "ffi_return_mapper_read_optional.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "ffi_return_mapper_read.jinja",
                    minijinja::context! {
                        class_name => return_type_name,
                    },
                ));
            }
        }

        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Vec(_)) {
        // Vec return types: FFI returns a JSON string pointer; deserialize into List<T>.
        // The body is delegated to a single `readJsonList` helper emitted by
        // `gen_helper_methods` so the JSON-deserialize boilerplate isn't duplicated
        // at every call site (which CPD flagged as copy-paste duplication).
        out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
                "ffi_return_read_json_list_optional.jinja",
                minijinja::context! {
                    type_ref => &type_ref,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "ffi_return_read_json_list_plain.jinja",
                minijinja::context! {
                    type_ref => &type_ref,
                },
            ));
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else if matches!(dispatch_return_type, TypeRef::Bytes) && is_bytes_result(func) {
        // Bytes-result functions use the out-param convention:
        //   (inputs..., out_ptr: *mut *mut u8, out_len: *mut usize, out_cap: *mut usize) -> i32
        // Never use the old direct-pointer pattern (FREE_STRING / byteSize()) for these.
        let free_bytes_handle = format!("NativeLib.{}_FREE_BYTES", prefix.to_uppercase());
        let args_with_sep = if call_args.is_empty() {
            String::new()
        } else {
            format!("{}, ", call_args.join(", "))
        };
        emit_ffi_ptr_cleanup(out);
        out.push_str(&crate::template_env::render(
            "bytes_result_call.jinja",
            minijinja::context! {
                ffi_handle => &ffi_handle,
                args => &args_with_sep,
                free_bytes_handle => &free_bytes_handle,
                optional => is_optional_return,
            },
        ));
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    } else {
        // Primitive return types (including boxed types for Optional)
        out.push_str(&crate::template_env::render(
            "ffi_invoke_primitive_result.jinja",
            minijinja::context! {
                cast_type => java_ffi_return_cast(&dispatch_return_type),
                ffi_handle => &ffi_handle,
                call_args => call_args.join(", "),
            },
        ));
        emit_ffi_ptr_cleanup(out);
        if is_optional_return {
            out.push_str("            return Optional.of(primitiveResult);\n");
        } else {
            out.push_str("            return primitiveResult;\n");
        }
        out.push_str("        } catch (Throwable e) {\n");
        out.push_str(&crate::template_env::render(
            "ffi_throw_exception.jinja",
            minijinja::context! {
                exception_class => format!("{}Exception", class_name),
            },
        ));
        out.push_str("        }\n");
    }

    out.push_str("    }\n");
}

pub(crate) fn gen_async_wrapper_method(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = match &func.return_type {
        TypeRef::Unit => "Void".to_string(),
        other => java_boxed_type(other).to_string(),
    };

    let sync_method_name = to_java_name(&func.name);
    let async_method_name = format!("{}Async", sync_method_name);
    let _param_names: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect();

    out.push_str(&crate::template_env::render(
        "ffi_async_method_signature.jinja",
        minijinja::context! {
            return_type => &return_type,
            async_method_name => &async_method_name,
            params => params.join(", "),
        },
    ));
    out.push_str("        return CompletableFuture.supplyAsync(() -> {\n");
    out.push_str("    }\n");
}

/// Generate the internal convertWithVisitor method to delegate visitor handling.
///
/// When the public convert method detects a non-null visitor in options,
/// it delegates to this private method which creates the VisitorBridge
/// and calls htm_convert_with_visitor internally.
fn gen_convert_with_visitor_internal_method(class_name: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(2048);
    let pu = prefix.to_uppercase();
    let exc = format!("{class_name}Exception");

    out.push_str(&crate::template_env::render(
        "convert_with_visitor_signature.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("        try (var arena = Arena.ofConfined();\n");
    out.push_str("            var cHtml = arena.allocateFrom(html);\n");
    out.push('\n');
    out.push_str("            MemorySegment optionsPtr = MemorySegment.NULL;\n");
    out.push_str("            if (options != null) {\n");
    out.push_str("                var optJson = arena.allocateFrom(MAPPER.writeValueAsString(options));\n");
    out.push_str(&crate::template_env::render(
        "ffi_conversion_options_invoke.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("            }\n");
    out.push_str("            if (optionsPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                var defaultJson = arena.allocateFrom(\"{}\");\n");
    out.push_str(&crate::template_env::render(
        "ffi_conversion_options_invoke.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("            }\n");
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "ffi_visitor_create.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("            if (visitorHandle.equals(MemorySegment.NULL)) {\n");
    out.push_str("                if (!optionsPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str(&crate::template_env::render(
        "ffi_options_free.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("                }\n");
    out.push_str(&crate::template_env::render(
        "ffi_throw_on_null.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            }\n");
    out.push('\n');
    out.push_str("            try {\n");
    out.push_str(&crate::template_env::render(
        "ffi_options_set_visitor.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str(&crate::template_env::render(
        "ffi_convert_invoke.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str(&crate::template_env::render(
        "ffi_options_free_conditional.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("                if (resultPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n");
    out.push_str("                    return null;\n");
    out.push_str("                }\n");
    out.push_str(&crate::template_env::render(
        "ffi_result_to_json.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str(&crate::template_env::render(
        "ffi_result_free.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("                if (jsonPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                    checkLastError();\n");
    out.push_str("                    return null;\n");
    out.push_str("                }\n");
    out.push_str("                String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
    out.push_str(&crate::template_env::render(
        "ffi_invoke_free_string.jinja",
        minijinja::context! {
            prefix => &pu,
        },
    ));
    out.push_str("                return MAPPER.readValue(json, ConversionResult.class);\n");
    out.push_str("            } catch (Throwable e) {\n");
    out.push_str(&crate::template_env::render(
        "ffi_throw_inner.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            } finally {\n");
    out.push_str(&crate::template_env::render(
        "ffi_visitor_free.jinja",
        minijinja::context! {
            pu => &pu,
        },
    ));
    out.push_str("                bridge.rethrowVisitorError();\n");
    out.push_str("            }\n");
    out.push_str(&crate::template_env::render(
        "ffi_catch_exception.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("            throw e;\n");
    out.push_str("        } catch (Throwable e) {\n");
    out.push_str(&crate::template_env::render(
        "ffi_throw_outer.jinja",
        minijinja::context! {
            exception_class => &exc,
        },
    ));
    out.push_str("        }\n");
    out.push_str("    }\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_opaque_types() -> AHashSet<String> {
        AHashSet::new()
    }

    fn create_test_bridge_sets() -> (HashSet<String>, HashSet<String>) {
        (HashSet::new(), HashSet::new())
    }

    fn create_test_function(name: &str, return_type: TypeRef) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test::{}", name),
            original_rust_path: String::new(),
            params: vec![],
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

    #[test]
    fn test_optional_string_return_emits_optional_empty() {
        let func = create_test_function("get_name", TypeRef::Optional(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(str);"));
    }

    #[test]
    fn test_optional_named_return_emits_optional_wrappers() {
        let func = create_test_function(
            "get_preset",
            TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string()))),
        );

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(MAPPER.readValue(json, EmbeddingPreset.class));"));
    }

    #[test]
    fn test_optional_vec_return_emits_optional_list() {
        let func = create_test_function(
            "list_items",
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
        );

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        // Vec returns now go through the readJsonList helper to deduplicate
        // the JSON-deserialize boilerplate (CPD was flagging multiple inline
        // copies). The empty-list-on-null path lives inside the helper.
        assert!(out.contains(
            "return Optional.of(readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
        ));
    }

    #[test]
    fn test_optional_bytes_result_emits_out_param_pattern() {
        // Bytes with error_type → out-param convention: i32 return + 3 trailing out-params.
        let func = FunctionDef {
            name: "get_data".to_string(),
            rust_path: "test::get_data".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Optional(Box::new(TypeRef::Bytes)),
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("outPtrHolder"), "should allocate outPtrHolder");
        assert!(out.contains("outLenHolder"), "should allocate outLenHolder");
        assert!(out.contains("outCapHolder"), "should allocate outCapHolder");
        assert!(out.contains("int rc = (int)"), "should have int rc");
        assert!(out.contains("TEST_FREE_BYTES"), "should call FREE_BYTES");
        assert!(out.contains("return Optional.empty();"));
        assert!(out.contains("return Optional.of(result);"));
    }

    #[test]
    fn test_bytes_result_emits_out_param_pattern_non_optional() {
        // Non-optional Bytes with error_type → out-param convention.
        let func = FunctionDef {
            name: "render_png".to_string(),
            rust_path: "test::render_png".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("outPtrHolder"), "should allocate outPtrHolder");
        assert!(out.contains("TEST_FREE_BYTES"), "should call FREE_BYTES");
        assert!(!out.contains("FREE_STRING"), "must not use FREE_STRING");
        assert!(out.contains("return result;"), "non-optional plain return");
        assert!(!out.contains("Optional.of(result)"), "must not wrap in Optional");
    }

    #[test]
    fn test_non_optional_string_return_no_optional_wrapper() {
        let func = create_test_function("get_name", TypeRef::String);

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("return null;"));
        assert!(out.contains("return str;"));
        assert!(!out.contains("Optional.empty()"));
        assert!(!out.contains("Optional.of(str)"));
    }

    #[test]
    fn test_path_return_wraps_with_path_of() {
        let func = create_test_function("cache_dir", TypeRef::Path);

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("return java.nio.file.Path.of(str);"));
        assert!(!out.contains("return str;"));
    }

    #[test]
    fn test_optional_path_return_wraps_with_path_of() {
        let func = create_test_function("maybe_cache_dir", TypeRef::Optional(Box::new(TypeRef::Path)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        assert!(out.contains("return Optional.of(java.nio.file.Path.of(str));"));
    }

    #[test]
    fn test_non_optional_vec_return_no_optional_wrapper() {
        let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        // The Vec dispatch path now delegates to the readJsonList helper.
        // Optional<List<T>> wrapping is added by the caller; non-optional
        // is a bare call.
        assert!(out.contains(
            "return readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
        ));
        assert!(!out.contains("Optional.of(readJsonList"));
    }

    #[test]
    fn vec_return_uses_helper_not_inline_json_deserialize() {
        // CPD regression: every Vec-returning method previously inlined a
        // ~15-line null-check + reinterpret + free + readValue block, which
        // CPD (rightly) flagged as duplication. The helper extraction means
        // the call site is one line and `readJsonList` appears exactly once
        // in the helper section.
        let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "test",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
        );

        // The previously-duplicated JSON-deserialize line must NOT appear at
        // the call site any more (it now lives only in the helper, which is
        // emitted by gen_helper_methods at the bottom of the class).
        assert!(!out.contains(
            "createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<"
        ));
    }
}
