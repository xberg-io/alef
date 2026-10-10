use super::*;

pub(super) fn emit_sync_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    let return_type = &invocation.dispatch_return_type;
    if matches!(return_type, TypeRef::Unit) && !invocation.is_clear_fn {
        return emit_void_return(out, invocation);
    }
    if invocation.is_clear_fn {
        return emit_clear_return(out, invocation);
    }
    match return_type {
        ty if is_ffi_string_return(ty) => emit_string_return(out, invocation),
        TypeRef::Named(name) => emit_named_return(out, invocation, name),
        TypeRef::Vec(inner) => emit_vec_return(out, invocation, inner),
        TypeRef::Bytes if is_bytes_result(invocation.func) => emit_bytes_return(out, invocation),
        _ => emit_primitive_return(out, invocation),
    }
}

fn emit_void_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_void.jinja",
        crate::alef_context! {
            ffi_handle => &invocation.ffi_handle,
            args => invocation.call_args.join(", "),
        },
    ));
    if invocation.func.error_type.is_some() {
        out.push_str("            checkLastError();\n");
    }
    emit_catch(out, invocation.class_name);
}

fn emit_clear_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    let mut call_args = invocation.call_args.clone();
    call_args.push("outErr".to_string());
    out.push_str("            var outErr = arena.allocate(ValueLayout.ADDRESS);\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_primitive_result.jinja",
        crate::alef_context! {
            cast_type => "int",
            ffi_handle => &invocation.ffi_handle,
            call_args => call_args.join(", "),
        },
    ));
    out.push_str("            if (primitiveResult != 0) {\n");
    out.push_str("                MemorySegment errPtr = outErr.get(ValueLayout.ADDRESS, 0);\n");
    out.push_str("                String msg = errPtr.equals(MemorySegment.NULL) ? \"clear failed (rc=\" + primitiveResult + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
    out.push_str("                throw new ");
    out.push_str(&format!("{}Exception", invocation.class_name));
    out.push_str("(primitiveResult, msg);\n            }\n");
    emit_catch(out, invocation.class_name);
}

fn emit_string_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    emit_result_pointer_call(out, invocation);
    emit_null_check(out, invocation.is_optional_return);
    let len_handle = format!(
        "NativeLib.{}_{}_LEN",
        invocation.prefix.to_uppercase(),
        invocation.func.name.to_uppercase()
    );
    out.push_str("            nativeResources.register(resultPtr, handle -> NativeLib.");
    out.push_str(&invocation.prefix.to_uppercase());
    out.push_str("_FREE_STRING.invoke(handle));\n");
    out.push_str("            long resultLen = (long) ");
    out.push_str(&len_handle);
    out.push_str(".invoke(");
    out.push_str(&invocation.call_args.join(", "));
    out.push_str(");\n");
    out.push_str("            String str = readCString(resultPtr, resultLen);\n");
    let return_expr = if matches!(invocation.dispatch_return_type, TypeRef::Path) {
        "java.nio.file.Path.of(str)"
    } else {
        "str"
    };
    emit_optional_expression_return(out, return_expr, invocation.is_optional_return);
    emit_catch(out, invocation.class_name);
}

fn emit_named_return(out: &mut String, invocation: &SyncInvocation<'_>, return_type_name: &str) {
    emit_result_pointer_call(out, invocation);
    emit_null_check(out, invocation.is_optional_return);
    if invocation.opaque_types.contains(return_type_name) {
        emit_opaque_return(out, return_type_name, invocation.is_optional_return);
    } else {
        emit_serializable_return(out, invocation, return_type_name);
    }
    emit_catch(out, invocation.class_name);
}

fn emit_opaque_return(out: &mut String, class_name: &str, optional: bool) {
    let template = if optional {
        "ffi_return_new_handle.jinja"
    } else {
        "ffi_return_new_instance.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        crate::alef_context! { class_name },
    ));
}

fn emit_serializable_return(out: &mut String, invocation: &SyncInvocation<'_>, return_type_name: &str) {
    let type_snake = return_type_name.to_snake_case();
    let free_handle = format!(
        "NativeLib.{}_{}_FREE",
        invocation.prefix.to_uppercase(),
        type_snake.to_uppercase()
    );
    let to_json_handle = format!(
        "NativeLib.{}_{}_TO_JSON",
        invocation.prefix.to_uppercase(),
        type_snake.to_uppercase()
    );
    out.push_str("            // CPD-OFF\n            nativeResources.register(resultPtr, handle -> ");
    out.push_str(&free_handle);
    out.push_str(".invoke(handle));\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_json_ptr.jinja",
        crate::alef_context! { to_json_handle },
    ));
    emit_json_null_handling(out, invocation);
    out.push_str("            nativeResources.register(jsonPtr, handle -> NativeLib.");
    out.push_str(&invocation.prefix.to_uppercase());
    out.push_str("_FREE_STRING.invoke(handle));\n");
    out.push_str("            String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);\n");
    emit_json_mapper_return(out, return_type_name, invocation.is_optional_return);
    out.push_str("            // CPD-ON\n");
}

fn emit_json_null_handling(out: &mut String, invocation: &SyncInvocation<'_>) {
    out.push_str("            if (jsonPtr.equals(MemorySegment.NULL)) {\n");
    out.push_str("                checkLastError();\n");
    if invocation.is_optional_return {
        out.push_str("                return Optional.empty();\n");
    } else {
        out.push_str("                throw new ");
        out.push_str(&format!("{}Exception", invocation.class_name));
        out.push_str("(\"");
        out.push_str(&to_java_name(&invocation.func.name));
        out.push_str(": failed to serialize response\", null);\n");
    }
    out.push_str("            }\n");
}

