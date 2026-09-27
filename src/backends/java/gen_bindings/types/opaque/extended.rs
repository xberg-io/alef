use super::*;

struct StaticFactorySymbols {
    method_name: String,
    prefix_upper: String,
    exception_class: String,
    ffi_handle: String,
    params_signature: String,
}

fn static_factory_symbols(
    method: &MethodDef,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
) -> StaticFactorySymbols {
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let method_upper = method.name.to_snake_case().to_uppercase();
    StaticFactorySymbols {
        method_name: safe_java_method_name(&method.name),
        exception_class: format!("{main_class}Exception"),
        ffi_handle: format!("NativeLib.{prefix_upper}_{owner_upper}_{method_upper}"),
        params_signature: method
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
            .join(", "),
        prefix_upper,
    }
}

fn emit_static_factory_header(
    out: &mut String,
    method: &MethodDef,
    class_name: &str,
    symbols: &StaticFactorySymbols,
) -> bool {
    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public static ");
    out.push_str(class_name);
    out.push(' ');
    out.push_str(&symbols.method_name);
    out.push('(');
    out.push_str(&symbols.params_signature);
    out.push_str(") throws ");
    out.push_str(&symbols.exception_class);
    out.push_str(" {\n");
    emit_factory_null_checks(out, method);
    emit_unsupported_factory_param(out, method, symbols)
}

fn emit_factory_null_checks(out: &mut String, method: &MethodDef) {
    for param in &method.params {
        if !param.optional && param_needs_null_check(&param.ty) {
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => param.name.to_lower_camel_case() },
            ));
        }
    }
}

fn emit_unsupported_factory_param(out: &mut String, method: &MethodDef, symbols: &StaticFactorySymbols) -> bool {
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

fn factory_needs_arena(method: &MethodDef) -> bool {
    method.params.iter().any(|param| match &param.ty {
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

fn emit_static_factory_setup(
    out: &mut String,
    method: &MethodDef,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
) {
    if factory_needs_arena(method) {
        out.push_str("        try (Arena arena = Arena.ofShared()) {\n");
    } else {
        out.push_str("        try {\n");
    }
    emit_java_resource_declarations(out, method, enum_names, opaque_type_names);
    out.push_str("            Throwable operationFailure = null;\n");
    out.push_str("            try {\n");
}

struct FactoryMarshalling<'a> {
    symbols: &'a StaticFactorySymbols,
    enum_names: &'a AHashSet<String>,
    opaque_type_names: &'a AHashSet<String>,
}

fn marshal_factory_params(
    out: &mut String,
    method: &MethodDef,
    context: &FactoryMarshalling<'_>,
) -> Option<Vec<String>> {
    let mut call_args = Vec::new();
    for param in &method.params {
        call_args.push(marshal_factory_param(out, param, context)?);
    }
    Some(call_args)
}

fn marshal_factory_param(
    out: &mut String,
    param: &crate::core::ir::ParamDef,
    context: &FactoryMarshalling<'_>,
) -> Option<String> {
    let param_name = param.name.to_lower_camel_case();
    let c_name = format!("c{}", to_class_name(&param.name));
    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            emit_factory_string_param(out, "stream_method_string_param.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Json => Some(param_name),
        TypeRef::Path => {
            emit_factory_path_param(out, &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char | TypeRef::Json) => {
            emit_factory_string_param(out, "stream_method_optional_string_param.jinja", &c_name, &param_name);
            Some(c_name)
        }
        TypeRef::Named(type_name) => marshal_factory_named_param(out, param, type_name, context),
        TypeRef::Primitive(_) | TypeRef::Duration => Some(param_name),
        _ => emit_unsupported_factory_marshalling(out, &param_name, context),
    }
}

fn emit_factory_string_param(out: &mut String, template: &str, c_name: &str, param_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! { c_name, param_name },
    ));
}

fn emit_factory_path_param(out: &mut String, c_name: &str, param_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        "marshal_path.jinja",
        minijinja::context! { cname => c_name, name => param_name },
    ));
}

fn marshal_factory_named_param(
    out: &mut String,
    param: &crate::core::ir::ParamDef,
    type_name: &str,
    context: &FactoryMarshalling<'_>,
) -> Option<String> {
    let param_name = param.name.to_lower_camel_case();
    let c_name = format!("c{}", to_class_name(&param.name));
    if context.enum_names.contains(type_name) {
        emit_factory_enum_param(out, param.optional, &param_name, &c_name);
    } else if context.opaque_type_names.contains(type_name) {
        emit_factory_opaque_param(out, param.optional, &param_name, &c_name);
        return Some(factory_opaque_arg(param.optional, &c_name));
    } else {
        emit_factory_record_param(out, param.optional, type_name, &param_name, &c_name, context);
    }
    Some(c_name)
}

