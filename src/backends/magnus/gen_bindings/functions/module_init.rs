use super::scan_args_defaults::{last_param_is_default_struct, needs_variadic_arity};
use crate::backends::magnus::gen_bindings::{classes, is_reserved_fn, streaming};
use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, ReceiverKind};

/// Check if a field contains a bridge handle that cannot be safely passed across thread boundaries.
fn is_thread_unsafe_field(field: &FieldDef, trait_bridges: &[TraitBridgeConfig]) -> bool {
    crate::codegen::generators::trait_bridge::is_bridge_handle_type_ref(&field.ty, trait_bridges)
}

/// Generate the module initialization function.
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::magnus::gen_bindings) fn gen_module_init(
    module_name: &str,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &std::collections::HashSet<&str>,
    exclude_types: &std::collections::HashSet<&str>,
    streaming_methods_by_owner: &std::collections::HashMap<String, Vec<String>>,
    streaming_iterator_registrations: &[String],
    streaming_method_registrations: &std::collections::HashMap<String, Vec<String>>,
    streaming_adapters: &[streaming::StreamingAdapter<'_>],
) -> String {
    let mut lines = vec![
        "#[magnus::init]".to_string(),
        "fn ruby_init(ruby: &Ruby) -> Result<(), Error> {".to_string(),
        crate::backends::magnus::template_env::render(
            "module_define.rs.jinja",
            minijinja::context! {
                module_name => module_name,
            },
        ),
        "".to_string(),
        "    // Ensure JSON library is loaded for Hash#to_json".to_string(),
        "    let _ = ruby.eval::<magnus::Value>(\"require \\\"json\\\"\");".to_string(),
        "".to_string(),
    ];

    if let Some(reg) = config.custom_registrations.for_language(Language::Ruby) {
        for class in &reg.classes {
            lines.push(crate::backends::magnus::template_env::render(
                "module_class_define.rs.jinja",
                minijinja::context! {
                    binding => "_class",
                    class_name => class,
                },
            ));
        }
        for func in &reg.functions {
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => func,
                    function_name => func,
                    arity => 0,
                },
            ));
        }
        lines.push("".to_string());
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if exclude_types.contains(typ.name.as_str()) {
            continue;
        }
        let has_variant_wrapper_ctor = typ.is_opaque
            && typ.is_variant_wrapper
            && !config.client_constructors.contains_key(&typ.name)
            && typ.methods.iter().any(|m| m.name == "new" && m.receiver.is_none());
        let class_used = typ.is_opaque
            || !typ.fields.is_empty()
            || typ.methods.iter().any(|m| !m.is_static)
            || has_variant_wrapper_ctor;
        let binding = if class_used { "class" } else { "_class" };
        lines.push(crate::backends::magnus::template_env::render(
            "module_class_define.rs.jinja",
            minijinja::context! {
                binding => binding,
                class_name => &typ.name,
            },
        ));

        if !typ.is_opaque && !typ.fields.is_empty() {
            lines.push(crate::backends::magnus::template_env::render(
                "module_class_singleton_method_register.rs.jinja",
                minijinja::context! {
                    ruby_name => "new",
                    type_name => &typ.name,
                    function_name => "new",
                    arity => -1,
                },
            ));
        } else if has_variant_wrapper_ctor {
            if let Some(ctor_method) = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none()) {
                let arity = ctor_method.params.len() as i32;
                lines.push(crate::backends::magnus::template_env::render(
                    "module_class_singleton_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => "new",
                        type_name => &typ.name,
                        function_name => "new",
                        arity => arity,
                    },
                ));
            }
        }

        if !typ.is_opaque {
            for field in binding_fields(&typ.fields) {
                if is_thread_unsafe_field(field, &config.trait_bridges) {
                    continue;
                }
                lines.push(crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &field.name,
                        type_name => &typ.name,
                        function_name => &field.name,
                        arity => 0,
                    },
                ));
            }
            if classes::has_content_string_field(typ) {
                lines.push(crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => "to_s",
                        type_name => &typ.name,
                        function_name => "to_s",
                        arity => 0,
                    },
                ));
            }
        }

        let streaming_owner_methods = streaming_methods_by_owner
            .get(typ.name.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        for method in &typ.methods {
            if !method.is_static {
                if method.name == "apply_update" {
                    continue;
                }

                if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
                    continue;
                }

                if streaming_owner_methods.contains(&method.name) {
                    continue;
                }

                let method_name = if method.is_async {
                    format!("{}_async", method.name)
                } else {
                    method.name.clone()
                };
                let param_count = method.params.len();
                lines.push(crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &method_name,
                        type_name => &typ.name,
                        function_name => &method_name,
                        arity => param_count,
                    },
                ));
            }
        }

        if let Some(regs) = streaming_method_registrations.get(typ.name.as_str()) {
            for reg in regs {
                lines.push(reg.clone());
            }
        }

        lines.push("".to_string());
    }

    for enum_def in &api.enums {
        if crate::backends::magnus::gen_bindings::is_reserved_enum(&enum_def.name)
            || exclude_types.contains(enum_def.name.as_str())
        {
            continue;
        }
        if enum_def.serde_tag.is_some() {
            continue;
        }
        let registrations = classes::data_enum_variant_constructor_registrations(enum_def);
        if registrations.is_empty() {
            continue;
        }
        lines.push(crate::backends::magnus::template_env::render(
            "module_class_define.rs.jinja",
            minijinja::context! {
                binding => "class",
                class_name => &enum_def.name,
            },
        ));
        for (ruby_name, function_name, arity) in &registrations {
            lines.push(crate::backends::magnus::template_env::render(
                "module_class_singleton_method_register.rs.jinja",
                minijinja::context! {
                    ruby_name => ruby_name,
                    type_name => &enum_def.name,
                    function_name => function_name,
                    arity => arity,
                },
            ));
        }
        lines.push("".to_string());
    }

    if !streaming_iterator_registrations.is_empty() {
        lines.extend(streaming_iterator_registrations.iter().cloned());
        lines.push("".to_string());
    }

    for func in &api.functions {
        if is_reserved_fn(&func.name) || exclude_functions.contains(func.name.as_str()) {
            continue;
        }
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        let has_bridge_param =
            crate::backends::magnus::trait_bridge::find_bridge_param(func, &config.trait_bridges).is_some();
        let has_options_field_binding =
            crate::backends::magnus::trait_bridge::find_options_field_binding(func, &config.trait_bridges).is_some();

        let is_default_config_func = last_param_is_default_struct(func, api);

        let param_count: i32 = if has_options_field_binding {
            -1
        } else if has_bridge_param {
            func.params.len() as i32
        } else if needs_variadic_arity(&func.params) || is_default_config_func {
            -1
        } else {
            func.params.len() as i32
        };
        let ruby_name = crate::backends::magnus::ruby_public_function_name(func);
        let function_name = crate::backends::magnus::ruby_native_function_name(func);
        lines.push(crate::backends::magnus::template_env::render(
            "module_function_register.rs.jinja",
            minijinja::context! {
                ruby_name => ruby_name,
                function_name => function_name.as_ref(),
                arity => param_count,
            },
        ));
    }

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|s| s == "ruby") {
            continue;
        }
        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => register_fn,
                    function_name => register_fn,
                    arity => 2,
                },
            ));
        }
        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => unregister_fn,
                    function_name => unregister_fn,
                    arity => 1,
                },
            ));
        }
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => clear_fn,
                    function_name => clear_fn,
                    arity => 0,
                },
            ));
        }
    }

    for adapter in streaming_adapters {
        lines.push(crate::backends::magnus::template_env::render(
            "module_function_register.rs.jinja",
            minijinja::context! {
                ruby_name => adapter.name,
                function_name => adapter.name,
                arity => 2,
            },
        ));
    }

    for error in &api.errors {
        let regs = crate::codegen::error_gen::magnus_error_methods_registrations(error);
        for reg_line in regs {
            lines.push(reg_line);
        }
    }

    if !api.services.is_empty() {
        use heck::ToSnakeCase as _;
        lines.push("    // Service entrypoints".to_string());
        for service in &api.services {
            let service_snake = service.name.to_snake_case();
            for ep in &service.entrypoints {
                let fn_name = format!("{service_snake}_{}", ep.method);
                let arity = 1 + ep.params.len() as i32;
                lines.push(crate::backends::magnus::template_env::render(
                    "module_function_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &fn_name,
                        function_name => format!("service::{fn_name}"),
                        arity => arity,
                    },
                ));
            }
        }
        lines.push("".to_string());
    }

    lines.push("".to_string());
    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}
