use crate::backends::go::c_symbols;
use crate::core::config::TraitBridgeConfig;

/// Generate a config-driven unregistration wrapper.
///
/// Returns an empty string when `bridge_cfg.unregister_fn` is `None`.
/// Also returns empty string if the configured name is just the snake_case version of
/// the standard `Unregister{TraitName}` PascalCase function (to avoid duplicates).
/// Otherwise emits a Go function whose name is `bridge_cfg.unregister_fn`,
/// accepting a `name string` parameter and calling the C-exported
/// `{ffi_prefix}_unregister_{trait_snake}` function via cgo.
pub(super) fn gen_unregistration_fn(bridge_cfg: &TraitBridgeConfig, ffi_prefix: &str, trait_name: &str) -> String {
    let Some(fn_name) = bridge_cfg.unregister_fn.as_deref() else {
        return String::new();
    };
    let standard_pascal_name = format!("Unregister{}", trait_name);
    let standard_snake_name = heck::AsSnakeCase(&standard_pascal_name).to_string();

    if fn_name == standard_snake_name {
        return String::new();
    }

    let c_function = c_symbols::trait_unregister_symbol(ffi_prefix, trait_name);
    let trait_snake = super::helpers::registry_var_stem(trait_name);
    let go_fn_name = super::registration_surface::configured_unregister_fn_name(fn_name);

    let mut out = String::new();
    out.push_str(&crate::backends::go::template_env::render(
        "unregister_fn_header.jinja",
        crate::alef_context! {
            fn_name => &go_fn_name,
            trait_name => trait_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "unregister_c_call.jinja",
        crate::alef_context! {
            c_function => c_function,
            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
            trait_name => trait_name,
            trait_snake => &trait_snake,
        },
    ));
    out.push_str("}\n");
    out
}

/// Generate a config-driven clear-all wrapper.
///
/// Returns an empty string when `bridge_cfg.clear_fn` is `None`.
/// Otherwise emits a Go function whose name is `bridge_cfg.clear_fn`,
/// taking no arguments and calling the C-exported
/// `{ffi_prefix}_clear_{trait_snake}` function via cgo.
pub(super) fn gen_clear_fn(bridge_cfg: &TraitBridgeConfig, ffi_prefix: &str, trait_name: &str) -> String {
    let Some(fn_name) = bridge_cfg.clear_fn.as_deref() else {
        return String::new();
    };
    let trait_snake = super::helpers::registry_var_stem(trait_name);
    let c_function = c_symbols::trait_clear_symbol(ffi_prefix, trait_name);
    let go_fn_name = super::registration_surface::clear_fn_name(fn_name);

    let mut out = String::new();
    out.push_str(&crate::backends::go::template_env::render(
        "clear_function_header.jinja",
        crate::alef_context! {
            fn_name => &go_fn_name,
            name => trait_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "clear_c_call.jinja",
        crate::alef_context! {
            c_function => c_function,
            trait_name => trait_name,
            trait_snake => &trait_snake,
        },
    ));
    out.push_str("}\n");
    out
}
