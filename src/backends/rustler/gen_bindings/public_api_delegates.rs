use crate::backends::rustler::gen_bindings::helpers::{elixir_return_typespec, elixir_typespec};
use crate::backends::rustler::template_env;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;

/// Build the Elixir behaviour module name for a plugin bridge's typed host surface.
fn behaviour_module(app_module: &str, trait_name: &str) -> String {
    if app_module.is_empty() {
        format!("{trait_name}.Host")
    } else {
        format!("{app_module}.{trait_name}.Host")
    }
}

/// Build the `@callback` typespec rows for a plugin bridge's typed host surface.
///
/// Each trait method becomes one callback. Trait-callback params are delivered to the host as the
/// decoded `{:trait_call, ...}` args map, so a `Named` struct param types as `map()` (its decoded
/// shape) via [`elixir_typespec`]; the return types as the method result via
/// [`elixir_return_typespec`]. This mirrors the host-implementable typed surface the other backends
/// emit (e.g. pyo3's `Protocol`) within Rustler's message-passing / behaviour model.
fn callback_rows(
    methods: &[MethodDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> Vec<minijinja::Value> {
    methods
        .iter()
        .map(|method| {
            let params_spec = method
                .params
                .iter()
                .map(|p| match (&p.ty, p.optional) {
                    (TypeRef::Optional(_), _) | (_, true) => {
                        let base = elixir_typespec(&p.ty, opaque_types, default_types);
                        if base.ends_with("| nil") {
                            base
                        } else {
                            format!("{base} | nil")
                        }
                    }
                    _ => elixir_typespec(&p.ty, opaque_types, default_types),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let return_spec = elixir_return_typespec(
                &method.return_type,
                method.error_type.is_some(),
                opaque_types,
                default_types,
            );
            minijinja::context! {
                method => method.name.clone(),
                name => method.name.to_snake_case(),
                params_spec => params_spec,
                return_spec => return_spec,
            }
        })
        .collect()
}

/// Surface types the trait-bridge delegate emitter needs to type the host behaviour.
pub(in crate::backends::rustler::gen_bindings) struct TraitDelegateCtx<'a> {
    pub api: &'a ApiSurface,
    pub app_module: &'a str,
    pub opaque_types: &'a AHashSet<String>,
    pub default_types: &'a AHashSet<String>,
    /// Top-level API function names, used to avoid double-emitting a `clear_fn` delegate.
    pub api_fn_names: &'a AHashSet<String>,
    pub native_mod: &'a str,
}

pub(in crate::backends::rustler::gen_bindings) fn append_trait_bridge_delegates(
    content: &mut String,
    config: &ResolvedCrateConfig,
    ctx: &TraitDelegateCtx<'_>,
) {
    let &TraitDelegateCtx {
        api,
        app_module,
        opaque_types,
        default_types,
        api_fn_names,
        native_mod,
    } = ctx;
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .iter()
            .any(|language| language == "elixir" || language == "rustler")
        {
            continue;
        }

        let behaviour_mod = behaviour_module(app_module, &bridge_cfg.trait_name);

        // Plugin-pattern bridges (those with a `register_*` function) get a typed,
        // host-implementable behaviour: one `@callback` per trait method with typed params and
        // return. Visitor/options-field bridges (no `register_*`) keep their existing surface.
        if bridge_cfg.register_fn.is_some() {
            if let Some(trait_def) = crate::codegen::generators::trait_bridge::find_trait_def(bridge_cfg, api) {
                if !trait_def.methods.is_empty() {
                    content.push_str(&template_env::render(
                        "elixir_trait_behaviour.ex.jinja",
                        minijinja::context! {
                            behaviour_module => &behaviour_mod,
                            trait_name => &bridge_cfg.trait_name,
                            register_fn => bridge_cfg.register_fn.as_deref().unwrap_or_default().to_snake_case(),
                            callbacks => callback_rows(&trait_def.methods, opaque_types, default_types),
                        },
                    ));
                    content.push('\n');
                }
            }
        }

        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let func_name = register_fn.to_snake_case();
            content.push_str(&template_env::render(
                "elixir_trait_register_delegate.ex.jinja",
                minijinja::context! {
                    trait_name => &bridge_cfg.trait_name,
                    func_name => &func_name,
                    behaviour_module => &behaviour_mod,
                    native_mod => native_mod,
                },
            ));
        }

        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let func_name = unregister_fn.to_snake_case();
            content.push_str(&template_env::render(
                "elixir_trait_unregister_delegate.ex.jinja",
                minijinja::context! {
                    trait_name => &bridge_cfg.trait_name,
                    func_name => &func_name,
                    native_mod => native_mod,
                },
            ));
        }

        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let func_name = clear_fn.to_snake_case();
            if !api_fn_names.contains(func_name.as_str()) {
                content.push_str(&template_env::render(
                    "elixir_trait_clear_delegate.ex.jinja",
                    minijinja::context! {
                        trait_name => &bridge_cfg.trait_name,
                        func_name => &func_name,
                        native_mod => native_mod,
                    },
                ));
            }
        }
    }
}
