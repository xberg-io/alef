use super::super::StreamingMethodMeta;
use super::adapters::{gen_adapter_wrapper, gen_opaque_streaming_static_wrapper};
use super::bridge_fields::gen_bridge_field_wrapper_function;
use super::wrappers::{gen_wrapper_function, gen_wrapper_method};
use crate::codegen::generators::trait_bridge::find_bridge_field;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::{AdapterConfig, HostCapsuleTypeConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// Check if a function returns a capsule type (Language passthrough).
fn is_capsule_function(func: &FunctionDef, capsule_types: &HashMap<String, HostCapsuleTypeConfig>) -> bool {
    match &func.return_type {
        TypeRef::Named(name) => capsule_types.contains_key(name),
        _ => false,
    }
}

/// Get the capsule config for a function's return type, if it is a capsule.
fn get_capsule_config<'a>(
    func: &FunctionDef,
    capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
) -> Option<&'a HostCapsuleTypeConfig> {
    match &func.return_type {
        TypeRef::Named(name) => capsule_types.get(name),
        _ => None,
    }
}

/// Skip methods that take opaque handle FFI pointers as first arg but operate on non-opaque types.
/// These are validation/property functions that shouldn't be exposed as static methods.
/// Examples: header_metadata_is_valid, conversion_options_default (Rust naming, snake_case
/// as stored in FunctionDef.name).
fn should_skip_ffi_method(func: &FunctionDef) -> bool {
    let name = &func.name;

    if name.ends_with("_is_valid") || name == "is_valid" {
        return true;
    }

    if name.ends_with("_default") || name == "default" {
        return true;
    }

    false
}

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::csharp::gen_bindings) fn gen_wrapper_class(
    api: &ApiSurface,
    namespace: &str,
    class_name: &str,
    exception_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
    streaming_methods: &HashSet<String>,
    _streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    exclude_functions: &HashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    _all_opaque_type_names: &HashSet<String>,
    adapters: &[AdapterConfig],
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().flat_map(|t| t.methods.iter()).any(|m| m.is_async);

    let mut out = render(
        "wrapper_class_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "has_async": has_async,
        })),
    );
    out.push('\n');

    let enum_names: HashSet<String> = api.enums.iter().map(|e| csharp_type_name(&e.name)).collect();

    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let handle_returned_types = super::super::errors::compute_handle_returned_types(api);

    for func in api.functions.iter().filter(|f| {
        !exclude_functions.contains(&f.name)
            && !should_skip_ffi_method(f)
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, trait_bridges)
    }) {
        let bridge_field = find_bridge_field(func, &api.types, trait_bridges);
        if let Some(bm) = bridge_field {
            out.push_str(&gen_bridge_field_wrapper_function(
                func,
                &bm,
                exception_name,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
            ));
        } else if is_capsule_function(func, capsule_types) {
            if let Some(cfg) = get_capsule_config(func, capsule_types) {
                out.push_str(&super::wrappers::gen_capsule_function_wrapper(
                    func,
                    exception_name,
                    prefix,
                    cfg,
                ));
            }
        } else {
            out.push_str(&gen_wrapper_function(
                func,
                exception_name,
                prefix,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
                bridge_param_names,
                bridge_type_aliases,
                has_visitor_callbacks,
                &api.types,
            ));
        }
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque {
            continue;
        }
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
            if method.name == "is_valid" || method.name == "default" {
                continue;
            }
            out.push_str(&gen_wrapper_method(
                method,
                exception_name,
                prefix,
                &typ.name,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
                bridge_param_names,
                bridge_type_aliases,
                &api.types,
            ));
        }
    }

    for typ in api.types.iter().filter(|typ| typ.is_opaque) {
        for method in &typ.methods {
            if !streaming_methods.contains(&method.name) {
                continue;
            }
            if let Some(meta) = _streaming_methods_meta.get(&method.name) {
                out.push_str(&gen_opaque_streaming_static_wrapper(
                    method,
                    &typ.name,
                    meta,
                    exception_name,
                ));
            }
        }
    }

    for adapter in adapters {
        if matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
            out.push_str(&gen_adapter_wrapper(adapter, prefix, exception_name, api));
        }
    }

    for bridge_cfg in trait_bridges {
        let trait_pascal = csharp_type_name(&bridge_cfg.trait_name);
        let has_super = bridge_cfg.super_trait.is_some();

        let register_method_name = format!("Register{trait_pascal}");
        out.push_str(&render(
            "trait_register_facade.jinja",
            minijinja::context! {
                trait_name => trait_pascal,
                method_name => register_method_name,
                has_super,
                exception_name,
            },
        ));

        if bridge_cfg.unregister_fn.is_some() {
            let unregister_method_name = format!("Unregister{trait_pascal}");
            out.push_str(&render(
                "trait_unregister_facade.jinja",
                minijinja::context! {
                    trait_name => trait_pascal,
                    method_name => unregister_method_name,
                    exception_name,
                },
            ));
        }
    }

    for bridge_cfg in trait_bridges {
        if let Some(clear_fn) = &bridge_cfg.clear_fn {
            let trait_pascal = csharp_type_name(&bridge_cfg.trait_name);
            let clear_method_name = to_csharp_name(clear_fn);

            out.push_str(&render(
                "trait_clear_facade.jinja",
                minijinja::context! {
                    trait_name => trait_pascal,
                    method_name => clear_method_name,
                },
            ));
        }
    }

    let has_base_error = !api.errors.is_empty();
    let (base_exception_class, has_invalid_input_variant, variant_dispatch_lines) = if has_base_error {
        let base_error = &api.errors[0];
        let base_ex = format!("{}Exception", base_error.name);
        let has_invalid = base_error.variants.iter().any(|v| v.name == "InvalidInput");
        let mut variants_with_prefix: Vec<(String, String)> = base_error
            .variants
            .iter()
            .filter(|v| v.name != "InvalidInput")
            .filter_map(|v| {
                let template = v.message_template.as_deref()?;
                let prefix_end = template.find('{').unwrap_or(template.len());
                let prefix = template[..prefix_end].trim_end().to_string();
                if prefix.is_empty() {
                    return None;
                }
                Some((format!("{}Exception", v.name), prefix))
            })
            .collect();
        variants_with_prefix.sort_by_key(|item| std::cmp::Reverse(item.1.len()));
        let dispatch_lines: Vec<String> = variants_with_prefix
            .into_iter()
            .map(|(class, prefix)| {
                let escaped_prefix = prefix.replace('\\', "\\\\").replace('"', "\\\"");
                format!("        if (message.StartsWith(\"{escaped_prefix}\")) return new {class}(message);")
            })
            .collect();
        (base_ex, has_invalid, dispatch_lines)
    } else {
        (String::new(), false, Vec::new())
    };

    out.push_str(&render(
        "error_helper_method.jinja",
        Value::from_serialize(serde_json::json!({
            "exception_name": exception_name,
            "has_base_error": has_base_error,
            "base_exception_class": base_exception_class,
            "has_invalid_input_variant": has_invalid_input_variant,
            "variant_dispatch_lines": variant_dispatch_lines,
        })),
    ));

    out.push_str("}\n");

    out
}
