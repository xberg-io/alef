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
) -> String {
    use crate::core::config::AdapterPattern;

    let adapter_name = &adapter.name;
    let js_name = to_node_name(adapter_name);
    let owner_type = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "napi adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });
    let js_owner_type = format!("Js{owner_type}");

    // When adapter.request_type is set and there's a single param, decompose the request struct
    // into its first field and use that as the function parameter (for ergonomic JS API).
    // E.g. CrawlStreamRequest { url: String } → accept (url string), construct req, call method.
    let (param_parts, param_conversions, core_params_list) = if adapter.request_type.is_some()
        && adapter.params.len() == 1
    {
        // Single request param: decompose by inspecting the request type's first field in IR.
        let param = &adapter.params[0];
        let param_ty_name = &param.ty;
        let ir_type = types.iter().find(|t| &t.name == param_ty_name);

        if let Some(ty_def) = ir_type {
            if ty_def.has_default {
                if let Some(first_field) = ty_def.fields.first() {
                    let field_name = &first_field.name;
                    // Handle Optional<T> by unwrapping to get T
                    let unwrapped_type = match &first_field.ty {
                        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
                        other => other,
                    };
                    let field_js_type = match unwrapped_type {
                        crate::core::ir::TypeRef::String => "String",
                        crate::core::ir::TypeRef::Bytes => "JsBytes",
                        crate::core::ir::TypeRef::Vec(inner) => {
                            // Vec<T> — determine T's type
                            match inner.as_ref() {
                                crate::core::ir::TypeRef::String => "Vec<String>",
                                crate::core::ir::TypeRef::Primitive(p) => {
                                    use crate::core::ir::PrimitiveType;
                                    match p {
                                        PrimitiveType::I32 => "Vec<i32>",
                                        PrimitiveType::I64 => "Vec<i64>",
                                        PrimitiveType::F64 => "Vec<f64>",
                                        PrimitiveType::Bool => "Vec<bool>",
                                        PrimitiveType::U8 => "Vec<u8>",
                                        _ => "Vec<String>", // Default fallback
                                    }
                                }
                                _ => "Vec<String>", // Default fallback
                            }
                        }
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
                                _ => "String", // Default fallback
                            }
                        }
                        _ => "String", // Fallback for complex types
                    };

                    let param_parts = vec![
                        format!("engine: &{js_owner_type}"),
                        format!("{field_name}: {field_js_type}"),
                    ];

                    let js_struct_name = format!("Js{param_ty_name}");
                    // Check if the field will become optional in the NAPI binding.
                    // Fields become optional when:
                    // 1. The field is already optional in the Rust IR, OR
                    // 2. The struct has Default derive (ty_def.has_default)
                    // This matches the logic in napi/gen_bindings/types.rs line 120:
                    // let field_type = if (field.optional || typ.has_default) && !already_optional
                    let is_field_optional_in_js = (first_field.optional || ty_def.has_default)
                        && !matches!(&first_field.ty, crate::core::ir::TypeRef::Optional(_));
                    let wrapped_field_value = if is_field_optional_in_js {
                        format!("Some({})", field_name)
                    } else {
                        field_name.clone()
                    };
                    // Use ..Default::default() to fill remaining fields — only safe because
                    // ty_def.has_default is true, which guarantees the JS struct derives Default.
                    // Omit the spread when the struct has exactly one field, because clippy's
                    // `needless_update` lint (denied under `-D warnings`) flags it as redundant.
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
                    // has_default but no fields: fallback to original behavior
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
                // has_default is false: struct has required fields, cannot safely decompose.
                // Fall through to the standard multi-param path.
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
            // Type not found in IR: fallback to original behavior
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
        // Multi-param or no request_type: use original behavior
        let mut param_parts = vec![format!("engine: &{js_owner_type}")];
        let mut param_conversions = Vec::new();

        for param in &adapter.params {
            let param_name = &param.name;
            let param_type = &param.ty;
            // Map to JS wrapper types for parameters
            let js_type = format!("Js{param_type}");
            param_parts.push(format!("{param_name}: {js_type}"));
            // Record conversion: "let core_req: {core_crate}::Type = req.into();"
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

        // Build the positional param list for core_req usage.
        let core_params_list = adapter
            .params
            .iter()
            .map(|p| format!("core_{}", p.name))
            .collect::<Vec<_>>()
            .join(", ");

        (param_parts, param_conversions, core_params_list)
    };

    match &adapter.pattern {
        AdapterPattern::Streaming => {
            // Streaming: replicate the instance method's channel/tokio spawn pattern.
            // The free function calls engine.inner.method(...).await to get the stream,
            // then wraps it in channels and spawns a background task.
            // item_type drives the Js{Type}::from(c) event cast in the channel loop.
            let item_type_name = adapter.item_type.as_deref().unwrap_or("Item");
            // Iterator struct name matches crate::adapters::streaming::iterator_name:
            // to_pascal_case(adapter_name) + "Iterator" (e.g. crawl_stream → CrawlStreamIterator).
            let return_iterator_type = format!("{}Iterator", adapter_name.to_pascal_case());

            let _method_call = if core_params_list.is_empty() {
                format!("engine.inner.{}()", adapter_name)
            } else {
                format!("engine.inner.{}({})", adapter_name, core_params_list)
            };

            let conversions_code = if param_conversions.is_empty() {
                String::new()
            } else {
                format!("{}\n", param_conversions.join("\n"))
            };

            // Generate the method call using the cloned inner engine
            let method_call_inner = if core_params_list.is_empty() {
                format!("inner.{}()", adapter_name)
            } else {
                format!("inner.{}({})", adapter_name, core_params_list)
            };

            format!(
                "#[allow(clippy::missing_errors_doc)]\n\
                 #[napi(js_name = \"{}\")]\n\
                 pub async fn {}({}) -> Result<{}> {{\n\
                 {}    let inner = engine.inner.clone();\n\
                     let (tx, rx) = tokio::sync::mpsc::channel(32);\n\
                     tokio::spawn(async move {{\n\
                         use futures_util::StreamExt;\n\
                         match {}.await {{\n\
                             Err(e) => {{\n\
                                 let _ = tx\n\
                                     .send(Err(napi::Error::new(napi::Status::GenericFailure, e.to_string())))\n\
                                     .await;\n\
                             }}\n\
                             Ok(mut stream) => {{\n\
                                 while let Some(chunk) = stream.next().await {{\n\
                                     let item = match chunk {{\n\
                                         Ok(c) => Js{}::from(c),\n\
                                         Err(e) => {{\n\
                                             let _ = tx\n\
                                                 .send(Err(napi::Error::new(napi::Status::GenericFailure, e.to_string())))\n\
                                                 .await;\n\
                                             break;\n\
                                         }}\n\
                                     }};\n\
                                     if tx.send(Ok(item)).await.is_err() {{\n\
                                         break;\n\
                                     }}\n\
                                 }}\n\
                             }}\n\
                         }}\n\
                     }});\n\
                     let iter = {} {{\n\
                         receiver: Arc::new(tokio::sync::Mutex::new(rx)),\n\
                     }};\n\
                     Ok(iter)\n\
                 }}\n\n",
                js_name,
                adapter_name,
                param_parts.join(", "),
                return_iterator_type,
                conversions_code,
                method_call_inner,
                item_type_name,
                return_iterator_type
            )
        }
        _ => String::new(), // Only Streaming pattern is relevant for NAPI
    }
}
