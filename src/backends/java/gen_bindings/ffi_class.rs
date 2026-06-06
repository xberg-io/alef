use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_java_name;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;
use std::collections::HashSet;

use super::helpers::{emit_javadoc_with_throws, is_bridge_param_java, render_nullable_type, safe_java_field_name};
use super::marshal::{
    ffi_param_args, gen_helper_methods, is_bytes_result, is_ffi_string_return, java_ffi_return_cast,
    java_ffi_return_expr, marshal_param_to_ffi,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_main_class(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
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

    // Map a trait bridge's `clear_fn` (the core Rust function name, e.g.
    // `clear_text_backends`) to the `NativeLib` handle constant emitted for it.
    // The FFI layer exports the clear function as `{prefix}_clear_{trait_snake}`
    // (singular, derived from the trait name) and `NativeLib.java` declares the
    // matching handle constant as `{PREFIX}_CLEAR_{TRAIT_SNAKE}`. The free-function
    // facade body must reference that same constant rather than deriving the name
    // from `func.name` (which is the plural core Rust function name).
    let clear_fn_handles: AHashMap<String, String> = config
        .trait_bridges
        .iter()
        .filter_map(|b| {
            b.clear_fn.as_ref().map(|clear_fn| {
                let trait_snake_upper = b.trait_name.to_snake_case().to_uppercase();
                let handle = format!("{}_CLEAR_{}", prefix.to_uppercase(), trait_snake_upper);
                (clear_fn.clone(), handle)
            })
        })
        .collect();

    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(4096);

    let header_out = crate::backends::java::template_env::render(
        "ffi_main_class_header.jinja",
        minijinja::context! { class_name => class_name },
    );
    body.push_str(&header_out);
    body.push('\n');

    // Generate static methods for free functions
    for func in &api.functions {
        // Always generate sync method (bridge params stripped from signature)
        gen_sync_function_method_with_visitor(
            &mut body,
            func,
            prefix,
            class_name,
            &opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_bridge,
            &clear_fn_handles,
            visitor_bridge_for_function(func, config).as_ref(),
        );
        body.push('\n');

        // Also generate async wrapper if marked as async
        if func.is_async {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            body.push('\n');
        }
    }

    // Streaming adapters with an `owner_type` are emitted as instance methods on
    // the owner's opaque-handle class (see `types.rs::gen_streaming_method`), which
    // is the only context that has the `handle` field and streaming helpers in
    // scope. The FFI class is a static-only surface, so it emits nothing here.

    // Add internal visitor helpers for each function whose options parameter carries
    // an options-field trait bridge.
    if has_visitor_bridge {
        for func in &api.functions {
            if let Some(visitor_bridge) = visitor_bridge_for_function(func, config) {
                body.push_str(&gen_convert_with_visitor_internal_method(
                    func,
                    class_name,
                    prefix,
                    &opaque_types,
                    bridge_param_names,
                    bridge_type_aliases,
                    &visitor_bridge,
                ));
                body.push('\n');
            }
        }
    }

    // Add helper methods only if they are referenced in the body
    gen_helper_methods(&mut body, prefix, class_name);

    let footer_out = crate::backends::java::template_env::render("ffi_main_class_footer.jinja", minijinja::context! {});
    body.push_str(&footer_out);

    // Now assemble the file with only the imports that are actually used in the body.
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::backends::java::template_env::render(
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
            needs_jackson_json_node => body.contains("JsonNode"),
            needs_nullable => body.contains("@Nullable"),
        },
    );

    out.push_str(&body);

    out
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
    clear_fn_handles: &AHashMap<String, String>,
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
    );
}

