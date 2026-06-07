use crate::core::config::TraitBridgeConfig;
use heck::ToPascalCase;

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
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let standard_pascal_name = format!("Unregister{}", trait_name);
    let standard_snake_name = heck::AsSnakeCase(&standard_pascal_name).to_string();

    // Skip if the configured name is the snake_case version of the standard Unregister{TraitName}
    // Go convention is PascalCase only; the PascalCase version is always emitted, so don't duplicate
    if fn_name == standard_snake_name {
        return String::new();
    }

    let c_function = format!("{}_unregister_{}", ffi_prefix, trait_snake);
    // Convert fn_name to PascalCase for Go (e.g., "unregister_text_backend" → "UnregisterTextBackend")
    let go_fn_name = fn_name.to_pascal_case();

    let mut out = String::new();
    out.push_str(&crate::backends::go::template_env::render(
        "unregister_fn_header.jinja",
        minijinja::context! {
            fn_name => &go_fn_name,
            trait_name => trait_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "unregister_c_call.jinja",
        minijinja::context! {
            c_function => c_function,
            ffi_prefix => ffi_prefix,
            trait_name => trait_name,
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
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let c_function = format!("{}_clear_{}", ffi_prefix, trait_snake);
    // Convert fn_name to PascalCase for Go (e.g., "clear_text_backends" → "ClearTextBackends")
    let go_fn_name = fn_name.to_pascal_case();

    let mut out = String::new();
    out.push_str(&crate::backends::go::template_env::render(
        "clear_function_header.jinja",
        minijinja::context! {
            fn_name => &go_fn_name,
            name => trait_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "clear_c_call.jinja",
        minijinja::context! {
            c_function => c_function,
            trait_name => trait_name,
            trait_snake => &trait_snake,
        },
    ));
    out.push_str("}\n");
    out
}