fn emit_factory_enum_param(out: &mut String, optional: bool, param_name: &str, c_name: &str) {
    let enum_expr = if optional {
        format!("{param_name} != null ? {param_name}.ordinal() : -1")
    } else {
        format!("{param_name}.ordinal()")
    };
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_enum_param.jinja",
        minijinja::context! { c_name, enum_expr },
    ));
}

fn emit_factory_opaque_param(out: &mut String, optional: bool, param_name: &str, c_name: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        "opaque_param_lease_assignment.jinja",
        minijinja::context! { optional, param_name, c_name },
    ));
}

fn factory_opaque_arg(optional: bool, c_name: &str) -> String {
    if optional {
        format!("{c_name}Lease != null ? {c_name}Lease.handle() : MemorySegment.NULL")
    } else {
        format!("{c_name}Lease.handle()")
    }
}

fn emit_factory_record_param(
    out: &mut String,
    optional: bool,
    type_name: &str,
    param_name: &str,
    c_name: &str,
    context: &FactoryMarshalling<'_>,
) {
    let type_upper = type_name.to_snake_case().to_uppercase();
    let from_json = format!("NativeLib.{}_{}_FROM_JSON", context.symbols.prefix_upper, type_upper);
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
            exception_class => context.symbols.exception_class,
            method_name => context.symbols.method_name,
            assignment_only => true,
        },
    ));
}

fn emit_unsupported_factory_marshalling(
    out: &mut String,
    param_name: &str,
    context: &FactoryMarshalling<'_>,
) -> Option<String> {
    out.push_str(&crate::backends::java::template_env::render(
        "stream_method_unsupported_param.jinja",
        minijinja::context! {
            param_name,
            exception_class => context.symbols.exception_class,
            method_name => context.symbols.method_name,
        },
    ));
    None
}

struct StaticFactoryReturnParams<'a> {
    method: &'a MethodDef,
    class_name: &'a str,
    symbols: &'a StaticFactorySymbols,
    call_args: Vec<String>,
    enum_names: &'a AHashSet<String>,
    opaque_type_names: &'a AHashSet<String>,
}

fn emit_static_factory_return(out: &mut String, params: StaticFactoryReturnParams) {
    let StaticFactoryReturnParams {
        method,
        class_name,
        symbols,
        call_args,
        enum_names,
        opaque_type_names,
    } = params;
    let cleanup = render_java_resource_cleanup(
        method,
        &symbols.prefix_upper,
        enum_names,
        opaque_type_names,
        "                ",
    );
    out.push_str(&crate::backends::java::template_env::render(
        "static_factory_return_handle.jinja",
        minijinja::context! {
            ffi_handle => symbols.ffi_handle,
            args_joined => call_args.join(", "),
            cleanup,
            exception_class => symbols.exception_class,
            method_name => symbols.method_name,
            class_name,
        },
    ));
}

pub(super) struct StaticFactoryMethodParams<'a> {
    pub(super) method: &'a MethodDef,
    pub(super) class_name: &'a str,
    pub(super) prefix: &'a str,
    pub(super) owner_snake: &'a str,
    pub(super) main_class: &'a str,
    pub(super) enum_names: &'a AHashSet<String>,
    pub(super) opaque_type_names: &'a AHashSet<String>,
}

pub(super) fn gen_static_factory_method(out: &mut String, params: StaticFactoryMethodParams) {
    let StaticFactoryMethodParams {
        method,
        class_name,
        prefix,
        owner_snake,
        main_class,
        enum_names,
        opaque_type_names,
    } = params;
    let symbols = static_factory_symbols(method, prefix, owner_snake, main_class);
    if !emit_static_factory_header(out, method, class_name, &symbols) {
        return;
    }
    emit_static_factory_setup(out, method, enum_names, opaque_type_names);
    let marshalling = FactoryMarshalling {
        symbols: &symbols,
        enum_names,
        opaque_type_names,
    };
    let Some(call_args) = marshal_factory_params(out, method, &marshalling) else {
        return;
    };
    emit_static_factory_return(
        out,
        StaticFactoryReturnParams {
            method,
            class_name,
            symbols: &symbols,
            call_args,
            enum_names,
            opaque_type_names,
        },
    );
}

/// True when the given `TypeRef` is a reference type whose Java representation may
/// be null (so we should `Objects.requireNonNull` it for non-optional params).
pub(super) fn param_needs_null_check(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Named(_)
            | TypeRef::Bytes
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _)
    )
}

