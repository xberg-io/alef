/// Map a raw Rust type name from adapter config to its Python equivalent.
///
/// Adapter param types are stored as raw strings in alef.toml (e.g. `"String"`).
/// Rust's `String` and `&str` both map to Python `str`; other names are passed through
/// unchanged since they are already Python-friendly type names defined in the IR.
pub(super) fn adapter_param_python_type(rust_type: &str) -> &str {
    match rust_type {
        "String" | "&str" | "&'static str" => "str",
        "bytes::Bytes" | "Vec<u8>" | "&[u8]" => "bytes",
        "()" => "None",
        other => other,
    }
}

/// Name of the `options._from_native_<snake>` converter a streaming adapter's body must apply
/// to each yielded item, or `None` when the item keeps its single native identity.
///
/// Both the `api.py` import list and the generated `yield` expression call this, so the module
/// the yield type annotation resolves to and the module the yielded value comes from cannot
/// drift apart. `options_publishable_return_types` must be the union of `options.py`'s public
/// input dataclasses AND its return-only `TypedDict`s (`orchestration.rs`'s
/// `options_publishable_return_types`) — the item's own type is never itself a function param, so
/// the narrower input-dataclass set alone would silently skip a return-only item type. ~keep
pub(super) fn streaming_item_converter(
    adapter: &crate::core::config::AdapterConfig,
    options_publishable_return_types: &std::collections::HashSet<String>,
) -> Option<String> {
    use heck::ToSnakeCase;

    if !matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
        return None;
    }
    let item_type = adapter.item_type.as_deref()?;
    if !options_publishable_return_types.contains(item_type) {
        return None;
    }
    Some(format!("_from_native_{}", item_type.to_snake_case()))
}

/// Name of the `options._from_native_<snake>` converter an `AsyncMethod` adapter's body must
/// apply to the value the engine returns, or `None` when the return type keeps its single
/// native identity.
///
/// Mirrors `streaming_item_converter` for the non-streaming adapter pattern: the wrapper's
/// `-> ReturnType` annotation names whatever `adapter.returns` says verbatim
/// (`adapter_param_python_type` only maps a handful of primitives), so when that name is a
/// public `options` type the body must convert the engine's native pyclass return value into it
/// — the engine has no idea the public type exists. `options_publishable_return_types` must be
/// the union described on `streaming_item_converter`, not the narrower input-dataclass set alone:
/// a return type is routinely `is_return_type`-only (published as a `TypedDict`, never accepted
/// as an input), and that shape used to leave this check unmatched. ~keep
pub(super) fn adapter_return_converter(
    adapter: &crate::core::config::AdapterConfig,
    options_publishable_return_types: &std::collections::HashSet<String>,
) -> Option<String> {
    use heck::ToSnakeCase;

    if !matches!(adapter.pattern, crate::core::config::AdapterPattern::AsyncMethod) {
        return None;
    }
    let return_type = adapter.returns.as_deref()?;
    if !options_publishable_return_types.contains(return_type) {
        return None;
    }
    Some(format!("_from_native_{}", return_type.to_snake_case()))
}

/// Build the pre-call `_rust_<name> = _to_rust_<snake>(<name>)` conversion statements for
/// adapter params typed as public `options` dataclasses, plus the call-site argument list to use
/// in place of the raw parameter names.
///
/// An adapter wrapper forwards its params straight to the engine method, which accepts the
/// native pyclass — not the `options` dataclass the param is annotated with. Plain function
/// wrappers already apply this exact `_to_rust_*` conversion (`emit_function_wrappers`); an
/// adapter is not exempt from it. A non-optional param falls back to the type's Rust default via
/// `config_default_on_none.jinja` when the converter still produced `None`, matching
/// `emit_function_wrappers`'s required-param handling. Params of any other type pass through
/// unchanged. ~keep
fn adapter_param_conversions(
    params: &[crate::core::config::AdapterParam],
    options_dataclass_types: &std::collections::HashSet<String>,
) -> (String, Vec<String>) {
    use heck::ToSnakeCase;

    let mut conversions = String::new();
    let mut args = Vec::with_capacity(params.len());
    for param in params {
        if !options_dataclass_types.contains(&param.ty) {
            args.push(param.name.clone());
            continue;
        }
        let snake = param.ty.to_snake_case();
        let var = format!("_rust_{}", param.name);
        let body = format!("_to_rust_{snake}({})", param.name);
        super::signature_params::emit_param_conversion(&mut conversions, &var, &param.name, &body, param.optional);
        if !param.optional {
            conversions.push_str(&crate::backends::pyo3::template_env::render(
                "config_default_on_none.jinja",
                minijinja::context! { var => &var, name => &param.ty },
            ));
        }
        args.push(var);
    }
    (conversions, args)
}

