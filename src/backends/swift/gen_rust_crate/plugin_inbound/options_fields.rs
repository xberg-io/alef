use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};

/// Emit the `extern "Rust"` declaration and `pub fn` body for the
/// `make_{trait_snake}_handle` factory — the OptionsField analogue of the
/// FunctionParam `register_*` entry point.
///
/// Called from `gen_rust_crate::mod` only when `bridge_cfg.bind_via == OptionsField`.
/// The factory takes a `Swift{Trait}Box`, wraps it in the Rust-side wrapper struct,
/// builds an `Arc<Mutex<...>>` matching the inner type alias path, and returns the
/// local opaque newtype via its existing `From<inner>` impl.
///
/// Returns `(extern_decl, fn_body)`. Both are empty strings when a required field
/// (`type_alias`) is not configured.
pub(crate) fn emit_options_field_factory(
    trait_def: &TypeDef,
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate: &str,
) -> (String, String) {
    debug_assert_eq!(bridge_config.bind_via, BridgeBinding::OptionsField);

    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();
    let box_name = format!("Swift{trait_name}Box");
    let wrapper_name = format!("Swift{trait_name}Wrapper");

    let type_alias = match bridge_config.type_alias.as_deref() {
        Some(a) => a,
        None => return (String::new(), String::new()),
    };

    let alias_def = api.types.iter().find(|t| t.name == type_alias);
    let inner_path = match alias_def {
        Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
        _ => format!("{source_crate}::{type_alias}"),
    };

    let type_alias_snake = heck::AsSnakeCase(type_alias).to_string();
    let fn_name = format!("make_{trait_snake}_{type_alias_snake}");
    let trait_camel = heck::AsUpperCamelCase(trait_name.as_str()).to_string();
    let swift_name = format!("make{trait_camel}Handle");

    let extern_decl = format!(
        "    extern \"Rust\" {{\n\
         \n\
         \x20\x20\x20\x20\x20\x20\x20\x20 #[swift_bridge(swift_name = \"{swift_name}\")]\n\
         \x20\x20\x20\x20\x20\x20\x20\x20 fn {fn_name}(swift_box: {box_name}) -> {type_alias};\n\
         \n\
         \x20\x20\x20\x20}}\n\n"
    );

    let fn_body = format!(
        "/// Construct a `{type_alias}` from a Swift `{box_name}` handle.\n\
         /// Called by Swift e2e tests via `{swift_name}(...)` to build a\n\
         /// `{type_alias}` that can be passed to the options-with-visitor helper.\n\
         pub fn {fn_name}(swift_box: ffi::{box_name}) -> {type_alias} {{\n\
         \x20   let __wrapper = {wrapper_name}::new(swift_box);\n\
         \x20   let __inner: {inner_path} = ::std::sync::Arc::new(::std::sync::Mutex::new(__wrapper));\n\
         \x20   {type_alias}::from(__inner)\n\
         }}\n"
    );

    (extern_decl, fn_body)
}

/// Emit bidirectional `From` impls required by the `OptionsField` factory and options-helper.
///
/// The factory (`make_{trait_snake}_{type_alias_snake}`) calls `{type_alias}::from(__inner)` where
/// `__inner: {inner_path}` — requires `impl From<inner_path> for type_alias`.
///
/// The options-helper (`{options_snake}_from_json_with_{field}`) calls:
/// - `<{inner_path}>::from(h)` where `h: type_alias` — requires `impl From<type_alias> for inner_path`
/// - `{options_type}::from(__core)` where `__core: core_options_path` — requires
///   `impl From<core_options_path> for options_type`
///
/// These are newtype-struct From impls (`.0` field access), not enum match-arm impls.
/// The guard `already_emitted` prevents duplicate emission when multiple bridges share
/// the same type alias or options type.
///
/// Returns an empty string when required config fields are absent.
///
/// Called from `gen_rust_crate::mod` only when `bridge_cfg.bind_via == OptionsField`.
pub(crate) fn emit_options_field_from_impls(
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate: &str,
    already_emitted: &mut std::collections::HashSet<String>,
) -> String {
    debug_assert_eq!(bridge_config.bind_via, BridgeBinding::OptionsField);

    let type_alias = match bridge_config.type_alias.as_deref() {
        Some(a) => a,
        None => return String::new(),
    };
    let options_type = match bridge_config.options_type.as_deref() {
        Some(o) => o,
        None => return String::new(),
    };

    let alias_def = api.types.iter().find(|t| t.name == type_alias);
    let inner_path = match alias_def {
        Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
        _ => format!("{source_crate}::{type_alias}"),
    };

    let opts_def = api.types.iter().find(|t| t.name == options_type);
    let core_options_path = match opts_def {
        Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
        _ => format!("{source_crate}::{options_type}"),
    };

    let mut out = String::new();

    let alias_key = format!("alias::{type_alias}::{inner_path}");
    if !already_emitted.contains(&alias_key) {
        already_emitted.insert(alias_key);
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_bidirectional_newtype_from_impls.rs.jinja",
            minijinja::context! {
                wrapper_type => type_alias,
                inner_type => inner_path,
            },
        ));
    }

    let opts_key = format!("opts::{options_type}::{core_options_path}");
    if !already_emitted.contains(&opts_key) {
        already_emitted.insert(opts_key);
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_newtype_from_impl.rs.jinja",
            minijinja::context! {
                wrapper_type => options_type,
                inner_type => core_options_path,
            },
        ));
    }

    out
}

