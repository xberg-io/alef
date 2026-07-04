use crate::backends::rustler::gen_bindings::helpers::{elixir_return_typespec, elixir_typespec};
use crate::backends::rustler::template_env;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;

/// Build the Elixir behaviour module name for a plugin bridge's typed host surface.
///
/// This name is emitted as a *nested* `defmodule` inside `defmodule {AppModule}`, so it
/// must be RELATIVE (`{Trait}.Host`). Qualifying it as `{AppModule}.{Trait}.Host` makes
/// Elixir resolve it relative to the enclosing module, producing the doubly-nested
/// `{AppModule}.{AppModule}.{Trait}.Host` — which also breaks every `{AppModule}.Native.*`
/// reference in the surrounding wrapper bodies.
fn behaviour_module(_app_module: &str, trait_name: &str) -> String {
    format!("{trait_name}.Host")
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
                    // Callback direction: the bridge encodes Named struct params as native
                    // Erlang maps (see the GenServer bridge — "args arrives as a native
                    // Erlang map"). The input-direction convention (configs as optional
                    // JSON strings) does NOT apply here, so default_types structs are
                    // `map()`, not `String.t() | nil`.
                    (TypeRef::Named(name), false) if !opaque_types.contains(name) => "map()".to_string(),
                    (TypeRef::Named(name), true) if !opaque_types.contains(name) => "map() | nil".to_string(),
                    (TypeRef::Optional(inner), _) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n)) => {
                        "map() | nil".to_string()
                    }
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

/// `{name, arity}` pairs for `@optional_callbacks`: the Rust-defaulted trait
/// methods (the bridge forwards them only when the module exports them) plus the
/// `Plugin` lifecycle hooks.
fn optional_callback_rows(methods: &[MethodDef]) -> Vec<minijinja::Value> {
    let mut rows: Vec<minijinja::Value> = methods
        .iter()
        .filter(|m| m.has_default_impl)
        .map(|m| {
            minijinja::context! {
                name => m.name.to_snake_case(),
                arity => m.params.len(),
            }
        })
        .collect();
    rows.push(minijinja::context! { name => "initialize", arity => 0 });
    rows.push(minijinja::context! { name => "shutdown", arity => 0 });
    rows
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
                            optional_callbacks => optional_callback_rows(&trait_def.methods),
                        },
                    ));
                    content.push('\n');
                }
            }
        }

        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let func_name = register_fn.to_snake_case();
            if !api_fn_names.contains(func_name.as_str()) {
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
        }

        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let func_name = unregister_fn.to_snake_case();
            if !api_fn_names.contains(func_name.as_str()) {
                content.push_str(&template_env::render(
                    "elixir_trait_unregister_delegate.ex.jinja",
                    minijinja::context! {
                        trait_name => &bridge_cfg.trait_name,
                        func_name => &func_name,
                        native_mod => native_mod,
                    },
                ));
            }
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

/// Append the visitor receive-loop helper functions when any API function carries a visitor
/// bridge (function-param or options-field mode). No-op otherwise.
pub(in crate::backends::rustler::gen_bindings) fn append_visitor_receive_loop(
    content: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    native_mod: &str,
) {
    // Emit the visitor receive loop helper if any function has a visitor bridge
    // (function_param or options_field mode).
    let has_visitor_bridges = api.functions.iter().any(|func| {
        func.params.iter().any(|p| {
            let named = match &p.ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            config.trait_bridges.iter().any(|b| {
                // function_param: match by param_name or type_alias
                let is_function_param = b.param_name.as_deref() == Some(p.name.as_str())
                    || named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false);
                // options_field: match when the param type is the configured options_type
                let is_options_field = b.bind_via == BridgeBinding::OptionsField
                    && named.is_some_and(|n| b.options_type.as_deref() == Some(n));
                is_function_param || is_options_field
            })
        })
    });

    if has_visitor_bridges {
        let visitor_result_metadata = config.trait_bridges.iter().find_map(|bridge_cfg| {
            match crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg) {
                Ok(metadata) => Some(metadata),
                Err(err) => {
                    eprintln!(
                        "[alef] gen_bindings(rustler): skip visitor helper metadata for trait bridge `{}`: {err}",
                        bridge_cfg.trait_name
                    );
                    None
                }
            }
        });
        if let Some(visitor_result_metadata) = visitor_result_metadata {
            let unit_result_variants = visitor_result_metadata
                .unit_variants
                .iter()
                .map(|variant| {
                    let atom_name = variant
                        .wire_name
                        .chars()
                        .all(|c| c == '_' || c.is_ascii_alphanumeric())
                        .then(|| variant.wire_name.to_snake_case());
                    minijinja::context! {
                        wire_name => variant.wire_name.clone(),
                        atom_name => atom_name,
                    }
                })
                .collect::<Vec<_>>();
            content.push_str(&template_env::render(
                "elixir_visitor_helper_functions.jinja",
                minijinja::context! {
                    native_mod => &native_mod,
                    default_result_wire_name => visitor_result_metadata.default_variant.wire_name,
                    unit_result_variants => unit_result_variants,
                },
            ));
        } else {
            eprintln!(
                "[alef] gen_bindings(rustler): skip visitor helper functions because no configured result enum metadata is available"
            );
        }
    }
}
