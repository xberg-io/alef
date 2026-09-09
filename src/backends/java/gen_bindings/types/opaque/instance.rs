use super::*;

struct InstanceMethodSymbols {
    method_name: String,
    prefix_upper: String,
    exception_class: String,
    ffi_handle: String,
    params_signature: String,
    return_type_java: String,
    dispatch_return: TypeRef,
    is_bytes_result: bool,
    is_optional_return: bool,
    owned_receiver: bool,
    /// The companion `MethodHandle` that answers whether this method's `Option` return was
    /// `Some`, or `None` when the FFI crate exports no companion for this return type and
    /// receiver. Resolved once, here, from the real `MethodDef` — the emitters downstream keep
    /// only `dispatch_return` (the *unwrapped* inner type) and `owned_receiver`, so asking the
    /// authority from there would mean rebuilding the very facts it judges. ~keep
    presence_handle: Option<String>,
}

fn instance_method_symbols(
    method: &MethodDef,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
) -> InstanceMethodSymbols {
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let method_upper = method.name.to_snake_case().to_uppercase();
    let is_bytes_result = method.return_type.returns_bytes_out_params();
    let (is_optional_return, dispatch_return) = match &method.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };
    let return_type_java = instance_return_type(method, is_bytes_result, is_optional_return);
    let ffi_handle = format!("NativeLib.{prefix_upper}_{owner_upper}_{method_upper}");
    let presence_handle =
        crate::backends::ffi::type_map::result_presence_companion_exists(&method.return_type, method.receiver.as_ref())
            .then(|| crate::backends::java::gen_bindings::result_presence::presence_handle_name(&ffi_handle));
    InstanceMethodSymbols {
        method_name: safe_java_method_name(&method.name),
        exception_class: format!("{main_class}Exception"),
        ffi_handle,
        params_signature: method_params_signature(method),
        return_type_java,
        dispatch_return,
        is_bytes_result,
        is_optional_return,
        owned_receiver: method.receiver == Some(ReceiverKind::Owned),
        presence_handle,
        prefix_upper,
    }
}

fn method_params_signature(method: &MethodDef) -> String {
    method
        .params
        .iter()
        .map(|param| {
            let param_type = if param.optional {
                java_boxed_type(&param.ty).to_string()
            } else {
                java_type(&param.ty).to_string()
            };
            format!("final {} {}", param_type, param.name.to_lower_camel_case())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn instance_return_type(method: &MethodDef, is_bytes_result: bool, is_optional_return: bool) -> String {
    if !is_bytes_result {
        return java_return_type(&method.return_type).to_string();
    }
    if is_optional_return {
        "java.util.Optional<byte[]>".to_owned()
    } else {
        "byte[]".to_owned()
    }
}

fn emit_instance_method_header(out: &mut String, method: &MethodDef, symbols: &InstanceMethodSymbols) -> bool {
    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public ");
    out.push_str(&symbols.return_type_java);
    out.push(' ');
    out.push_str(&symbols.method_name);
    out.push('(');
    out.push_str(&symbols.params_signature);
    out.push(')');
    if method.name != "clone" {
        out.push_str(" throws ");
        out.push_str(&symbols.exception_class);
    }
    out.push_str(" {\n");
    emit_instance_null_checks(out, method);
    emit_unsupported_instance_param(out, method, symbols)
}

fn emit_instance_null_checks(out: &mut String, method: &MethodDef) {
    for param in &method.params {
        if !param.optional && param_needs_null_check(&param.ty) {
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => param.name.to_lower_camel_case() },
            ));
        }
    }
}

fn emit_unsupported_instance_param(out: &mut String, method: &MethodDef, symbols: &InstanceMethodSymbols) -> bool {
    let Some(param) = method
        .params
        .iter()
        .find(|param| !java_opaque_method_param_supported(&param.ty))
    else {
        return true;
    };
    out.push_str(&crate::backends::java::template_env::render(
        "opaque_unsupported_param.jinja",
        minijinja::context! {
            exception_class => symbols.exception_class,
            method_name => symbols.method_name,
            param_name => param.name.to_lower_camel_case(),
        },
    ));
    out.push_str("    }\n\n");
    false
}

fn instance_method_needs_arena(method: &MethodDef, is_bytes_result: bool) -> bool {
    is_bytes_result
        || method.params.iter().any(|param| match &param.ty {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_) => true,
            TypeRef::Optional(inner)
                if matches!(
                    inner.as_ref(),
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_)
                ) =>
            {
                true
            }
            _ => false,
        })
}