fn streaming_request_argument(
    owner_type: &str,
    request: &crate::core::ir::TypeDef,
    first_field: &crate::core::ir::FieldDef,
    options_input_types: &std::collections::HashSet<String>,
) -> (Vec<String>, Option<String>) {
    use heck::ToSnakeCase;

    let field_name = &first_field.name;
    let request_name = &request.name;
    let is_vec = matches!(&first_field.ty, crate::core::ir::TypeRef::Vec(_));
    let primitive_type = if is_vec { "list[str]" } else { "str" };
    let runtime_type = if is_vec { "list" } else { "str" };
    let has_public_dto = options_input_types.contains(request_name);
    let input_type = if has_public_dto {
        format!("{primitive_type} | {request_name} | _rust.{request_name}")
    } else {
        format!("{primitive_type} | _rust.{request_name}")
    };
    let params = vec![format!("engine: {owner_type}"), format!("{field_name}: {input_type}")];
    let construction = crate::backends::pyo3::template_env::render(
        "adapter_streaming_request_input.jinja",
        minijinja::context! {
            field_name => field_name,
            request_name => request_name,
            runtime_type => runtime_type,
            has_public_dto => has_public_dto,
            converter => format!("_to_rust_{}", request_name.to_snake_case()),
        },
    );
    (params, Some(construction))
}