/// Emit the `extern "Rust"` declaration and `pub fn` body for the
/// `{options_snake}_from_json_with_{field}` helper — mirrors the dart backend's
/// `create_{options_snake}_from_json_with_{field}`.
///
/// Deserialises a core `{OptionsType}` from JSON, attaches the given
/// `Option<{TypeAlias}>` visitor handle to the field, then converts to the mirror
/// struct via its existing `From<core>` impl.
///
/// Returns `(extern_decl, fn_body)`. Both are empty strings when required fields
/// (`type_alias`, `options_type`, or the resolved options field) are not configured.
///
/// Called from `gen_rust_crate::mod` only when `bridge_cfg.bind_via == OptionsField`.
pub(crate) fn emit_options_field_options_helper(
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate: &str,
) -> (String, String) {
    debug_assert_eq!(bridge_config.bind_via, BridgeBinding::OptionsField);

    let type_alias = match bridge_config.type_alias.as_deref() {
        Some(a) => a,
        None => return (String::new(), String::new()),
    };
    let options_type = match bridge_config.options_type.as_deref() {
        Some(o) => o,
        None => return (String::new(), String::new()),
    };
    let field = match bridge_config.resolved_options_field() {
        Some(f) => f.to_string(),
        None => return (String::new(), String::new()),
    };

    let options_snake = heck::AsSnakeCase(options_type).to_string();
    let fn_name = format!("{options_snake}_from_json_with_{field}");
    let swift_name = heck::AsLowerCamelCase(fn_name.as_str()).to_string();

    let alias_def = api.types.iter().find(|t| t.name == type_alias);
    let inner_path = match alias_def {
        Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
        _ => format!("{source_crate}::{type_alias}"),
    };

    let opts_def = api.types.iter().find(|t| t.name == options_type);
    let core_options_path = match opts_def {
        Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
        _ => format!("{source_crate}::{options_type}"),
    };

    let extern_decl = format!(
        concat!(
            "    extern \"Rust\" {{\n",
            "\n",
            "         #[swift_bridge(swift_name = \"{swift_name}\")]\n",
            "         fn {fn_name}(json: String, {field}: Option<{type_alias}>)",
            " -> Result<{options_type}, String>;\n",
            "\n",
            "    }}\n\n"
        ),
        swift_name = swift_name,
        fn_name = fn_name,
        field = field,
        type_alias = type_alias,
        options_type = options_type,
    );

    let fn_body = format!(
        "/// Deserialise a `{options_type}` from JSON and attach a visitor handle to its\n\
         /// `{field}` field. Used by Swift e2e tests to thread a `{type_alias}` into the\n\
         /// conversion call without needing a mutable post-construction setter.\n\
         pub fn {fn_name}(json: String, {field}: Option<{type_alias}>) -> Result<{options_type}, String> {{\n\
         \x20   let mut __core: {core_options_path} = ::serde_json::from_str(&json).map_err(|e| e.to_string())?;\n\
         \x20   __core.{field} = {field}.map(|h| <{inner_path}>::from(h));\n\
         \x20   Ok({options_type}::from(__core))\n\
         }}\n"
    );

    (extern_decl, fn_body)
}
