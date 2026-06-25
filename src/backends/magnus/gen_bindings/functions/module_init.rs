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

    // Custom registrations (before generated ones)
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
        // Variant-wrapper opaque types expose a static `new` singleton method — include
        // them in the `class` binding so `define_singleton_method` can be called on it.
        let has_variant_wrapper_ctor = typ.is_opaque
            && typ.is_variant_wrapper
            && !config.client_constructors.contains_key(&typ.name)
            && typ.methods.iter().any(|m| m.name == "new" && m.receiver.is_none());
        // Opaque types must always be registered even with zero instance methods,
        // so consumer code can reference the class and dynamically attach methods
        // at runtime.
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
            // Always register the constructor as variadic (-1) since the impl now uses a
            // hash-based kwargs constructor regardless of field count. This keeps Ruby
            // callers consistent: every `Type.new(field: ...)` works whether the type has
            // 3 fields or 30.
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
            // Register the static `new` emitted by magnus_variant_wrapper_constructor as
            // a Ruby singleton method. Arity matches the number of params on the `new`
            // MethodDef so Magnus can route positional arguments correctly.
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
                // Skip thread-unsafe fields (e.g., VisitorHandle) that cannot be used in Magnus methods
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
            // Register to_s for structs that have a `content: String` or `content: Option<String>` field.
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
                // Skip apply_update methods: they mutate self without returning a value,
                // which is incompatible with Magnus's method! macro which requires RubyMethod traits.
                // Callers can use from_update instead.
                if method.name == "apply_update" {
                    continue;
                }

                // Skip &mut self methods: Magnus's method! macro doesn't support mutable receivers.
                // These methods mutate the wrapper in place, which isn't compatible with Ruby's
                // object model. Callers should use builder patterns or from_* constructors instead.
                if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
                    continue;
                }

                // Streaming methods register via streaming_method_registrations below.
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

        // Append streaming method registrations (e.g. chat_stream → DefaultClient::chat_stream)
        // for this owner type. These are emitted by the streaming module.
        if let Some(regs) = streaming_method_registrations.get(typ.name.as_str()) {
            for reg in regs {
                lines.push(reg.clone());
            }
        }

        lines.push("".to_string());
    }

    // Register data-enum classes that expose per-variant singleton constructors
    // (`Shape.circle(radius)`). Data enums without qualifying struct variants stay unregistered —
    // they round-trip purely through serde IntoValue/TryConvert and need no Ruby class.
    for enum_def in &api.enums {
        if crate::backends::magnus::gen_bindings::is_reserved_enum(&enum_def.name)
            || exclude_types.contains(enum_def.name.as_str())
        {
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

    // Register iterator classes (e.g. ChatStreamIterator) at module scope.
    if !streaming_iterator_registrations.is_empty() {
        lines.extend(streaming_iterator_registrations.iter().cloned());
        lines.push("".to_string());
    }

    for func in &api.functions {
        if is_reserved_fn(&func.name) || exclude_functions.contains(func.name.as_str()) {
            continue;
        }
        // Skip trait-bridge-managed names (clear_fn) — they get a dedicated
        // registration in the trait_bridge loop below.
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        // Functions with a trait_bridge param use fixed-arity signatures, while
        // options_field bindings use variadic arity. For bridge_param, register fixed arity
        // since those functions don't use scan_args. For options_field, register variadic
        // (-1) since the generated body uses scan_args to unpack arguments.
        let has_bridge_param =
            crate::backends::magnus::trait_bridge::find_bridge_param(func, &config.trait_bridges).is_some();
        let has_options_field_binding =
            crate::backends::magnus::trait_bridge::find_options_field_binding(func, &config.trait_bridges).is_some();

        let is_default_config_func = last_param_is_default_struct(func, api);

        let param_count: i32 = if has_options_field_binding {
            // options_field binding functions use variadic arity with scan_args
            -1
        } else if has_bridge_param {
            // bridge_param functions use fixed arity
            func.params.len() as i32
        } else if needs_variadic_arity(&func.params) || is_default_config_func {
            // Functions with optional params OR default-config last param use variadic arity
            -1
        } else {
            // Functions with only required params use fixed arity
            func.params.len() as i32
        };
        if func.is_async {
            // Register both sync (blocking) and async variants
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &func.name,
                    function_name => &func.name,
                    arity => param_count,
                },
            ));
            let async_name = format!("{}_async", func.name);
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &async_name,
                    function_name => &async_name,
                    arity => param_count,
                },
            ));
        } else {
            lines.push(crate::backends::magnus::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &func.name,
                    function_name => &func.name,
                    arity => param_count,
                },
            ));
        }
    }

    // Register trait bridge entry points: pub fn register_xxx(rb_obj, name) -> Result<...>
    // is emitted by the trait_bridge generator; surface it on the Ruby module here.
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

    // Register module-level wrapper functions for streaming adapters.
    // These allow calling `SampleCrawler.crawl_stream(engine, request)` at module level,
    // mirroring the pattern of non-streaming functions like `crawl`.
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

    // Register error info classes for errors with introspection methods.
    // Each class is defined as a Ruby class on the module and gets define_method
    // calls for status_code, transient?, and error_type.
    for error in &api.errors {
        let regs = crate::codegen::error_gen::magnus_error_methods_registrations(error);
        for reg_line in regs {
            lines.push(reg_line);
        }
    }

    // Register service-API entrypoint functions (generated in `service.rs`).
    // Each entrypoint takes `registrations` (1 param) plus any entrypoint-specific
    // params, so arity = 1 + ep.params.len().
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