#[allow(clippy::too_many_arguments)]
fn gen_sync_function_method_with_visitor(
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
            let annotated = render_nullable_type(&ptype, p.optional);
            format!("final {annotated} {}", to_java_name(&p.name))
        })
        .collect();

    let return_type = java_return_type(&func.return_type);
    let exception_class_name = format!("{}Exception", class_name);
    // Free-function rustdoc renders above the static-method signature so the
    // raw-FFI class doubles as a documented public surface.
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

    // Call FFI.
    //
    // Most free functions map 1:1 onto an FFI export named `{prefix}_{func.name}`,
    // so the handle constant is `{PREFIX}_{FUNC_NAME}`. Trait-bridge `clear_fn`
    // functions are the exception: the core Rust function is plural
    // (`clear_text_backends`) but the FFI export — and therefore the `NativeLib`
    // handle constant — is the singular trait-derived `{PREFIX}_CLEAR_{TRAIT_SNAKE}`.
    // Use the pre-computed mapping so this body agrees with `NativeLib.java`.
    let ffi_handle = match clear_fn_handles.get(&func.name) {
        Some(handle) => format!("NativeLib.{}", handle),
        None => format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase()),
    };

    // Build call args: bridge params get MemorySegment.NULL, others are marshalled normally.
    // Important: Bytes parameters expand to (pointer, length) pairs, so each FFI param
    // must have a corresponding argument.
    let call_args: Vec<String> = func
        .params
        .iter()
        .flat_map(|p| {
            if is_bridge_param_java(p, bridge_param_names, bridge_type_aliases) {
                vec!["MemorySegment.NULL".to_string()]
            } else {
                // Apply the same optional-wrapping logic used when marshalling.
                let effective_ty = if p.optional && !matches!(p.ty, TypeRef::Optional(_)) {
                    TypeRef::Optional(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                };
                // ffi_param_args returns one or more args (Bytes expands to 2)
                ffi_param_args(&to_java_name(&p.name), &effective_ty, opaque_types)
            }
        })
        .collect();

    // Emit a helper closure to free FFI-allocated param pointers (e.g. options created by _from_json)
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

    // Unwrap Optional<T> to determine the actual dispatch type and whether we're optional.
    let (is_optional_return, dispatch_return_type) = match &func.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    // Trait-bridge clear_fn functions are special: they return Result<()> in Rust
    // (so return_type == Unit in the IR) but are exported as
    // `sample_core_clear_X(char **out_error) -> i32` in the FFI.
    // We detect them via the clear_fn_handles map and treat them as i32 returns
    // with error handling (allocate out-error, invoke, check result code).
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
            // Void returns can't surface failure through the return value;
            // the FFI sets last_error and the Java caller must check it.
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
        // Trait-bridge clear_fn: allocate out-error, invoke with error pointer, check result.
        // The FFI function signature is: int sample_core_clear_X(char **out_error)
        // Pattern: allocate MemorySegment for address, pass to FFI, check return code.
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
        // Named return types: FFI returns a struct pointer.
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
            // Opaque handles: wrap the raw pointer directly, caller owns and will close()
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
            // Record types: use _to_json to serialize the full struct to JSON, then deserialize.
            // NOTE: _content only returns the markdown string field, not a full JSON object.
            let type_snake = return_type_name.to_snake_case();
            let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
            let to_json_handle = format!(
                "NativeLib.{}_{}_TO_JSON",
                prefix.to_uppercase(),
                type_snake.to_uppercase()
            );
            // CPD-OFF — the FFI tail (null-check resultPtr → result_to_json →
            // free result → null-check jsonPtr → reinterpret → free string →
            // MAPPER.readValue) is intentionally repeated verbatim in
            // the visitor helper below. PMD CPD flags it as a 15-line
            // duplication; extracting it into a helper would require threading
            // the Arena, MAPPER, and free-handle through a private method for
            // marginal benefit. Wrap both copies with CPD-OFF/ON markers
            // instead, matching the existing precedent for the Builder class.
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
        // Vec return types: FFI returns a JSON string pointer; deserialize into List<T>.
        // The body is delegated to a single `readJsonList` helper emitted by
        // `gen_helper_methods` so the JSON-deserialize boilerplate isn't duplicated
        // at every call site (which CPD flagged as copy-paste duplication).
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
        // Primitive return types (including boxed types for Optional)
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
            // Fallible primitive returns use a sentinel value (0) on error and
            // set last_error on the FFI side; the Java caller must check it
            // explicitly because the primitive itself can't distinguish a
            // legitimate 0 from an error.
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
    let param_names: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect();

    out.push_str(&crate::backends::java::template_env::render(
        "ffi_async_method_signature.jinja",
        minijinja::context! {
            return_type => &return_type,
            async_method_name => &async_method_name,
            params => params.join(", "),
        },
    ));
    out.push_str("        return CompletableFuture.supplyAsync(() -> {\n");
    out.push_str("            try {\n");
    if matches!(func.return_type, TypeRef::Unit) {
        out.push_str("                ");
        out.push_str(&sync_method_name);
        out.push('(');
        out.push_str(&param_names.join(", "));
        out.push_str(");\n");
        out.push_str("                return null;\n");
    } else {
        out.push_str("                return ");
        out.push_str(&sync_method_name);
        out.push('(');
        out.push_str(&param_names.join(", "));
        out.push_str(");\n");
    }
    out.push_str("            } catch (Throwable e) {\n");
    out.push_str("                throw new CompletionException(e);\n");
    out.push_str("            }\n");
    out.push_str("        });\n");
    out.push_str("    }\n");
}