fn emit_instance_method_setup(
    out: &mut String,
    method: &MethodDef,
    symbols: &InstanceMethodSymbols,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
) {
    if instance_method_needs_arena(method, symbols.is_bytes_result) {
        out.push_str("        try (Arena arena = Arena.ofShared()) {\n");
    } else {
        out.push_str("        try {\n");
    }
    emit_java_resource_declarations(out, method, enum_names, opaque_type_names);
    if symbols.owned_receiver {
        out.push_str("            HandleTransfer handleTransfer = null;\n");
    }
    out.push_str("            Throwable operationFailure = null;\n");
    out.push_str("            try {\n");
    if symbols.owned_receiver {
        out.push_str("            handleTransfer = takeHandle();\n");
    }
}

struct ParamMarshalling<'a> {
    prefix_upper: &'a str,
    exception_class: &'a str,
    method_name: &'a str,
    opaque_type_names: &'a AHashSet<String>,
}

fn marshal_instance_params(
    out: &mut String,
    method: &MethodDef,
    context: &ParamMarshalling<'_>,
) -> Option<Vec<String>> {
    let mut call_args = Vec::new();
    for param in &method.params {
        call_args.push(marshal_instance_param(out, param, context)?);
    }
    Some(call_args)
}

fn marshal_instance_param(
    out: &mut String,
    param: &crate::core::ir::ParamDef,
    context: &ParamMarshalling<'_>,
) -> Option<String> {
    let param_name = param.name.to_lower_camel_case();
    let c_name = format!("c{}", to_class_name(&param.name));
    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            emit_string_param(out, "stream_method_string_param.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Json => Some(param_name),
        TypeRef::Path => {
            emit_path_param(out, "marshal_path.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
            emit_string_param(out, "stream_method_optional_string_param.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json) => Some(param_name),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
            emit_path_param(out, "marshal_optional_path.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Named(type_name) => marshal_named_instance_param(out, param, type_name, context),
        TypeRef::Optional(inner) => marshal_optional_instance_param(out, inner, &param_name, &c_name, context),
        TypeRef::Primitive(_) | TypeRef::Duration => Some(param_name),
        _ => emit_unsupported_param(out, &param_name, context),
    }
}

fn emit_string_param(out: &mut String, template: &str, c_name: &str, param_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! { c_name, param_name },
    ));
}

fn emit_path_param(out: &mut String, template: &str, c_name: &str, param_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! { cname => c_name, name => param_name },
    ));
}

fn marshal_named_instance_param(
    out: &mut String,
    param: &crate::core::ir::ParamDef,
    type_name: &str,
    context: &ParamMarshalling<'_>,
) -> Option<String> {
    let param_name = param.name.to_lower_camel_case();
    let c_name = format!("c{}", to_class_name(&param.name));
    if context.opaque_type_names.contains(type_name) {
        emit_opaque_param_lease(out, param.optional, &param_name, &c_name);
        return Some(opaque_lease_arg(param.optional, &c_name));
    }
    emit_named_json_param(out, param.optional, type_name, &param_name, &c_name, context);
    Some(c_name)
}

fn marshal_optional_instance_param(
    out: &mut String,
    inner: &TypeRef,
    param_name: &str,
    c_name: &str,
    context: &ParamMarshalling<'_>,
) -> Option<String> {
    let TypeRef::Named(type_name) = inner else {
        return emit_unsupported_param(out, param_name, context);
    };
    if context.opaque_type_names.contains(type_name) {
        emit_opaque_param_lease(out, true, param_name, c_name);
        return Some(opaque_lease_arg(true, c_name));
    }
    emit_named_json_param(out, true, type_name, param_name, c_name, context);
    Some(c_name.to_owned())
}

fn emit_opaque_param_lease(out: &mut String, optional: bool, param_name: &str, c_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        "opaque_param_lease_assignment.jinja",
        minijinja::context! { optional, param_name, c_name },
    ));
}

fn opaque_lease_arg(optional: bool, c_name: &str) -> String {
    if optional {
        format!("{c_name}Lease != null ? {c_name}Lease.handle() : MemorySegment.NULL")
    } else {
        format!("{c_name}Lease.handle()")
    }
}

fn emit_named_json_param(
    out: &mut String,
    optional: bool,
    type_name: &str,
    param_name: &str,
    c_name: &str,
    context: &ParamMarshalling<'_>,
) {
    let type_upper = type_name.to_snake_case().to_uppercase();
    let from_json = format!("NativeLib.{}_{}_FROM_JSON", context.prefix_upper, type_upper);
    let template = if optional {
        "stream_method_optional_named_param.jinja"
    } else {
        "stream_method_named_param.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! {
            c_name,
            param_name,
            from_json,
            exception_class => context.exception_class,
            method_name => context.method_name,
            assignment_only => true,
        },
    ));
}

fn emit_unsupported_param(out: &mut String, param_name: &str, context: &ParamMarshalling<'_>) -> Option<String> {
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_unsupported_param.jinja",
        minijinja::context! {
            param_name,
            exception_class => context.exception_class,
            method_name => context.method_name,
        },
    ));
    None
}

