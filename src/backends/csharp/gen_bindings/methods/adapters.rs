use super::super::StreamingMethodMeta;
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::AdapterConfig;
use crate::core::ir::{ApiSurface, MethodDef};
use heck::ToLowerCamelCase;

/// Generate a static wrapper method for a streaming method on an opaque type.
/// Delegates to the instance method on the opaque handle class.
pub(super) fn gen_opaque_streaming_static_wrapper(
    method: &MethodDef,
    opaque_type_name: &str,
    meta: &StreamingMethodMeta,
    _exception_name: &str,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    let class_name = csharp_type_name(opaque_type_name);
    let method_name = to_csharp_name(&method.name);
    let item_type = csharp_type_name(&meta.item_type);
    let method_name = if method.is_async {
        format!("{method_name}Async")
    } else {
        method_name
    };
    let params_decl = method
        .params
        .iter()
        .map(|param| {
            let param_name = param.name.to_lower_camel_case();
            let param_type = csharp_type(&param.ty);
            format!("{param_type} {param_name}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let args = method
        .params
        .iter()
        .map(|param| param.name.to_lower_camel_case())
        .collect::<Vec<_>>()
        .join(", ");
    let doc_lines: Vec<String> = method.doc.lines().map(str::to_owned).collect();
    let param_docs: Vec<String> = method
        .params
        .iter()
        .map(|param| param.name.to_lower_camel_case())
        .collect();

    out.push_str(&render(
        "opaque_streaming_static_wrapper.jinja",
        minijinja::context! {
            doc_lines,
            opaque_type_name,
            param_docs,
            is_async => method.is_async,
            item_type,
            method_name,
            class_name,
            params_decl,
            args,
        },
    ));

    out
}

/// Generate a wrapper method for a streaming adapter.
/// Emits a public async method that returns IAsyncEnumerable<ItemType>.
pub(super) fn gen_adapter_wrapper(
    adapter: &AdapterConfig,
    _prefix: &str,
    _exception_name: &str,
    _api: &ApiSurface,
) -> String {
    use crate::backends::csharp::template_env::render;

    let adapter_name = &adapter.name;
    let method_name = to_csharp_name(adapter_name);
    let Some(owner_type) = adapter.owner_type.as_deref() else {
        return String::new();
    };
    let Some(item_type) = adapter.item_type.as_deref() else {
        return String::new();
    };
    let cs_item_type = csharp_type_name(item_type);
    let owner_cs_name = csharp_type_name(owner_type);

    let mut param_parts = vec!["IntPtr engine".to_string()];
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type = if param.ty.contains("::") {
            let parts: Vec<&str> = param.ty.split("::").collect();
            csharp_type_name(parts.last().unwrap_or(&"object"))
        } else {
            csharp_type_name(&param.ty)
        };
        param_parts.push(format!("{param_type} {param_name}"));
    }
    let params_decl = param_parts.join(", ");

    let mut setup_lines = Vec::new();
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type_pascal = to_csharp_name(param.ty.split("::").last().unwrap_or(""));
        setup_lines.push(format!(
            "        var {param_name}Json = JsonSerializer.Serialize({param_name}, JsonSerializationOptions);"
        ));
        setup_lines.push(format!(
            "        var {param_name}Handle = NativeMethods.{param_type_pascal}FromJson({param_name}Json);"
        ));
    }
    let setup_code = if setup_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", setup_lines.join("\n"))
    };

    let mut native_args = vec!["engine".to_string()];
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        native_args.push(format!("{param_name}Handle"));
    }
    let native_args_str = native_args.join(", ");

    let mut cleanup_lines = Vec::new();
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type_pascal = to_csharp_name(param.ty.split("::").last().unwrap_or(""));
        cleanup_lines.push(format!(
            "            NativeMethods.{param_type_pascal}Free({param_name}Handle);"
        ));
    }
    let cleanup_code = cleanup_lines.join("\n");

    let adapter_cs_name = to_csharp_name(adapter_name);
    let start_method = format!("{owner_cs_name}{adapter_cs_name}Start");
    let next_method = format!("{owner_cs_name}{adapter_cs_name}Next");
    let free_method = format!("{owner_cs_name}{adapter_cs_name}Free");

    render(
        "streaming_adapter_wrapper.jinja",
        minijinja::context! {
            item_type => cs_item_type,
            method_name,
            params_decl,
            setup_code,
            start_method,
            native_args => native_args_str,
            next_method,
            free_method,
            cleanup_code,
        },
    )
}