#[derive(Debug, Clone)]
struct VisitorFunctionBridge {
    options_param_java: String,
    options_param_c: String,
    options_type_handle: String,
    options_field_java: String,
    options_field_native: String,
    internal_method_name: String,
}

fn visitor_bridge_for_function(func: &FunctionDef, config: &ResolvedCrateConfig) -> Option<VisitorFunctionBridge> {
    config
        .trait_bridges
        .iter()
        .find_map(|bridge| visitor_bridge_for_trait_bridge(func, bridge))
}

fn visitor_bridge_for_trait_bridge(func: &FunctionDef, bridge: &TraitBridgeConfig) -> Option<VisitorFunctionBridge> {
    if bridge.bind_via != BridgeBinding::OptionsField {
        return None;
    }

    let options_type = bridge.options_type.as_deref()?;
    let options_field = bridge.resolved_options_field()?;
    let options_param = func
        .params
        .iter()
        .find(|param| param_type_name(param) == Some(options_type))?;
    let options_param_java = to_java_name(&options_param.name);

    Some(VisitorFunctionBridge {
        options_param_c: format!("c{options_param_java}"),
        options_type_handle: options_type.to_snake_case().to_uppercase(),
        options_param_java,
        options_field_java: safe_java_field_name(options_field),
        options_field_native: options_field.to_snake_case(),
        internal_method_name: format!("{}WithVisitorInternal", to_java_name(&func.name)),
    })
}