fn receiver_call_args(out: &mut String, owned_receiver: bool, call_args: Vec<String>) -> String {
    let receiver_arg = if owned_receiver {
        "handleTransfer.handle()"
    } else {
        out.push_str("            try (HandleLease handleLease = borrowHandle()) {\n");
        "handleLease.handle()"
    };
    let mut all_args = vec![receiver_arg.to_owned()];
    all_args.extend(call_args);
    all_args.join(", ")
}

fn receiver_commit(symbols: &InstanceMethodSymbols) -> &'static str {
    if symbols.owned_receiver {
        "            handleTransfer.commit();\n"
    } else {
        ""
    }
}

struct ResultMarshalling<'a> {
    symbols: &'a InstanceMethodSymbols,
    args_joined: &'a str,
    opaque_type_names: &'a AHashSet<String>,
    to_json_type_names: &'a AHashSet<String>,
}

fn emit_instance_result(out: &mut String, context: &ResultMarshalling<'_>) {
    let symbols = context.symbols;
    if symbols.is_bytes_result {
        emit_bytes_result(out, context);
    } else if let TypeRef::Named(return_type_name) = &symbols.dispatch_return {
        emit_named_result(out, return_type_name, context);
    } else if is_ffi_string_return(&symbols.dispatch_return) {
        emit_string_result(out, context);
    } else if matches!(symbols.dispatch_return, TypeRef::Primitive(_) | TypeRef::Duration) {
        emit_primitive_result(out, context);
    } else if matches!(symbols.dispatch_return, TypeRef::Unit) {
        emit_unit_result(out, context);
    } else {
        emit_unsupported_return(out, context);
    }
}

fn emit_bytes_result(out: &mut String, context: &ResultMarshalling<'_>) {
    let symbols = context.symbols;
    let empty_return = if symbols.is_optional_return {
        "return java.util.Optional.empty();"
    } else {
        "return null;"
    };
    let success_return = if symbols.is_optional_return {
        "java.util.Optional.of(result)"
    } else {
        "result"
    };
    let free_bytes = format!("NativeLib.{}_FREE_BYTES", symbols.prefix_upper);
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_bytes_result.jinja",
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(symbols),
            // The template branches on `optional` alone. Passing only the pre-rendered
            // `empty_return`/`success_return` left it undefined, which minijinja reads as
            // falsy, so an Option<Vec<u8>> method returned a bare byte[] from a method
            // declared Optional<byte[]> and would not compile. ~keep
            optional => symbols.is_optional_return,
            empty_return,
            free_bytes,
            success_return,
        },
    ));
}

fn emit_named_result(out: &mut String, return_type_name: &str, context: &ResultMarshalling<'_>) {
    if context.opaque_type_names.contains(return_type_name) {
        emit_opaque_result(out, return_type_name, context);
    } else if context.to_json_type_names.contains(return_type_name) {
        emit_json_named_result(out, return_type_name, context);
    } else {
        emit_unsupported_return(out, context);
    }
}

fn emit_opaque_result(out: &mut String, return_type_name: &str, context: &ResultMarshalling<'_>) {
    let symbols = context.symbols;
    let empty_return = if symbols.is_optional_return {
        "java.util.Optional.empty()".to_owned()
    } else {
        "null".to_owned()
    };
    let success_return = if symbols.is_optional_return {
        format!("java.util.Optional.of(new {return_type_name}(resultPtr))")
    } else {
        format!("new {return_type_name}(resultPtr)")
    };
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_opaque_handle_result.jinja",
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(symbols),
            empty_return,
            success_return,
        },
    ));
}

fn emit_json_named_result(out: &mut String, return_type_name: &str, context: &ResultMarshalling<'_>) {
    let symbols = context.symbols;
    let return_upper = return_type_name.to_snake_case().to_uppercase();
    let ret_free = format!("NativeLib.{}_{}_FREE", symbols.prefix_upper, return_upper);
    let ret_to_json = format!("NativeLib.{}_{}_TO_JSON", symbols.prefix_upper, return_upper);
    let (empty_return, success_return) = named_return_expressions(return_type_name, symbols.is_optional_return);
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_named_result.jinja",
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(symbols),
            to_json => ret_to_json,
            exception_class => symbols.exception_class,
            method_name => symbols.method_name,
            prefix_upper => symbols.prefix_upper,
            return_type_name,
            ret_free,
            empty_return,
            success_return,
        },
    ));
}

fn named_return_expressions(return_type_name: &str, optional: bool) -> (String, String) {
    if optional {
        return (
            "java.util.Optional.empty()".to_owned(),
            format!("return java.util.Optional.of(STREAM_MAPPER.readValue(json, {return_type_name}.class));"),
        );
    }
    (
        "null".to_owned(),
        format!("return STREAM_MAPPER.readValue(json, {return_type_name}.class);"),
    )
}

