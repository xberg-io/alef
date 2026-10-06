use crate::codegen::naming::{go_param_name, go_type_name, to_go_name};
use crate::core::ir::TypeRef;

/// The DTO field a streaming adapter's Go wrapper decomposes into a single ergonomic scalar
/// parameter, when it does.
///
/// Mirrors the exact condition [`gen_adapter_wrapper`] renders from: exactly one configured
/// `[[adapters]] params` entry whose declared type resolves (in `types`) to an IR struct with
/// at least one field. `None` covers every shape `gen_adapter_wrapper`'s non-decomposing branch
/// handles instead -- zero or multiple configured params, a param type absent from `types`, or
/// a fieldless struct -- so Go exposes `adapter.params` unchanged in every one of those. Exposed
/// (`pub(crate)`) so e2e/snippet argument rendering can ask this question directly instead of
/// re-deriving its own copy of the same test, which is how the Java/C# streaming-request bug
/// happened in the first place. ~keep
pub(crate) fn adapter_flattened_field<'a>(
    adapter: &crate::core::config::AdapterConfig,
    types: &'a [crate::core::ir::TypeDef],
) -> Option<&'a crate::core::ir::FieldDef> {
    if adapter.request_type.is_none() || adapter.params.len() != 1 {
        return None;
    }
    let param_ty_name = &adapter.params[0].ty;
    types.iter().find(|t| &t.name == param_ty_name)?.fields.first()
}

fn adapter_wrapper_params(
    adapter: &crate::core::config::AdapterConfig,
    types: &[crate::core::ir::TypeDef],
    owner_type: &str,
    request_type_simple: &str,
) -> (Vec<String>, Option<String>) {
    if let Some(first_field) = adapter_flattened_field(adapter, types) {
        let field_name = &first_field.name;
        let field_name_go = to_go_name(field_name);

        let go_field_type = match &first_field.ty {
            TypeRef::String => "string".to_string(),
            TypeRef::Vec(inner) if matches!(**inner, TypeRef::String) => "[]string".to_string(),
            TypeRef::Vec(_) => "[]interface{}".to_string(),
            other => crate::backends::go::type_map::go_type(other).into_owned(),
        };

        let wrapper_params = vec![
            format!("engine *{owner_type}"),
            format!("{field_name_go} {go_field_type}"),
        ];

        let struct_field_name = to_go_name(field_name);
        let construction = format!("req := &{request_type_simple}{{{struct_field_name}: {field_name_go}}}\n\t");

        (wrapper_params, Some(construction))
    } else {
        let mut params = vec![format!("engine *{owner_type}")];
        for p in &adapter.params {
            let go_param_type = match p.ty.as_str() {
                "String" => "string".to_string(),
                ty => ty.rsplit("::").next().unwrap_or(ty).to_string(),
            };
            let param_name = go_param_name(&p.name);
            params.push(format!("{param_name} {go_param_type}"));
        }
        (params, None)
    }
}

fn adapter_method_call(
    adapter: &crate::core::config::AdapterConfig,
    method_call_name: &str,
    has_request: bool,
) -> String {
    if has_request {
        format!("engine.{}WithContext(ctx, *req)", method_call_name)
    } else {
        let param_args = adapter
            .params
            .iter()
            .map(|p| go_param_name(&p.name))
            .collect::<Vec<_>>()
            .join(", ");
        if param_args.is_empty() {
            format!("engine.{}WithContext(ctx)", method_call_name)
        } else {
            format!("engine.{}WithContext(ctx, {})", method_call_name, param_args)
        }
    }
}

/// Emit a module-level wrapper function for a streaming adapter.
/// This allows tests/consumers to call pkg.CrawlStream(engine, url) instead of engine.CrawlStream(url).
/// For adapters with a request_type, decompose the first field into primitive parameters for ergonomics.
pub(in crate::backends::go::gen_bindings) fn gen_adapter_wrapper(
    adapter: &crate::core::config::AdapterConfig,
    _pkg_name: &str,
    types: &[crate::core::ir::TypeDef],
) -> String {
    let adapter_name = &adapter.name;
    let go_func_name = to_go_name(adapter_name);
    let owner_type_raw = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });
    // `methods.rs` declares the receiver type (and the `<Recv><Method>Stream` iterator it
    // returns) by running the IR type name through `go_type_name`. `[[adapters]] owner_type` is a
    // plain config string with nothing upstream normalizing it, so this wrapper must apply the
    // same helper before using it as either a Go type reference or a Stream-type name component
    // -- otherwise a configured value containing an initialism (`ApiClient`, `HttpEngine`, ...)
    // names an identifier `methods.rs` never declares. See #447. ~keep
    let owner_type = go_type_name(owner_type_raw);
    let owner_type = owner_type.as_str();
    // The Stream type name comes from the owning method's own wrapper (see
    // `gen_streaming_method_wrapper`), not from `item_type` directly, but `item_type` is still
    // required config for the adapter as a whole, so the presence check stays here.
    let _item_type = adapter.item_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `item_type` in `[[adapters]]` config (the Rust item type yielded by the stream)"
        )
    });

    let request_type = adapter.request_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `request_type` in `[[adapters]]` config (the Rust request payload type)"
        )
    });
    let request_type_simple = request_type.rsplit("::").next().unwrap_or(request_type);

    let (param_parts, request_construction) = adapter_wrapper_params(adapter, types, owner_type, request_type_simple);

    let method_call_name = to_go_name(adapter_name);
    let stream_type_name = format!("{owner_type}{method_call_name}Stream");
    let context_return_type = format!("*{stream_type_name}, error");
    let item_type = go_type_name(_item_type.rsplit("::").next().unwrap_or(_item_type));
    let compatibility_return_type = format!("<-chan {item_type}, error");
    let method_call = adapter_method_call(adapter, &method_call_name, request_construction.is_some());

    let context_params = std::iter::once("ctx context.Context".to_string())
        .chain(param_parts.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ");
    let compatibility_args = std::iter::once("context.Background()".to_string())
        .chain(std::iter::once("engine".to_string()))
        .chain(adapter_flattened_field(adapter, types).map(|field| to_go_name(&field.name)))
        .chain(if adapter_flattened_field(adapter, types).is_some() {
            Vec::new()
        } else {
            adapter.params.iter().map(|p| go_param_name(&p.name)).collect()
        })
        .collect::<Vec<_>>()
        .join(", ");

    crate::backends::go::template_env::render(
        "adapter_wrapper.jinja",
        minijinja::context! {
            go_func_name => &go_func_name,
            go_func_with_context_name => format!("{go_func_name}WithContext"),
            owner_type => owner_type,
            method_call_name => &method_call_name,
            params => param_parts.join(", "),
            context_params => &context_params,
            compatibility_args => &compatibility_args,
            compatibility_return_type => &compatibility_return_type,
            context_return_type => &context_return_type,
            request_construction => request_construction.as_deref(),
            method_call => &method_call,
        },
    )
}
