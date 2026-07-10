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

/// Emit a module-level wrapper function for an adapter-based method.
///
/// Two patterns are supported:
/// - `AdapterPattern::Streaming`: the method returns an async stream; emit
///   `async def foo(engine, ...) -> AsyncIterator[Item]: async for item in engine.foo(...): yield item`
/// - `AdapterPattern::AsyncMethod`: the method is a regular async call returning a single value;
///   emit `async def foo(engine, ...) -> ReturnType: return await engine.foo(...)`
///
/// For streaming adapters that take request objects, the wrapper accepts primitive args
/// (e.g., `url: str`) and constructs the request object before calling the engine method.
///
/// Any other pattern is silently skipped (not applicable to the Python layer).
pub(super) fn emit_adapter_wrapper(
    out: &mut String,
    adapter: &crate::core::config::AdapterConfig,
    types: &[crate::core::ir::TypeDef],
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
                let field_name = &first_field.name;
                let is_vec = matches!(&first_field.ty, crate::core::ir::TypeRef::Vec(_));
                let python_type = if is_vec { "list[str]" } else { "str" };
                let wrapper_params = vec![format!("engine: {owner_type}"), format!("{field_name}: {python_type}")];
                let construction = format!("    req = _rust.{short_name}({field_name}={field_name})\n");
                (wrapper_params, Some(construction))
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

    let params_list = if request_construction.is_some() {
        "req".to_string()
    } else {
        adapter
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
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
                    params_list => params_list,
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
                    params_list => params_list,
                },
            ));
        }
        _ => return,
    }

    out.push_str("\n\n");
}