/// Emit a module-level wrapper function for an adapter-based method.
///
/// Two patterns are supported:
/// - `AdapterPattern::Streaming`: the method returns an async stream; emit
///   `async def foo(engine, ...) -> AsyncIterator[Item]: async for item in engine.foo(...): yield item`
/// - `AdapterPattern::AsyncMethod`: the method is a regular async call returning a single value;
///   emit `async def foo(engine, ...) -> ReturnType: return await engine.foo(...)`
///
/// Request-based streaming wrappers retain the first-field primitive convenience argument
/// and also accept complete public/native requests without discarding the remaining fields.
///
/// Any other pattern is silently skipped (not applicable to the Python layer).
///
/// `options_input_types` names the types that `options.py` emits as public *input* dataclasses —
/// used for an adapter param's `_to_rust_*` conversion, which only ever applies to a param, so
/// the narrower input-only set is the right question to ask there.
///
/// `options_publishable_return_types` is the wider question a RETURN value (or a streaming
/// adapter's item) has to answer: does `options.py` publish this name at all, whether as an input
/// dataclass or as a return-only `TypedDict`. A streamed item or an `AsyncMethod` return whose
/// type is in that set is annotated as the `options` public type, so the body must run the
/// `_from_native_<snake>` converter — the engine yields/returns the native `_internal_bindings`
/// pyclass. Passing the narrower `options_input_types` for this question used to leave a
/// return-only type unconverted while its `-> ReturnType` annotation still resolved to the public
/// `.options` name (see `orchestration.rs`'s `options_publishable_return_types` doc). ~keep
pub(super) fn emit_adapter_wrapper(
    out: &mut String,
    adapter: &crate::core::config::AdapterConfig,
    types: &[crate::core::ir::TypeDef],
    options_input_types: &std::collections::HashSet<String>,
    options_publishable_return_types: &std::collections::HashSet<String>,
) {
    use crate::core::config::AdapterPattern;
    use heck::ToSnakeCase;

    let adapter_name = &adapter.name;
    let owner_type = adapter.owner_type.as_deref().unwrap_or("Handle");

    let (param_parts, request_construction) = if matches!(&adapter.pattern, AdapterPattern::Streaming)
        && adapter.request_type.is_some()
        && adapter.params.len() == 1
    {
        let param = &adapter.params[0];
        let short_name = &param.ty;
        let ir_type = types.iter().find(|t| &t.name == short_name);
        if let Some(ty_def) = ir_type {
            if let Some(first_field) = ty_def.fields.first() {
                streaming_request_argument(owner_type, ty_def, first_field, options_input_types)
            } else {
                let mut params = vec![format!("engine: {owner_type}")];
                for p in &adapter.params {
                    let python_type = adapter_param_python_type(&p.ty);
                    let ann = if p.optional {
                        format!("{python_type} | None = None")
                    } else {
                        python_type.to_string()
                    };
                    params.push(format!("{}: {ann}", p.name));
                }
                (params, None)
            }
        } else {
            let mut params = vec![format!("engine: {owner_type}")];
            for p in &adapter.params {
                let python_type = adapter_param_python_type(&p.ty);
                let annotation = if p.optional {
                    format!("{python_type} | None = None")
                } else {
                    python_type.to_string()
                };
                params.push(format!("{}: {}", p.name, annotation));
            }
            (params, None)
        }
    } else {
        let mut params = vec![format!("engine: {owner_type}")];
        for param in &adapter.params {
            let param_name = &param.name;
            let python_type = adapter_param_python_type(&param.ty);
            let annotation = if param.optional {
                format!("{python_type} | None = None")
            } else {
                python_type.to_string()
            };
            params.push(format!("{param_name}: {annotation}"));
        }
        (params, None)
    };

    let doc_content = {
        let snake = adapter_name.to_snake_case();
        let sentence = snake.replace('_', " ");
        let mut chars = sentence.chars();
        let capitalized = match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        };
        format!("{capitalized}.")
    };

    let (params_list, param_conversions) = if request_construction.is_some() {
        ("req".to_string(), None)
    } else {
        let (conversions, args) = adapter_param_conversions(&adapter.params, options_input_types);
        (
            args.join(", "),
            if conversions.is_empty() {
                None
            } else {
                Some(conversions)
            },
        )
    };

    match &adapter.pattern {
        AdapterPattern::Streaming => {
            let item_type = adapter.item_type.as_deref().unwrap_or("()");
            let return_type = format!("AsyncIterator[{item_type}]");
            out.push_str(&crate::backends::pyo3::template_env::render(
                "adapter_streaming_wrapper.jinja",
                minijinja::context! {
                    adapter_name => adapter_name,
                    params => param_parts.join(", "),
                    return_type => return_type,
                    doc_content => doc_content,
                    request_construction => request_construction.unwrap_or_default(),
                    param_conversions => param_conversions.unwrap_or_default(),
                    params_list => params_list,
                    item_converter =>
                        streaming_item_converter(adapter, options_publishable_return_types).unwrap_or_default(),
                },
            ));
        }
        AdapterPattern::AsyncMethod => {
            let raw_return = adapter.returns.as_deref().unwrap_or("None");
            let return_type = adapter_param_python_type(raw_return);
            out.push_str(&crate::backends::pyo3::template_env::render(
                "adapter_async_wrapper.jinja",
                minijinja::context! {
                    adapter_name => adapter_name,
                    params => param_parts.join(", "),
                    return_type => return_type,
                    doc_content => doc_content,
                    request_construction => request_construction.unwrap_or_default(),
                    param_conversions => param_conversions.unwrap_or_default(),
                    params_list => params_list,
                    return_converter =>
                        adapter_return_converter(adapter, options_publishable_return_types).unwrap_or_default(),
                },
            ));
        }
        _ => return,
    }

    out.push_str("\n\n");
}
