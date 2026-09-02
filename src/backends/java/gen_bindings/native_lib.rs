use crate::core::config::{AdapterPattern, BridgeBinding, ResolvedCrateConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EntrypointKind, MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::BTreeSet;

use super::marshal::{
    gen_ffi_layout_with_enums, gen_function_descriptor, is_bytes_result, is_ffi_string_return, push_param_layouts,
};
use super::result_presence::presence_handle_declaration;

/// Returns true if the FFI backend exports a `{type_name}_to_json` symbol for this type.
/// This matches the predicate in `src/backends/ffi/gen_bindings/mod.rs`:
/// - Opaque types do NOT get `_to_json` (they are handles, not serializable values)
/// - Types without serde derives do NOT get `_to_json`
/// - Update types (ending with "Update") do NOT get `_to_json` (deserialize-only)
/// - Types with an existing `to_json` method do NOT get auto `_to_json` (method collision)
fn should_emit_to_json_handle(typ: &TypeDef) -> bool {
    !typ.is_opaque && typ.has_serde && !typ.name.ends_with("Update") && !typ.methods.iter().any(|m| m.name == "to_json")
}

/// Returns true if the FFI backend exports a `{type_name}_from_json` symbol for this type.
/// This matches the predicate in `src/backends/ffi/gen_bindings/mod.rs`:
/// - Opaque types do NOT get `_from_json` (they are handles, not serializable values)
/// - Types without serde derives do NOT get `_from_json`
fn should_emit_from_json_handle(typ: &TypeDef) -> bool {
    !typ.is_opaque && typ.has_serde
}

/// Detection mirroring `is_bytes_result` for `MethodDef` — `Result<Vec<u8>>`-returning
/// methods use the (out_ptr, out_len, out_cap) triple FFI ABI.
fn is_bytes_result_method(method: &MethodDef) -> bool {
    if method.error_type.is_none() {
        return false;
    }
    matches!(method.return_type, TypeRef::Bytes)
        || matches!(&method.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
}

fn extract_required_symbols(blocks: impl IntoIterator<Item = impl AsRef<str>>) -> BTreeSet<String> {
    const FIND_PREFIX: &str = "LIB.find(\"";
    let mut symbols = BTreeSet::new();

    for block in blocks {
        let mut remaining = block.as_ref();
        while let Some(start) = remaining.find(FIND_PREFIX) {
            remaining = &remaining[start + FIND_PREFIX.len()..];
            let Some(end) = remaining.find('"') else {
                break;
            };
            let symbol = &remaining[..end];
            if !symbol.starts_with('_') {
                symbols.insert(symbol.to_string());
            }
            remaining = &remaining[end + 1..];
        }
    }

    symbols
}

fn service_entrypoint_is_exported(entrypoint: &crate::core::ir::EntrypointDef, api: &ApiSurface) -> bool {
    if !matches!(entrypoint.kind, EntrypointKind::Finalize) {
        return true;
    }
    match &entrypoint.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(name) => api.types.iter().any(|typ| typ.name == *name),
        _ => false,
    }
}

fn service_required_symbols(api: &ApiSurface, prefix: &str) -> BTreeSet<String> {
    let mut symbols = BTreeSet::new();
    for service in &api.services {
        let service_name = service.name.to_snake_case();
        symbols.insert(format!("{prefix}_{service_name}_new"));
        symbols.insert(format!("{prefix}_{service_name}_free"));
        for registration in &service.registrations {
            symbols.insert(format!(
                "{prefix}_{service_name}_register_{}",
                registration.method.to_snake_case()
            ));
            for variant in registration
                .variants
                .iter()
                .filter(|variant| variant.wrapper_call.is_some())
            {
                symbols.insert(format!("{prefix}_{service_name}_{}", variant.name.to_snake_case()));
            }
        }
        for configurator in &service.configurators {
            symbols.insert(format!("{prefix}_{service_name}_{}", configurator.name.to_snake_case()));
        }
        for entrypoint in &service.entrypoints {
            if service_entrypoint_is_exported(entrypoint, api) {
                symbols.insert(format!(
                    "{prefix}_{service_name}_ep_{}",
                    entrypoint.method.to_snake_case()
                ));
            }
        }
    }
    symbols
}

pub(crate) fn gen_native_lib(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    prefix: &str,
    has_visitor_pattern: bool,
) -> String {
    let lib_name = config.ffi_lib_name();

    let trait_bridge_handles: AHashSet<String> = config
        .trait_bridges
        .iter()
        .filter(|b| {
            !b.exclude_languages
                .contains(&crate::core::config::Language::Java.to_string())
        })
        .filter(|bridge| !(has_visitor_pattern && bridge.bind_via == BridgeBinding::OptionsField))
        .flat_map(|b| {
            let trait_snake = b.trait_name.to_snake_case();
            let trait_upper = trait_snake.to_uppercase();
            let mut handles = vec![
                format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper),
                format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper),
            ];
            if let Some(clear_fn) = &b.clear_fn {
                handles.push(format!("{}_{}", prefix.to_uppercase(), clear_fn.to_uppercase()));
            }
            handles
        })
        .collect();

    let bridge_type_aliases: AHashSet<String> = config
        .trait_bridges
        .iter()
        .filter(|b| {
            !b.exclude_languages
                .contains(&crate::core::config::Language::Java.to_string())
        })
        .filter_map(|b| b.type_alias.clone())
        .collect();

    let ffi_excluded: AHashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let mut function_handles = Vec::new();
    let mut optional_symbols = BTreeSet::new();

    if !config.components.is_empty() {
        let prefix_upper = prefix.to_uppercase();
        for (handle_suffix, symbol_suffix, descriptor) in [
            (
                "COMPONENT_LOAD",
                "component_load",
                "FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS)",
            ),
            (
                "COMPONENT_PREFETCH",
                "component_prefetch",
                "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)",
            ),
            (
                "COMPONENT_STATUS",
                "component_status",
                "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)",
            ),
            (
                "COMPONENT_CACHE_PATH",
                "component_cache_path",
                "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)",
            ),
        ] {
            function_handles.push(crate::backends::java::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => format!("{prefix_upper}_{handle_suffix}"),
                    ffi_name => format!("{prefix}_{symbol_suffix}"),
                    layout => descriptor,
                },
            ));
        }
    }

    for func in &api.functions {
        let handle_name = format!("{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

        if trait_bridge_handles.contains(&handle_name) {
            continue;
        }

        let ffi_name = format!("{}_{}", prefix, func.name.to_lowercase());

        let mut param_layouts: Vec<String> = Vec::new();
        push_param_layouts(&func.params, &enum_names, &mut param_layouts);
        let return_layout = if is_bytes_result(func) {
            param_layouts.push("ValueLayout.ADDRESS".to_string());
            param_layouts.push("ValueLayout.ADDRESS".to_string());
            param_layouts.push("ValueLayout.ADDRESS".to_string());
            "ValueLayout.JAVA_LONG".to_string()
        } else {
            gen_ffi_layout_with_enums(&func.return_type, &enum_names)
        };

        let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

        let handle_code = if ffi_excluded.contains(&func.name) {
            optional_symbols.insert(ffi_name.clone());
            crate::backends::java::template_env::render(
                "method_handle_nullable.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        } else {
            crate::backends::java::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            )
        };
        function_handles.push(handle_code);

        // Deliberately gated on `result_presence_companion_exists` alone, with no second
        // condition: the wrapper emitter in `ffi_class::sync_functions::returns` consults that
        // same predicate and cannot see `ffi_excluded`, so any extra condition here would let the
        // declaration and the call site disagree and reference an undeclared handle. ~keep
        if let Some(handle) =
            presence_handle_declaration(&func.return_type, None, &handle_name, &ffi_name, &param_layouts)
        {
            function_handles.push(handle);
        }

        if is_ffi_string_return(&func.return_type) {
            let len_handle_name = format!("{}_{}_LEN", prefix.to_uppercase(), func.name.to_uppercase());
            let len_ffi_name = format!("{}_{}_len", prefix, func.name.to_lowercase());
            let len_layout = gen_function_descriptor("ValueLayout.JAVA_LONG", &param_layouts);
            function_handles.push(crate::backends::java::template_env::render(
                "method_handle_len.jinja",
                minijinja::context! {
                    handle_name => len_handle_name,
                    ffi_name => len_ffi_name,
                    layout => len_layout,
                },
            ));
        }
    }

    {
        let free_name = format!("{}_free_string", prefix);
        let handle_name = format!("{}_FREE_STRING", prefix.to_uppercase());
        let handle_code = crate::backends::java::template_env::render(
            "method_handle_free.jinja",
            minijinja::context! {
                handle_name => handle_name,
                ffi_name => free_name,
            },
        );
        function_handles.push(handle_code);
    }

    let has_bytes_results = api.functions.iter().any(is_bytes_result)
        || api
            .types
            .iter()
            .any(|typ| typ.methods.iter().any(is_bytes_result_method));
    if has_bytes_results {
        let free_bytes_name = format!("{}_free_bytes", prefix);
        let handle_name = format!("{}_FREE_BYTES", prefix.to_uppercase());
        let handle_code = crate::backends::java::template_env::render(
            "method_handle_free_bytes.jinja",
            minijinja::context! {
                handle_name => handle_name,
                ffi_name => free_bytes_name,
            },
        );
        function_handles.push(handle_code);
    }

    let mut emitted_free_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_to_json_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_from_json_handles: AHashSet<String> = AHashSet::new();

    let opaque_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let to_json_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| should_emit_to_json_handle(t))
        .map(|t| t.name.clone())
        .chain(api.enums.iter().filter(|e| e.has_serde).map(|e| e.name.clone()))
        .collect();
    let from_json_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| should_emit_from_json_handle(t))
        .map(|t| t.name.clone())
        .collect();

    let mut accessor_handles = Vec::new();

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        let type_upper = type_snake.to_uppercase();
        if should_emit_from_json_handle(typ) {
            let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
            let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
            if emitted_from_json_handles.insert(from_json_handle.clone()) {
                accessor_handles.push(crate::backends::java::template_env::render(
                    "method_handle_from_json.jinja",
                    minijinja::context! {
                        handle_name => from_json_handle,
                        ffi_name => from_json_ffi,
                    },
                ));
            }
        }
        let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
        let free_ffi = format!("{}_{}_free", prefix, type_snake);
        if emitted_free_handles.insert(free_handle.clone()) {
            accessor_handles.push(crate::backends::java::template_env::render(
                "method_handle_free.jinja",
                minijinja::context! {
                    handle_name => free_handle,
                    ffi_name => free_ffi,
                },
            ));
        }
    }

    for func in &api.functions {
        let inner_named = match &func.return_type {
            TypeRef::Named(n) => Some(n),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(name) = inner_named {
            let type_snake = name.to_snake_case();
            let type_upper = type_snake.to_uppercase();

            if to_json_type_names.contains(name.as_str()) {
                let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
                let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
                if emitted_to_json_handles.insert(to_json_handle.clone()) {
                    let handle_code = crate::backends::java::template_env::render(
                        "method_handle_to_json.jinja",
                        minijinja::context! {
                            handle_name => to_json_handle,
                            ffi_name => to_json_ffi,
                        },
                    );
                    accessor_handles.push(handle_code);
                }
            }

            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_free.jinja",
                    minijinja::context! {
                        handle_name => free_handle,
                        ffi_name => free_ffi,
                    },
                );
                accessor_handles.push(handle_code);
            }
        }
    }

    for func in &api.functions {
        for param in &func.params {
            let inner_name = match &param.ty {
                TypeRef::Named(n) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(name) = inner_name
                && !opaque_type_names.contains(name.as_str())
                && !bridge_type_aliases.contains(name.as_str())
            {
                let type_snake = name.to_snake_case();
                let type_upper = type_snake.to_uppercase();

                let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                if emitted_from_json_handles.insert(from_json_handle.clone()) {
                    let handle_code = crate::backends::java::template_env::render(
                        "method_handle_from_json.jinja",
                        minijinja::context! {
                            handle_name => from_json_handle,
                            ffi_name => from_json_ffi,
                        },
                    );
                    accessor_handles.push(handle_code);
                }

                let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                let free_ffi = format!("{}_{}_free", prefix, type_snake);
                if emitted_free_handles.insert(free_handle.clone()) {
                    let handle_code = crate::backends::java::template_env::render(
                        "method_handle_free.jinja",
                        minijinja::context! {
                            handle_name => free_handle,
                            ffi_name => free_ffi,
                        },
                    );
                    accessor_handles.push(handle_code);
                }
            }
        }
    }

    let builder_class_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.fields.is_empty() && t.has_default)
        .map(|t| format!("{}Builder", t.name))
        .collect();

    let mut builder_handles = Vec::new();

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque && !builder_class_names.contains(&typ.name) {
            let type_snake = typ.name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let prefix_upper = prefix.to_uppercase();

            let free_handle = format!("{}_{}_FREE", prefix_upper, type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_free.jinja",
                    minijinja::context! {
                        handle_name => free_handle,
                        ffi_name => free_ffi,
                    },
                );
                builder_handles.push(handle_code);
            }
        }
    }

    let mut trait_handles = Vec::new();
    let mut emitted_register_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_unregister_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_clear_handles: AHashSet<String> = AHashSet::new();

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .contains(&crate::core::config::Language::Java.to_string())
        {
            continue;
        }
        if has_visitor_pattern && bridge_cfg.bind_via == BridgeBinding::OptionsField {
            continue;
        }

        let trait_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_upper = trait_snake.to_uppercase();
        let vtable_layout = "ValueLayout.ADDRESS".to_string();

        let register_handle_name = format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper);
        let register_ffi_name = format!("{}_register_{}", prefix, trait_snake);
        if emitted_register_handles.insert(register_handle_name.clone()) {
            let handle_code = crate::backends::java::template_env::render(
                "method_handle_register.jinja",
                minijinja::context! {
                    handle_name => register_handle_name,
                    ffi_name => register_ffi_name,
                    vtable_layout => &vtable_layout,
                },
            );
            trait_handles.push(handle_code);
        }

        if bridge_cfg.unregister_fn.is_some() {
            let unregister_handle_name = format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper);
            let unregister_ffi_name = format!("{}_unregister_{}", prefix, trait_snake);
            if emitted_unregister_handles.insert(unregister_handle_name.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_unregister.jinja",
                    minijinja::context! {
                        handle_name => unregister_handle_name,
                        ffi_name => unregister_ffi_name,
                    },
                );
                trait_handles.push(handle_code);
            }
        }

        if bridge_cfg.clear_fn.is_some() {
            let clear_handle_name = format!("{}_CLEAR_{}", prefix.to_uppercase(), trait_upper);
            let clear_ffi_name = format!("{}_clear_{}", prefix, trait_snake);
            if emitted_clear_handles.insert(clear_handle_name.clone()) {
                let handle_code = crate::backends::java::template_env::render(
                    "method_handle_clear.jinja",
                    minijinja::context! {
                        handle_name => clear_handle_name,
                        ffi_name => clear_ffi_name,
                    },
                );
                trait_handles.push(handle_code);
            }
        }
    }

    let stream_item_types: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| a.item_type.as_ref())
        .cloned()
        .collect();
    let stream_request_types: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| a.params.first().map(|p| p.ty.as_str()))
        .filter_map(|ty| ty.rsplit("::").next())
        .map(|s| s.to_string())
        .collect();

    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        let Some(owner_type) = adapter.owner_type.as_deref() else {
            continue;
        };
        let Some(item_type) = adapter.item_type.as_deref() else {
            continue;
        };
        let Some(request_type) = adapter.params.first().map(|p| p.ty.as_str()).filter(|s| !s.is_empty()) else {
            continue;
        };

        let owner_snake = owner_type.to_snake_case();
        let owner_upper = owner_snake.to_uppercase();
        let adapter_snake = adapter.name.to_snake_case();
        let adapter_upper = adapter_snake.to_uppercase();
        let prefix_upper = prefix.to_uppercase();

        let start_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_START");
        let start_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_start");
        let start_layout =
            "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)".to_string();
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_normal.jinja",
            minijinja::context! {
                handle_name => start_handle,
                ffi_name => start_ffi,
                layout => start_layout,
            },
        ));

        let next_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_NEXT");
        let next_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_next");
        let next_layout = "FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)".to_string();
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_normal.jinja",
            minijinja::context! {
                handle_name => next_handle,
                ffi_name => next_ffi,
                layout => next_layout,
            },
        ));

        let free_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_FREE");
        let free_ffi = format!("{prefix}_{owner_snake}_{adapter_snake}_free");
        accessor_handles.push(crate::backends::java::template_env::render(
            "method_handle_free.jinja",
            minijinja::context! {
                handle_name => free_handle,
                ffi_name => free_ffi,
            },
        ));

        let request_snake = request_type.to_snake_case();
        let request_upper = request_snake.to_uppercase();

        if from_json_type_names.contains(request_type) || stream_request_types.contains(request_type) {
            let req_from_json_handle = format!("{prefix_upper}_{request_upper}_FROM_JSON");
            let req_from_json_ffi = format!("{prefix}_{request_snake}_from_json");
            if emitted_from_json_handles.insert(req_from_json_handle.clone()) {
                accessor_handles.push(crate::backends::java::template_env::render(
                    "method_handle_from_json.jinja",
                    minijinja::context! {
                        handle_name => req_from_json_handle,
                        ffi_name => req_from_json_ffi,
                    },
                ));
            }
        }
        let req_free_handle = format!("{prefix_upper}_{request_upper}_FREE");
        let req_free_ffi = format!("{prefix}_{request_snake}_free");
        if emitted_free_handles.insert(req_free_handle.clone()) {
            accessor_handles.push(crate::backends::java::template_env::render(
                "method_handle_free.jinja",
                minijinja::context! {
                    handle_name => req_free_handle,
                    ffi_name => req_free_ffi,
                },
            ));
        }

        let item_snake = item_type.to_snake_case();
        let item_upper = item_snake.to_uppercase();

        if to_json_type_names.contains(item_type) || stream_item_types.contains(item_type) {
            let item_to_json_handle = format!("{prefix_upper}_{item_upper}_TO_JSON");
            let item_to_json_ffi = format!("{prefix}_{item_snake}_to_json");
            if emitted_to_json_handles.insert(item_to_json_handle.clone()) {
                accessor_handles.push(crate::backends::java::template_env::render(
                    "method_handle_to_json.jinja",
                    minijinja::context! {
                        handle_name => item_to_json_handle,
                        ffi_name => item_to_json_ffi,
                    },
                ));
            }
        }
        let item_free_handle = format!("{prefix_upper}_{item_upper}_FREE");
        let item_free_ffi = format!("{prefix}_{item_snake}_free");
        if emitted_free_handles.insert(item_free_handle.clone()) {
            accessor_handles.push(crate::backends::java::template_env::render(
                "method_handle_free.jinja",
                minijinja::context! {
                    handle_name => item_free_handle,
                    ffi_name => item_free_ffi,
                },
            ));
        }
    }

    // NOTE: This loop only processes NON-OPAQUE types. Opaque types do NOT have FFI
    let streaming_adapter_method_keys: AHashSet<(String, String)> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.clone()?;
            Some((owner, a.name.to_snake_case()))
        })
        .collect();
    for typ in api.types.iter().filter(|t| t.is_opaque && !t.is_trait) {
        for method in &typ.methods {
            if streaming_adapter_method_keys.contains(&(typ.name.clone(), method.name.to_snake_case())) {
                continue;
            }
            if matches!(method.name.as_str(), "default" | "to_json" | "from_json") {
                continue;
            }
            if method.returns_ref_to_owner(&typ.name) {
                continue;
            }
            let owner_snake = typ.name.to_snake_case();
            let owner_upper = owner_snake.to_uppercase();
            let method_snake = method.name.to_snake_case();
            let method_upper = method_snake.to_uppercase();
            let handle_name = format!("{}_{}_{}", prefix.to_uppercase(), owner_upper, method_upper);
            let ffi_name = format!("{}_{}_{}", prefix, owner_snake, method_snake);

            let mut param_layouts: Vec<String> = if method.is_static {
                Vec::new()
            } else {
                vec!["ValueLayout.ADDRESS".to_string()]
            };
            push_param_layouts(&method.params, &enum_names, &mut param_layouts);
            let return_layout = if is_bytes_result_method(method) {
                param_layouts.push("ValueLayout.ADDRESS".to_string());
                param_layouts.push("ValueLayout.ADDRESS".to_string());
                param_layouts.push("ValueLayout.ADDRESS".to_string());
                "ValueLayout.JAVA_LONG".to_string()
            } else {
                gen_ffi_layout_with_enums(&method.return_type, &enum_names)
            };
            let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

            let handle_code = crate::backends::java::template_env::render(
                "method_handle_normal.jinja",
                minijinja::context! {
                    handle_name => handle_name,
                    ffi_name => ffi_name,
                    layout => layout_str,
                },
            );
            function_handles.push(handle_code);

            if let Some(handle) = presence_handle_declaration(
                &method.return_type,
                method.receiver.as_ref(),
                &handle_name,
                &ffi_name,
                &param_layouts,
            ) {
                function_handles.push(handle);
            }

            let return_named = match &method.return_type {
                TypeRef::Named(n) => Some(n.clone()),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => Some(n.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(name) = return_named {
                let type_snake = name.to_snake_case();
                let type_upper = type_snake.to_uppercase();

                if to_json_type_names.contains(&name) {
                    let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
                    let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
                    if emitted_to_json_handles.insert(to_json_handle.clone()) {
                        accessor_handles.push(crate::backends::java::template_env::render(
                            "method_handle_to_json.jinja",
                            minijinja::context! {
                                handle_name => to_json_handle,
                                ffi_name => to_json_ffi,
                            },
                        ));
                    }
                }
                let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                let free_ffi = format!("{}_{}_free", prefix, type_snake);
                if emitted_free_handles.insert(free_handle.clone()) {
                    accessor_handles.push(crate::backends::java::template_env::render(
                        "method_handle_free.jinja",
                        minijinja::context! {
                            handle_name => free_handle,
                            ffi_name => free_ffi,
                        },
                    ));
                }
            }

            for p in &method.params {
                let param_named = match &p.ty {
                    TypeRef::Named(n) => Some(n.clone()),
                    TypeRef::Optional(inner) => match inner.as_ref() {
                        TypeRef::Named(n) => Some(n.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = param_named
                    && !bridge_type_aliases.contains(name.as_str())
                    && from_json_type_names.contains(name.as_str())
                {
                    let type_snake = name.to_snake_case();
                    let type_upper = type_snake.to_uppercase();
                    let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                    let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                    if emitted_from_json_handles.insert(from_json_handle.clone()) {
                        accessor_handles.push(crate::backends::java::template_env::render(
                            "method_handle_from_json.jinja",
                            minijinja::context! {
                                handle_name => from_json_handle,
                                ffi_name => from_json_ffi,
                            },
                        ));
                    }
                    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                    let free_ffi = format!("{}_{}_free", prefix, type_snake);
                    if emitted_free_handles.insert(free_handle.clone()) {
                        accessor_handles.push(crate::backends::java::template_env::render(
                            "method_handle_free.jinja",
                            minijinja::context! {
                                handle_name => free_handle,
                                ffi_name => free_ffi,
                            },
                        ));
                    }
                }
            }
        }
    }

    let visitor_handles = if has_visitor_pattern {
        let options_fields: Vec<String> = config
            .trait_bridges
            .iter()
            .filter(|bridge| bridge.bind_via == BridgeBinding::OptionsField)
            .filter_map(|bridge| bridge.resolved_options_field().map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        crate::backends::java::gen_visitor::gen_native_lib_visitor_handles(prefix, &options_fields)
    } else {
        String::new()
    };

    let handle_blocks = function_handles
        .iter()
        .chain(&accessor_handles)
        .chain(&builder_handles)
        .chain(&trait_handles)
        .map(String::as_str)
        .chain(std::iter::once(visitor_handles.as_str()));
    let mut required_symbols = extract_required_symbols(handle_blocks);
    required_symbols.extend(service_required_symbols(api, prefix));
    required_symbols.retain(|symbol| !optional_symbols.contains(symbol));
    required_symbols.insert(format!("{prefix}_last_error_code"));
    required_symbols.insert(format!("{prefix}_last_error_context"));
    let required_symbols: Vec<String> = required_symbols.into_iter().collect();
    let library_environment_prefix: String = lib_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();

    let class_body = crate::backends::java::template_env::render(
        "native_lib.jinja",
        minijinja::context! {
            class_name => "NativeLib",
            lib_name => lib_name,
            library_environment_prefix => library_environment_prefix,
            prefix => prefix,
            prefix_upper => prefix.to_uppercase(),
            required_symbols => required_symbols,
            function_handles => function_handles,
            accessor_handles => accessor_handles,
            builder_handles => builder_handles,
            trait_handles => trait_handles,
            visitor_handles => visitor_handles,
        },
    );

    let mut out = String::with_capacity(class_body.len() + 512);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("package ");
    out.push_str(package);
    out.push_str(";\n\n");

    if class_body.contains("Arena") {
        out.push_str("import java.lang.foreign.Arena;\n");
    }
    if class_body.contains("FunctionDescriptor") {
        out.push_str("import java.lang.foreign.FunctionDescriptor;\n");
    }
    if class_body.contains("Linker") {
        out.push_str("import java.lang.foreign.Linker;\n");
    }
    if class_body.contains("MemoryLayout") {
        out.push_str("import java.lang.foreign.MemoryLayout;\n");
    }
    if class_body.contains("MemorySegment") {
        out.push_str("import java.lang.foreign.MemorySegment;\n");
    }
    if class_body.contains("SymbolLookup") {
        out.push_str("import java.lang.foreign.SymbolLookup;\n");
    }
    if class_body.contains("ValueLayout") {
        out.push_str("import java.lang.foreign.ValueLayout;\n");
    }
    if class_body.contains("MethodHandle") {
        out.push_str("import java.lang.invoke.MethodHandle;\n");
    }
    out.push_str("import java.io.File;\n");
    out.push_str("import java.net.URL;\n");
    out.push_str("import java.nio.file.Files;\n");
    out.push_str("import java.nio.file.Path;\n");
    out.push_str("import java.nio.file.Paths;\n");
    out.push_str("import java.nio.file.StandardCopyOption;\n");
    out.push_str("import java.util.ArrayList;\n");
    out.push_str("import java.util.Enumeration;\n");
    out.push_str("import java.util.List;\n");
    out.push_str("import java.util.jar.JarEntry;\n");
    out.push_str("import java.util.jar.JarFile;\n");
    out.push('\n');

    out.push_str(&class_body);

    // `NativeLib.java`'s Panama descriptors hand-specify the handle as `ValueLayout.ADDRESS` /
    // `ValueLayout.JAVA_LONG`; nothing here derives them from the cbindgen header, so a
    // pointer-vs-`uint64_t` straddle links fine and only corrupts at call time. Stamped inside
    // the backend body so the marker is part of the content `finalize_hashes` hashes; stamping
    // after `inject_hash_line` would leave every generated Java file permanently stale. ~keep
    hash::inject_stamp_line(
        &out,
        hash::HANDLE_ABI_STAMP_KEY,
        crate::core::template_versions::abi::HANDLE_ABI_VERSION,
    )
}

#[cfg(test)]
mod tests {
    /// Number of required-symbol entries rendered in this test's fixture context.
    /// Kept as a named constant so the assertion below documents its own expected
    /// line count instead of a bare magic number.
    const SYMBOL_COUNT: usize = 3;

    /// `native_lib.jinja`'s `REQUIRED_SYMBOLS` loop separates each entry with
    /// `{% if not loop.last %},{% endif +%}` — the trailing `+` on `endif` is load
    /// bearing: minijinja's `trim_blocks(true)` (set in `template_env::make_env`)
    /// strips the newline that immediately follows *any* block tag, including an
    /// inline `{% endif %}` that closes a same-line conditional. Without `+%}`
    /// every iteration's line ending is eaten and all entries collapse onto one
    /// line, which is exactly what shipped in a consumer's generated
    /// `packages/java/<group-path>/NativeLib.java` and tripped checkstyle's
    /// LineLengthCheck. Render the template directly and
    /// assert one quoted symbol per output line so a regression (dropping the `+`)
    /// fails this test instead of only surfacing downstream in a generated package.
    #[test]
    fn required_symbols_array_renders_one_entry_per_line() {
        // Alphabetical, matching the `BTreeSet<String>` order `gen_native_lib`
        // actually feeds into this template's `required_symbols` context value.
        let required_symbols = vec![
            "sample_extract_free_string".to_string(),
            "sample_last_error_code".to_string(),
            "sample_last_error_context".to_string(),
        ];
        assert_eq!(required_symbols.len(), SYMBOL_COUNT);

        let rendered = crate::backends::java::template_env::render(
            "native_lib.jinja",
            minijinja::context! {
                class_name => "NativeLib",
                lib_name => "sample",
                library_environment_prefix => "SAMPLE",
                prefix => "sample",
                prefix_upper => "SAMPLE",
                required_symbols => required_symbols,
                function_handles => Vec::<String>::new(),
                accessor_handles => Vec::<String>::new(),
                builder_handles => Vec::<String>::new(),
                trait_handles => Vec::<String>::new(),
                visitor_handles => "",
            },
        );

        let array_body: Vec<&str> = rendered
            .lines()
            .skip_while(|line| !line.contains("REQUIRED_SYMBOLS = {"))
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with("};"))
            .collect();

        assert_eq!(
            array_body,
            vec![
                "        \"sample_extract_free_string\",",
                "        \"sample_last_error_code\",",
                "        \"sample_last_error_context\"",
            ],
            "each REQUIRED_SYMBOLS entry must render on its own line; got:\n{rendered}"
        );
    }

    #[test]
    fn native_lib_carries_the_handle_abi_stamp() {
        let output = super::gen_native_lib(
            &crate::core::ir::ApiSurface::default(),
            &crate::core::config::ResolvedCrateConfig::default(),
            "dev.sample",
            "sample",
            false,
        );

        assert!(
            output.contains("ValueLayout.ADDRESS"),
            "fixture must actually emit the hand-specified Panama layout this stamp guards:\n{output}"
        );
        crate::backends::ffi::handle_abi_stamp::assert_stamped_before_hashing(&output, "java NativeLib.java");
    }
}