struct StreamingMethodSymbols {
    method_name: String,
    item_type: String,
    request_type: String,
    request_param: String,
    exception_class: String,
    start_handle: String,
    next_handle: String,
    free_handle: String,
    req_from_json: String,
    req_free: String,
    item_to_json: String,
    item_free: String,
    prefix_upper: String,
}

fn streaming_method_symbols(
    adapter: &AdapterConfig,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
) -> StreamingMethodSymbols {
    let item_type = adapter.item_type.as_deref().unwrap_or("Object");
    let request_type_full = adapter.params[0].ty.as_str();
    let request_type = request_type_full.rsplit("::").next().unwrap_or(request_type_full);
    let request_param = adapter.params[0].name.to_lower_camel_case();
    let request_param = if request_param.is_empty() {
        "request".to_owned()
    } else {
        request_param
    };
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let adapter_upper = adapter.name.to_snake_case().to_uppercase();
    let request_upper = request_type.to_snake_case().to_uppercase();
    let item_upper = item_type.to_snake_case().to_uppercase();
    StreamingMethodSymbols {
        method_name: adapter.name.to_lower_camel_case(),
        item_type: item_type.to_owned(),
        request_type: request_type.to_owned(),
        request_param,
        exception_class: format!("{main_class}Exception"),
        start_handle: format!("{prefix_upper}_{owner_upper}_{adapter_upper}_START"),
        next_handle: format!("{prefix_upper}_{owner_upper}_{adapter_upper}_NEXT"),
        free_handle: format!("{prefix_upper}_{owner_upper}_{adapter_upper}_FREE"),
        req_from_json: format!("{prefix_upper}_{request_upper}_FROM_JSON"),
        req_free: format!("{prefix_upper}_{request_upper}_FREE"),
        item_to_json: format!("{prefix_upper}_{item_upper}_TO_JSON"),
        item_free: format!("{prefix_upper}_{item_upper}_FREE"),
        prefix_upper,
    }
}

/// Emit a streaming iterator method body for an opaque-handle owner.
///
/// Generates `public Iterator<Item> <camelName>(Request request)` that calls the
/// FFI iterator-handle trio (`_start`, `_next`, `_free`), deserializing each chunk
/// pointer via `<item>_to_json` + `<item>_free` and rethrowing FFI errors as
/// `<MainClass>Exception`.
///
/// NOTE: Streaming item types must have serde derives in the Rust source.
/// This codegen always emits the `{PREFIX}_{ITEM}_TO_JSON` symbol name, which must
/// exist in the C FFI layer. If a cfg-gated type (e.g. `#[cfg(not(wasm32))]`)
/// lacks the symbol, that indicates a C FFI generation failure, not a Java codegen issue.
pub(super) fn gen_streaming_method(
    out: &mut String,
    adapter: &AdapterConfig,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    _to_json_type_names: &AHashSet<String>,
) {
    let symbols = streaming_method_symbols(adapter, prefix, owner_snake, main_class);
    out.push_str(&crate::backends::java::template_env::render(
        "streaming_iterator_method.jinja",
        minijinja::context! {
            item_type => symbols.item_type,
            method_name => symbols.method_name,
            request_type => symbols.request_type,
            request_param => symbols.request_param,
            exception_class => symbols.exception_class,
            req_from_json => symbols.req_from_json,
            start_handle => symbols.start_handle,
            req_free => symbols.req_free,
            next_handle => symbols.next_handle,
            prefix_upper => symbols.prefix_upper,
            item_to_json => symbols.item_to_json,
            item_free => symbols.item_free,
            free_handle => symbols.free_handle,
        },
    ));
}

/// Emit shared helpers (`STREAM_MAPPER`, `checkLastFfiError`, optionally `readBytesResult`)
/// used by the streaming iterator method bodies above.
pub(super) fn gen_streaming_helpers(out: &mut String, prefix: &str, main_class: &str) {
    let prefix_upper = prefix.to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let needs_read_bytes_result = out.contains("readBytesResult(");
    let free_bytes = format!("NativeLib.{prefix_upper}_FREE_BYTES");
    let needs_stream_mapper = out.contains("STREAM_MAPPER");

    out.push_str(&crate::backends::java::template_env::render(
        "streaming_helpers.jinja",
        minijinja::context! {
            exception_class => exception_class,
            prefix_upper => prefix_upper,
            needs_read_bytes_result => needs_read_bytes_result,
            free_bytes => free_bytes,
            needs_stream_mapper => needs_stream_mapper,
        },
    ));
}
