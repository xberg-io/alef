use crate::codegen::naming::to_node_name;
use heck::{ToPascalCase, ToSnakeCase};

pub(in crate::backends::napi::gen_bindings) fn gen_tokio_runtime() -> String {
    "static WORKER_POOL: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    // 16 MB worker stack: a deep consumer future (e.g. a multi-stage OCR pipeline) overflows the
    // default (~2 MB) worker stack and aborts the process with SIGBUS.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect(\"Failed to create Tokio runtime\")
});"
    .to_string()
}

/// Emit a module-level wrapper function for an adapter (streaming method).
pub(in crate::backends::napi::gen_bindings) fn gen_adapter_wrapper(
    adapter: &crate::core::config::AdapterConfig,
    core_crate: &str,
    types: &[crate::core::ir::TypeDef],
    inputs: Option<&crate::backends::napi::gen_bindings::containers::ContainerContext>,
) -> anyhow::Result<String> {
    use crate::core::config::AdapterPattern;

    let adapter_name = &adapter.name;
    let js_name = to_node_name(adapter_name);
    let owner_type = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "napi adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });
    let js_owner_type = format!("Js{owner_type}");

    let (param_parts, param_conversions, core_params_list) = if adapter.request_type.is_some()
        && adapter.params.len() == 1
    {
        let param = &adapter.params[0];
        let param_ty_name = &param.ty;
        let ir_type = types.iter().find(|t| &t.name == param_ty_name);

        if let Some(ty_def) = ir_type {
            if ty_def.has_default {
                if let Some(first_field) = ty_def.fields.first() {
                    let field_name = &first_field.name;
                    let unwrapped_type = match &first_field.ty {
                        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
                        other => other,
                    };
                    let field_js_type = match unwrapped_type {
                        crate::core::ir::TypeRef::String => "String",
                        crate::core::ir::TypeRef::Bytes => "JsBytes",
                        crate::core::ir::TypeRef::Vec(inner) => match inner.as_ref() {
                            crate::core::ir::TypeRef::String => "Vec<String>",
                            crate::core::ir::TypeRef::Primitive(p) => {
                                use crate::core::ir::PrimitiveType;
                                match p {
                                    PrimitiveType::I32 => "Vec<i32>",
                                    PrimitiveType::I64 => "Vec<i64>",
                                    PrimitiveType::F64 => "Vec<f64>",
                                    PrimitiveType::Bool => "Vec<bool>",
                                    PrimitiveType::U8 => "Vec<u8>",
                                    _ => "Vec<String>",
                                }
                            }
                            _ => "Vec<String>",
                        },
                        crate::core::ir::TypeRef::Primitive(p) => {
                            use crate::core::ir::PrimitiveType;
                            match p {
                                PrimitiveType::I32 => "i32",
                                PrimitiveType::I64 => "i64",
                                PrimitiveType::F64 => "f64",
                                PrimitiveType::Bool => "bool",
                                PrimitiveType::U8 => "u8",
                                PrimitiveType::U32 => "u32",
                                PrimitiveType::Usize => "usize",
                                _ => "String",
                            }
                        }
                        _ => "String",
                    };

                    let param_parts = vec![
                        format!("engine: &{js_owner_type}"),
                        format!("{field_name}: {field_js_type}"),
                    ];

                    let js_struct_name = format!("Js{param_ty_name}");
                    let is_field_optional_in_js = (first_field.optional || ty_def.has_default)
                        && !matches!(&first_field.ty, crate::core::ir::TypeRef::Optional(_));
                    let wrapped_field_value = if is_field_optional_in_js {
                        format!("Some({})", field_name)
                    } else {
                        field_name.clone()
                    };
                    let core_var_name = format!("core_{}", param_ty_name.to_snake_case());
                    let default_spread = if ty_def.fields.len() > 1 {
                        ", ..Default::default()"
                    } else {
                        ""
                    };
                    let param_conversions = vec![format!(
                        "    let {core_var_name}: {core_crate}::{param_ty_name} = {js_struct_name} {{ {field_name}: {wrapped_field_value}{default_spread} }}.into();",
                        core_var_name = core_var_name,
                        param_ty_name = param_ty_name,
                        js_struct_name = js_struct_name,
                        field_name = field_name,
                        core_crate = core_crate,
                        default_spread = default_spread,
                    )];

                    let core_params = core_var_name;

                    (param_parts, param_conversions, core_params)
                } else {
                    let mut param_parts = vec![format!("engine: &{js_owner_type}")];
                    let mut param_conversions = Vec::new();
                    for param in &adapter.params {
                        let param_name = &param.name;
                        let param_type = &param.ty;
                        let js_type = format!("Js{param_type}");
                        param_parts.push(format!("{param_name}: {js_type}"));
                        let core_type = if param_type.contains("::") {
                            param_type.clone()
                        } else {
                            format!("{core_crate}::{param_type}")
                        };
                        param_conversions.push(format!(
                            "    let core_{}: {} = {}.into();",
                            param_name, core_type, param_name
                        ));
                    }
                    let core_params_list = adapter
                        .params
                        .iter()
                        .map(|p| format!("core_{}", p.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (param_parts, param_conversions, core_params_list)
                }
            } else {
                let mut param_parts = vec![format!("engine: &{js_owner_type}")];
                let mut param_conversions = Vec::new();
                for param in &adapter.params {
                    let param_name = &param.name;
                    let param_type = &param.ty;
                    let js_type = format!("Js{param_type}");
                    param_parts.push(format!("{param_name}: {js_type}"));
                    let core_type = if param_type.contains("::") {
                        param_type.clone()
                    } else {
                        format!("{core_crate}::{param_type}")
                    };
                    param_conversions.push(format!(
                        "    let core_{}: {} = {}.into();",
                        param_name, core_type, param_name
                    ));
                }
                let core_params_list = adapter
                    .params
                    .iter()
                    .map(|p| format!("core_{}", p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                (param_parts, param_conversions, core_params_list)
            }
        } else {
            let mut param_parts = vec![format!("engine: &{js_owner_type}")];
            let mut param_conversions = Vec::new();
            for param in &adapter.params {
                let param_name = &param.name;
                let param_type = &param.ty;
                let js_type = format!("Js{param_type}");
                param_parts.push(format!("{param_name}: {js_type}"));
                let core_type = if param_type.contains("::") {
                    param_type.clone()
                } else {
                    format!("{core_crate}::{param_type}")
                };
                param_conversions.push(format!(
                    "    let core_{}: {} = {}.into();",
                    param_name, core_type, param_name
                ));
            }
            let core_params_list = adapter
                .params
                .iter()
                .map(|p| format!("core_{}", p.name))
                .collect::<Vec<_>>()
                .join(", ");
            (param_parts, param_conversions, core_params_list)
        }
    } else {
        let mut param_parts = vec![format!("engine: &{js_owner_type}")];
        let mut param_conversions = Vec::new();

        for param in &adapter.params {
            let param_name = &param.name;
            let param_type = &param.ty;
            let js_type = format!("Js{param_type}");
            param_parts.push(format!("{param_name}: {js_type}"));
            let core_type = if param_type.contains("::") {
                param_type.clone()
            } else {
                format!("{core_crate}::{param_type}")
            };
            param_conversions.push(format!(
                "    let core_{}: {} = {}.into();",
                param_name, core_type, param_name
            ));
        }

        let core_params_list = adapter
            .params
            .iter()
            .map(|p| format!("core_{}", p.name))
            .collect::<Vec<_>>()
            .join(", ");

        (param_parts, param_conversions, core_params_list)
    };

    let param_conversions = param_conversions
        .into_iter()
        .map(|conversion| {
            let affected = adapter.params.iter().any(|parameter| {
                let name = parameter.ty.rsplit("::").next().unwrap_or(&parameter.ty);
                inputs.is_some_and(|inputs| inputs.is_fallible(&crate::core::ir::TypeRef::Named(name.to_string())))
                    && (conversion.contains(&format!("core_{}:", parameter.name))
                        || conversion.contains(&format!("core_{}:", name.to_snake_case())))
            });
            if affected {
                conversion.replace(
                    ".into();",
                    &format!(
                        ".try_into(){}?;",
                        crate::backends::napi::gen_bindings::inputs::INVALID_ARG
                    ),
                )
            } else {
                conversion
            }
        })
        .collect::<Vec<_>>();

    Ok(match &adapter.pattern {
        AdapterPattern::Streaming => {
            let item_type_name = adapter.item_type.as_deref().unwrap_or("Item");
            let return_iterator_type = format!("{}Iterator", adapter_name.to_pascal_case());

            let conversions_code = if param_conversions.is_empty() {
                String::new()
            } else {
                format!("{}\n", param_conversions.join("\n"))
            };

            let method_call_inner = crate::adapters::rust_callable::receiver_call(
                &adapter.core_path,
                "inner",
                "&inner",
                &core_params_list,
            )?;

            // See `src/adapters/streaming.rs`'s pyo3/napi bodies for why `tx.closed()` is raced
            // against `stream.next()`: it stops pulling from `stream` as soon as the JS-side
            // consumer drops the async iterator, instead of only noticing on the next send. The
            // race cannot drop a produced item — `StreamExt::next` either returns `Pending`
            // (nothing produced, safe to drop) or `Ready` (already claimed by `select!`).

            crate::backends::napi::template_env::render(
                "streaming_adapter_wrapper.rs.jinja",
                crate::alef_context! {
                    js_name => &js_name,
                    adapter_name => adapter_name,
                    params_str => param_parts.join(", "),
                    iterator_type => &return_iterator_type,
                    conversions_code => &conversions_code,
                    core_call => &method_call_inner,
                    item_type_name => item_type_name,
                },
            )
        }
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};

    fn streaming_adapter() -> AdapterConfig {
        AdapterConfig {
            name: "list_items".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "list_items".to_string(),
            params: vec![AdapterParam {
                name: "cursor".to_string(),
                ty: "String".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: None,
            owner_type: Some("Client".to_string()),
            item_type: Some("Item".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: vec![],
        }
    }

    /// Regression test for the napi module-level streaming wrapper (the "second copy" of the
    /// napi streaming forwarder, distinct from `src/adapters/streaming.rs`'s `gen_node_body`).
    /// Before the fix, this wrapper only noticed a closed consumer on the next `tx.send`, so a
    /// JS-side `for await` loop that broke early left the forwarder pulling one more upstream
    /// item. It must instead race `tx.closed()` against `stream.next()` and break immediately.
    #[test]
    fn streaming_wrapper_selects_on_consumer_close() {
        let adapter = streaming_adapter();

        let body = gen_adapter_wrapper(&adapter, "my_crate", &[], None).expect("valid adapter");

        assert!(
            body.contains("_ = tx.closed() => break,"),
            "streaming wrapper must race the sender's `closed()` future against `stream.next()` \
             so a dropped JS consumer stops the forwarder without waiting for another item. Got: {body}"
        );
        assert!(
            body.contains("tokio::select! {"),
            "streaming wrapper must select between consumer close and the next stream item. Got: {body}"
        );
        assert!(
            body.contains("drop(stream);"),
            "streaming wrapper must drop the core stream as soon as the forward loop exits. Got: {body}"
        );
    }
}