fn param_type_name(param: &ParamDef) -> Option<&str> {
    match &param.ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn public_arg_names(
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> Vec<String> {
    func.params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn gen_convert_with_visitor_internal_method(
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
    // CPD-OFF — see the comment on the matching block emitted by the
    // non-visitor convert() above. Duplicating this FFI tail is intentional;
    // PMD CPD must not flag the pair as a real finding.
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

fn return_type_name(return_type: &TypeRef) -> Option<&str> {
    match return_type {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
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
            binding_excluded: false,
            binding_exclusion_reason: None,
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            error_type: Some("SampleCrateError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
            &AHashMap::new(),
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
    fn test_options_field_visitor_setter_uses_configured_renderer_field() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "parse".to_string(),
                rust_path: "syntax::parse".to_string(),
                params: vec![
                    ParamDef {
                        name: "source".to_string(),
                        ty: TypeRef::String,
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "options".to_string(),
                        ty: TypeRef::Named("ParseOptions".to_string()),
                        ..ParamDef::default()
                    },
                ],
                return_type: TypeRef::Named("WalkOutcome".to_string()),
                error_type: Some("ParseError".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "SyntaxWalker".to_string(),
                type_alias: Some("SyntaxWalkerHandle".to_string()),
                param_name: Some("renderer".to_string()),
                bind_via: BridgeBinding::OptionsField,
                options_type: Some("ParseOptions".to_string()),
                options_field: Some("renderer".to_string()),
                context_type: Some("SyntaxContext".to_string()),
                result_type: Some("WalkOutcome".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let out = gen_main_class(
            &api,
            &config,
            "dev.syntax",
            "Syntax",
            "syn",
            &HashSet::new(),
            &HashSet::new(),
            true,
        );

        assert!(
            out.contains("NativeLib.SYN_OPTIONS_SET_RENDERER.invoke("),
            "Java options-field bridge must invoke the renderer-derived setter"
        );
        assert!(
            !out.contains("SYN_OPTIONS_SET_VISITOR_HANDLE") && !out.contains("options_set_visitor_handle"),
            "Java options-field bridge must not bind the legacy visitor_handle setter"
        );
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
            error_type: Some("SampleCrateError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            &AHashMap::new(),
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
            &AHashMap::new(),
        );

        // The previously-duplicated JSON-deserialize line must NOT appear at
        // the call site any more (it now lives only in the helper, which is
        // emitted by gen_helper_methods at the bottom of the class).
        assert!(!out.contains(
            "createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<"
        ));
    }

    #[test]
    fn clear_fn_body_references_singular_native_lib_handle() {
        // Regression: a trait-bridge `clear_fn` is the plural core Rust function
        // name, but the FFI export and the `NativeLib`
        // handle constant are the singular trait-derived form
        // (`KRZ_CLEAR_OCR_BACKEND`). The facade body must reference that exact
        // constant and invoke it with an out-error parameter and check the result
        // code, just like other fallible FFI functions.
        let func = create_test_function("clear_ocr_backends", TypeRef::Unit);

        let mut clear_fn_handles = AHashMap::new();
        clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "krz",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
            &clear_fn_handles,
        );

        // Must use the singular trait-derived handle constant
        assert!(
            out.contains("NativeLib.KRZ_CLEAR_OCR_BACKEND.invoke"),
            "clear_fn body must reference the singular trait-derived handle, got:\n{out}"
        );
        assert!(
            !out.contains("KRZ_CLEAR_OCR_BACKENDS"),
            "clear_fn body must not reference the plural core-function-derived handle, got:\n{out}"
        );
        // Must allocate out-error buffer
        assert!(
            out.contains("var outErr = arena.allocate(ValueLayout.ADDRESS)"),
            "clear_fn body must allocate outErr, got:\n{out}"
        );
        // Must pass outErr to the FFI invocation
        assert!(
            out.contains("outErr)"),
            "clear_fn body must pass outErr to FFI invocation, got:\n{out}"
        );
        // Must check the return code for error
        assert!(
            out.contains("if (primitiveResult != 0)"),
            "clear_fn body must check primitiveResult != 0, got:\n{out}"
        );
    }

    #[test]
    fn non_clear_fn_body_derives_handle_from_function_name() {
        // Functions not registered as trait-bridge `clear_fn`s keep deriving the
        // handle constant from `func.name` (1:1 with their FFI export).
        let func = create_test_function("list_ocr_backends", TypeRef::Vec(Box::new(TypeRef::String)));

        let mut clear_fn_handles = AHashMap::new();
        clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "krz",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
            &clear_fn_handles,
        );

        assert!(
            out.contains("NativeLib.KRZ_LIST_OCR_BACKENDS"),
            "non-clear_fn body must derive the handle from func.name, got:\n{out}"
        );
    }

    #[test]
    fn clear_fn_error_throws_exception_with_code_and_message() {
        // Regression: clear_fn error path must construct SampleCrateRsException
        // with (int code, String message) constructor, not (String) constructor.
        // The error throw must be `new TestClassException(primitiveResult, msg)`
        // matching the SampleCrateRsException(int, String) constructor signature.
        let func = create_test_function("clear_ocr_backends", TypeRef::Unit);

        let mut clear_fn_handles = AHashMap::new();
        clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

        let mut out = String::new();
        let opaque_types = create_test_opaque_types();
        let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

        gen_sync_function_method(
            &mut out,
            &func,
            "krz",
            "TestClass",
            &opaque_types,
            &bridge_param_names,
            &bridge_type_aliases,
            false,
            &clear_fn_handles,
        );

        // Must throw with (int code, String msg) two-argument constructor in the error path
        assert!(
            out.contains("throw new TestClassException(primitiveResult, msg)"),
            "clear_fn error path must throw TestClassException(primitiveResult, msg), got:\n{out}"
        );
    }
}