fn emit_string_result(out: &mut String, context: &ResultMarshalling<'_>) {
    let symbols = context.symbols;
    let template = if symbols.is_optional_return {
        "stream_method_optional_string_result.jinja"
    } else {
        "stream_method_string_result.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(symbols),
            prefix_upper => symbols.prefix_upper,
        },
    ));
}

fn emit_primitive_result(out: &mut String, context: &ResultMarshalling<'_>) {
    use crate::backends::java::gen_bindings::result_presence;

    let symbols = context.symbols;
    let template = if symbols.is_optional_return {
        "stream_method_optional_primitive_result.jinja"
    } else {
        "stream_method_primitive_result.jinja"
    };
    let is_optional_long = matches!(
        symbols.dispatch_return,
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Isize | PrimitiveType::Usize)
            | TypeRef::Duration
    );
    let java_primitive_expr = java_ffi_return_expr(&symbols.dispatch_return, "result");
    let (present_expr, empty_expr) = if is_optional_long {
        (
            "java.util.OptionalLong.of(result)".to_string(),
            "java.util.OptionalLong.empty()",
        )
    } else {
        (
            format!("java.util.Optional.of({java_primitive_expr})"),
            "java.util.Optional.empty()",
        )
    };
    let return_expr = match &symbols.presence_handle {
        Some(_) => result_presence::presence_conditional(&present_expr, empty_expr),
        None => present_expr,
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(symbols),
            java_primitive_type => java_ffi_return_cast(&symbols.dispatch_return),
            java_primitive_expr => java_primitive_expr,
            is_optional_long => is_optional_long,
            return_expr => return_expr,
            presence_capture => symbols
                .presence_handle
                .as_ref()
                .map(|handle| result_presence::presence_capture_line(handle, context.args_joined))
                .unwrap_or_default(),
        },
    ));
}

fn emit_unit_result(out: &mut String, context: &ResultMarshalling<'_>) {
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_unit_result.jinja",
        minijinja::context! {
            ffi_handle => context.symbols.ffi_handle,
            args_joined => context.args_joined,
            named_frees => receiver_commit(context.symbols),
        },
    ));
}

fn emit_unsupported_return(out: &mut String, context: &ResultMarshalling<'_>) {
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_unsupported_return.jinja",
        minijinja::context! {
            named_frees => "",
            method_name => context.symbols.method_name,
            exception_class => context.symbols.exception_class,
        },
    ));
}

fn emit_instance_method_cleanup(
    out: &mut String,
    method: &MethodDef,
    symbols: &InstanceMethodSymbols,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
) {
    if !symbols.owned_receiver {
        out.push_str("            }\n");
    }
    out.push_str("            } catch (Throwable failure) {\n");
    out.push_str("                operationFailure = failure;\n                throw failure;\n");
    out.push_str("            } finally {\n");
    out.push_str(&render_java_resource_cleanup(
        method,
        &symbols.prefix_upper,
        enum_names,
        opaque_type_names,
        "                ",
    ));
    if symbols.owned_receiver {
        out.push_str("                if (handleTransfer != null) { handleTransfer.close(); }\n");
    }
    out.push_str("            }\n");
    emit_instance_catch(out, method, symbols);
}

fn emit_instance_catch(out: &mut String, method: &MethodDef, symbols: &InstanceMethodSymbols) {
    let catch_template = if method.name == "clone" {
        "stream_method_catch_unchecked.jinja"
    } else {
        "stream_method_catch.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        catch_template,
        minijinja::context! {
            exception_class => symbols.exception_class,
            method_name => symbols.method_name,
        },
    ));
}

/// Emit a non-streaming instance method on an opaque-handle owner.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_instance_method(
    out: &mut String,
    method: &MethodDef,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) {
    let symbols = instance_method_symbols(method, prefix, owner_snake, main_class);
    if !emit_instance_method_header(out, method, &symbols) {
        return;
    }
    emit_instance_method_setup(out, method, &symbols, enum_names, opaque_type_names);
    let marshalling = ParamMarshalling {
        prefix_upper: &symbols.prefix_upper,
        exception_class: &symbols.exception_class,
        method_name: &symbols.method_name,
        opaque_type_names,
    };
    let Some(call_args) = marshal_instance_params(out, method, &marshalling) else {
        return;
    };
    let args_joined = receiver_call_args(out, symbols.owned_receiver, call_args);
    emit_instance_result(
        out,
        &ResultMarshalling {
            symbols: &symbols,
            args_joined: &args_joined,
            opaque_type_names,
            to_json_type_names,
        },
    );
    emit_instance_method_cleanup(out, method, &symbols, enum_names, opaque_type_names);
}
