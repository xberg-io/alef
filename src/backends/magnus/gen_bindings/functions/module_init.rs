use super::scan_args_defaults::{last_param_is_default_struct, needs_variadic_arity};
use crate::backends::magnus::gen_bindings::{classes, is_reserved_fn, streaming};
use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, ReceiverKind};

/// Check if a field contains a bridge handle that cannot be safely passed across thread boundaries.
fn is_thread_unsafe_field(field: &FieldDef, trait_bridges: &[TraitBridgeConfig]) -> bool {
    crate::codegen::generators::trait_bridge::is_bridge_handle_type_ref(&field.ty, trait_bridges)
}

/// Prefix a `ruby_init` statement with `#[cfg(<gate>)]` when `gate` is non-empty.
///
/// The single mechanism every reference-emission site in this function shares: a class's own
/// singleton constructor registration, its field-accessor and `to_s` registrations, and its
/// per-method registrations all name a `{Type}::{function}` path that may not exist once the
/// type (or the member itself) is compiled out by its own `#[cfg(...)]` -- `method!`/`function!`
/// resolve that path at compile time, so an ungated registration is a hard `E0433`/`E0425`, not a
/// missing Ruby method. `#[magnus::init]`'s body is a flat statement list (not a set of items),
/// so `#[cfg]` on the individual statement is the only place the gate can go. ~keep
fn gate_statement(cfg: Option<&str>, statement: String) -> String {
    match cfg {
        Some(gate) if !gate.is_empty() => format!("    #[cfg({gate})]\n{statement}"),
        _ => statement,
    }
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
    let core_import = config.core_import_name();
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Ruby);
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

        // Every registration below names `{Type}::{member}` as a path (`function!`/`method!`
        // resolve it at compile time), so each one needs the SAME gate `typ.cfg` already applies
        // to the type's own declaration (`classes::gen_struct`/`gen_opaque_struct`) -- an
        // ungated registration for a type `#[cfg]` compiled out is a hard `E0433`/`E0425` here,
        // not a missing Ruby method. ~keep
        let typ_cfg = typ.cfg.as_deref();
        if !typ.is_opaque && !typ.fields.is_empty() {
            let registration = crate::backends::magnus::template_env::render(
                "module_class_singleton_method_register.rs.jinja",
                minijinja::context! {
                    ruby_name => "new",
                    type_name => &typ.name,
                    function_name => "new",
                    arity => -1,
                },
            );
            lines.push(gate_statement(typ_cfg, registration));
        } else if has_variant_wrapper_ctor
            && let Some(ctor_method) = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())
        {
            let arity = ctor_method.params.len() as i32;
            let registration = crate::backends::magnus::template_env::render(
                "module_class_singleton_method_register.rs.jinja",
                minijinja::context! {
                    ruby_name => "new",
                    type_name => &typ.name,
                    function_name => "new",
                    arity => arity,
                },
            );
            lines.push(gate_statement(ctor_method.cfg_within(typ_cfg).as_deref(), registration));
        }

        let mut registered_field_names: ahash::AHashSet<&str> = ahash::AHashSet::default();
        if !typ.is_opaque {
            for field in binding_fields(&typ.fields) {
                if is_thread_unsafe_field(field, &config.trait_bridges) {
                    continue;
                }
                registered_field_names.insert(field.name.as_str());
                let registration = crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &field.name,
                        type_name => &typ.name,
                        function_name => &field.name,
                        arity => 0,
                    },
                );
                // Combines the type's own gate with the field's own gate (e.g. an `Option<T>`
                // field whose `T` is independently cfg-gated) -- the same combination
                // `classes::gen_struct_methods` now applies to this accessor's `fn` definition.
                // ~keep
                lines.push(gate_statement(field.cfg_within(typ_cfg).as_deref(), registration));
            }
            if classes::has_content_string_field(typ) {
                let registration = crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => "to_s",
                        type_name => &typ.name,
                        function_name => "to_s",
                        arity => 0,
                    },
                );
                lines.push(gate_statement(typ_cfg, registration));
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

                if matches!(method.receiver, Some(ReceiverKind::RefMut)) && !typ.is_opaque {
                    continue;
                }

                if streaming_owner_methods.contains(&method.name) {
                    continue;
                }

                // The field accessor with this name was registered just above and the
                // same-named `fn` is dropped from the impl block (see `gen_struct_methods`),
                // so registering the method here would reference a non-existent function. ~keep
                if !method.is_async && registered_field_names.contains(method.name.as_str()) {
                    continue;
                }

                let method_name = if method.is_async {
                    format!("{}_async", method.name)
                } else {
                    method.name.clone()
                };
                let param_count = method.params.len();
                let registration = crate::backends::magnus::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &method_name,
                        type_name => &typ.name,
                        function_name => &method_name,
                        arity => param_count,
                    },
                );
                // The registration must carry the same gate the wrapper `fn` got in
                // `classes::gen_opaque_instance_method`/`gen_instance_method`: `method!` resolves
                // `{type}::{method}` as a path, so registering a method the gate compiled out is a
                // hard E0599, not a missing Ruby method. Combined with `typ_cfg` via `cfg_within`
                // -- a method's own `cfg` only ever carries its `impl` block's gate, never the
                // owning type's, so a type-level-only gate (no additional method-level `#[cfg]`)
                // left this registration ungated even though `{type_name}::{method_name}` no
                // longer exists once `typ_cfg` compiles the type out. ~keep
                lines.push(gate_statement(method.cfg_within(typ_cfg).as_deref(), registration));
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
        let registrations = classes::data_enum_variant_constructor_registrations(
            enum_def,
            &core_import,
            Some(enabled_features.as_slice()),
        );
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
        let registration = crate::backends::magnus::template_env::render(
            "module_function_register.rs.jinja",
            minijinja::context! {
                ruby_name => ruby_name,
                function_name => function_name.as_ref(),
                arity => param_count,
            },
        );
        // The registration must carry the same gate `prepend_cfg` already applied to the `fn`
        // this `function!(...)` call names: `#[magnus::init]`'s body is a flat statement list
        // (not a set of items), so `#[cfg]` on the registration statement is the only place the
        // gate can go, exactly as the method loop above does via `method.cfg`. Without it, a
        // definition compiled out by its own `#[cfg(feature = "X")]` leaves this line naming a
        // function that does not exist -- a hard `E0425 cannot find value` here, not a missing
        // Ruby method. ~keep
        lines.push(match func.cfg.as_deref() {
            Some(gate) if !gate.is_empty() => {
                crate::backends::magnus::template_env::render(
                    "cfg_attribute.rs.jinja",
                    minijinja::context! { predicate => gate },
                ) + &registration
            }
            _ => registration,
        });
    }

    for bridge_cfg in &config.trait_bridges {
        if crate::backends::magnus::trait_bridge::active_bridge_trait(bridge_cfg, api).is_none() {
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
