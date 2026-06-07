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

    // For streaming adapters with a request_type, decompose the request into primitives.
    // E.g., CrawlStreamRequest { url } → url: str, url
    // This allows e2e tests to pass `crawl_stream(engine, "url")` instead of
    // `crawl_stream(engine, CrawlStreamRequest(url="url"))`.
    let (param_parts, request_construction) = if matches!(&adapter.pattern, AdapterPattern::Streaming)
        && adapter.request_type.is_some()
        && adapter.params.len() == 1
    {
        // Streaming with a single request param: decompose to primitives by
        // inspecting the request type's first field in the IR.
        // E.g. a type with field `url: String` → `url: str`; `urls: Vec<String>` → `urls: list[str]`.
        let param = &adapter.params[0];
        let short_name = &param.ty; // short type name, e.g. the param's declared type
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
                // Type has no fields; fall back to original behavior
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
            // Type not found in IR; fall back to original behavior
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
        // Non-streaming or multi-param: use original behavior
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

    // Build the docstring from the adapter name.
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

    // Build the positional param list for the method call (no `self` — that's `engine`).
    let params_list = if request_construction.is_some() {
        // If we constructed a request object, use it
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
            // Streaming: the engine method returns an async iterator; re-yield each item.
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
            // Non-streaming: the engine method is a coroutine returning a single value.
            // Emit a plain async def that awaits and returns the result.
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
        // Other patterns (SyncFunction, CallbackBridge) are not applicable
        // to the Python api.py wrapper layer — skip them silently.
        _ => return,
    }

    out.push_str("\n\n");
}