fn emit_json_mapper_return(out: &mut String, class_name: &str, optional: bool) {
    let template = if optional {
        "ffi_return_mapper_read_optional.jinja"
    } else {
        "ffi_return_mapper_read.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        crate::alef_context! { class_name },
    ));
}

fn emit_vec_return(out: &mut String, invocation: &SyncInvocation<'_>, inner: &TypeRef) {
    emit_result_pointer_call(out, invocation);
    let type_ref = format!(
        "new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }}",
        java_boxed_type(inner)
    );
    let template = if invocation.is_optional_return {
        "ffi_return_read_json_list_optional.jinja"
    } else {
        "ffi_return_read_json_list_plain.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        crate::alef_context! { type_ref },
    ));
    emit_catch(out, invocation.class_name);
}

fn emit_bytes_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    let args = if invocation.call_args.is_empty() {
        String::new()
    } else {
        format!("{}, ", invocation.call_args.join(", "))
    };
    let free_bytes_handle = format!("NativeLib.{}_FREE_BYTES", invocation.prefix.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "bytes_result_call.jinja",
        crate::alef_context! {
            ffi_handle => &invocation.ffi_handle,
            args,
            free_bytes_handle,
            optional => invocation.is_optional_return,
        },
    ));
    emit_catch(out, invocation.class_name);
}

fn emit_primitive_return(out: &mut String, invocation: &SyncInvocation<'_>) {
    let call_args = invocation.call_args.join(", ");
    let presence = crate::backends::java::gen_bindings::result_presence::presence_capture(
        &invocation.func.return_type,
        None,
        &invocation.ffi_handle,
        &call_args,
    );
    if let Some(capture) = &presence {
        out.push_str(capture);
    }
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_primitive_result.jinja",
        crate::alef_context! {
            cast_type => java_ffi_return_cast(&invocation.dispatch_return_type),
            ffi_handle => &invocation.ffi_handle,
            call_args => &call_args,
        },
    ));
    if invocation.func.error_type.is_some() {
        out.push_str("            checkLastError();\n");
    }
    let return_expr = java_ffi_return_expr(&invocation.dispatch_return_type, "primitiveResult");
    let return_expr = if invocation.is_optional_return {
        let present = format!("Optional.of({return_expr})");
        match presence {
            Some(_) => {
                crate::backends::java::gen_bindings::result_presence::presence_conditional(&present, "Optional.empty()")
            }
            None => present,
        }
    } else {
        return_expr
    };
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_return_primitive_result.jinja",
        crate::alef_context! { return_expr },
    ));
    emit_catch(out, invocation.class_name);
}

/// Emits the write-back return for a `&mut T` DTO parameter on a unit-returning function
/// (issue #380): the FFI mutator is invoked for effect, then the already-marshaled parameter
/// handle -- registered for free during parameter marshalling, so it must NOT be registered
/// again here -- is read back out via `_to_json` and decoded into a fresh `T`, which becomes
/// the method's return value. Without this, the temporary handle the host built from the
/// caller's JSON is mutated and then freed unread, so the caller's value is silently untouched.
pub(super) fn emit_writeback_return(
    out: &mut String,
    invocation: &SyncInvocation<'_>,
    handle_var: &str,
    return_type_name: &str,
) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_invoke_void.jinja",
        crate::alef_context! {
            ffi_handle => &invocation.ffi_handle,
            args => invocation.call_args.join(", "),
        },
    ));
    if invocation.func.error_type.is_some() {
        out.push_str("            checkLastError();\n");
    }
    let type_snake = return_type_name.to_snake_case();
    let to_json_handle = format!(
        "NativeLib.{}_{}_TO_JSON",
        invocation.prefix.to_uppercase(),
        type_snake.to_uppercase()
    );
    let free_string_handle = format!("NativeLib.{}_FREE_STRING", invocation.prefix.to_uppercase());
    let exception_class = format!("{}Exception", invocation.class_name);
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_writeback_return.jinja",
        crate::alef_context! {
            to_json_handle,
            handle_var,
            free_string_handle,
            exception_class,
            method_name => to_java_name(&invocation.func.name),
            class_name => return_type_name,
        },
    ));
    emit_catch(out, invocation.class_name);
}

fn emit_result_pointer_call(out: &mut String, invocation: &SyncInvocation<'_>) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_ptr_call.jinja",
        crate::alef_context! {
            ffi_handle => &invocation.ffi_handle,
            args => invocation.call_args.join(", "),
        },
    ));
}

pub(super) fn emit_null_check(out: &mut String, optional: bool) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_null_check.jinja",
        crate::alef_context! {
            var => "resultPtr",
            optional,
        },
    ));
}

fn emit_optional_expression_return(out: &mut String, expression: &str, optional: bool) {
    let template = if optional {
        "ffi_return_optional_expr.jinja"
    } else {
        "ffi_return_expr.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        crate::alef_context! { expr => expression },
    ));
}

fn emit_catch(out: &mut String, class_name: &str) {
    super::super::error_catch::emit_method_catch_chain(out, &format!("{}Exception", class_name));
}
