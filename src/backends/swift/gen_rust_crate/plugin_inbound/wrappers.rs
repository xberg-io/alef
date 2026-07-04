use crate::backends::swift::gen_rust_crate::trait_bridge::SwiftBridgeGenerator;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator as _, TraitBridgeSpec};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};

use super::has_plugin_super;
use super::method_impls::{emit_inbound_method_impl, error_type_path, result_type};

/// Emit the wrapper struct and trait impls for an inbound plugin trait.
///
/// Generates the Send/Sync wrapper newtype, the `Plugin` super-trait impl, the
/// trait impl itself with JSON marshalling, and the `register_*`/`unregister_*` fns.
pub(crate) fn emit_inbound_wrapper(
    trait_def: &TypeDef,
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    error_constructor: &str,
) -> String {
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();
    let box_name = format!("Swift{trait_name}Box");
    let wrapper_name = format!("Swift{trait_name}Wrapper");

    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    let emit_plugin = has_plugin_super(bridge_config);
    let plugin_path = if emit_plugin {
        resolve_plugin_supertrait_path(api, bridge_config)
    } else {
        None
    };
    let mut out = String::new();

    // No module-scope phantom impl for the inbound (Swift-side) trait: the matching
    // `extern "Rust" { fn alef_phantom_vec_swift_{trait}() -> Vec<Swift{Trait}Box>; }`
    // inside the bridge module is enough to force swift-bridge-build to emit the
    // Vec accessor C symbols. `Swift{Trait}Box` is an `extern "Swift" type`, not a
    // Rust-side type, so a module-level `pub fn ... -> Vec<Swift{Trait}Box>` would
    // not compile ("cannot find type … in this scope"). The outbound Rust trait's
    // phantom_impl (in trait_bridge.rs) is unaffected since `{Trait}Box` is a real
    // Rust struct there.

    // 1. Wrapper struct with name cache + Send/Sync.
    // The name_cache field is only needed for Plugin super-trait (which returns &str from name()).
    if emit_plugin {
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_wrapper_struct.rs.jinja",
            minijinja::context! {
                trait_name => trait_name,
                wrapper_name => &wrapper_name,
                box_name => &box_name,
            },
        ));
    } else {
        // Non-Plugin trait: emit a simpler wrapper struct without name_cache.
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_plain_wrapper_struct.rs.jinja",
            minijinja::context! {
                trait_name => trait_name,
                wrapper_name => &wrapper_name,
                box_name => &box_name,
            },
        ));
        // Emit `Debug` when the trait's supertrait list includes it. The opaque swift-bridge
        // handle does not derive Debug, so we write a manual impl that identifies the wrapper
        // by name only — sufficient for trait satisfaction.
        if trait_def
            .super_traits
            .iter()
            .any(|s| s == "Debug" || s.ends_with("::Debug"))
        {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_plain_wrapper_debug.rs.jinja",
                minijinja::context! {
                    wrapper_name => &wrapper_name,
                },
            ));
        }
    }

    // 2. Plugin super-trait impl — only when the trait declares Plugin as a super-trait.
    if emit_plugin {
        if let Some(plugin_path) = plugin_path.as_deref() {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_plugin_impl.rs.jinja",
                minijinja::context! {
                    plugin_path => plugin_path,
                    wrapper_name => &wrapper_name,
                    result_type => result_type(source_crate, error_type, "()"),
                },
            ));
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_plugin_path_compile_error.rs.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                },
            ));
        }
    }
    let _ = trait_snake;

    // 3. Trait impl.
    let has_async = trait_def.methods.iter().any(|m| m.is_async);
    out.push_str(&crate::backends::swift::template_env::render(
        "inbound_trait_impl_open.rs.jinja",
        minijinja::context! {
            has_async => has_async,
            trait_path => &trait_path,
            wrapper_name => &wrapper_name,
        },
    ));
    let lifetime_type_names: std::collections::HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_lifetime_params)
        .map(|t| t.name.clone())
        .collect();
    for method in &trait_def.methods {
        emit_inbound_method_impl(
            &mut out,
            method,
            &trait_snake,
            source_crate,
            type_paths,
            error_type,
            emit_plugin,
            &lifetime_type_names,
            api,
        );
    }
    out.push_str("}\n\n");

    // 4. Registration entry points.
    if let Some(register_fn) = bridge_config.register_fn.as_deref() {
        if let Some(registry_getter) = bridge_config.registry_getter.as_deref() {
            let extra_args = bridge_config
                .register_extra_args
                .as_deref()
                .map(|a| format!(", {a}"))
                .unwrap_or_default();
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_register_fn.rs.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    register_fn => register_fn,
                    box_name => &box_name,
                    trait_path => &trait_path,
                    wrapper_name => &wrapper_name,
                    registry_getter => registry_getter,
                    extra_args => &extra_args,
                },
            ));
        }
    }

    let spec = build_bridge_spec(
        bridge_config,
        trait_def,
        source_crate,
        type_paths,
        error_type,
        error_constructor,
    );
    let generator = SwiftBridgeGenerator;

    let unregister_code = generator.gen_unregistration_fn(&spec);
    if !unregister_code.is_empty() {
        out.push_str(&unregister_code);
        out.push('\n');
    }

    let clear_code = generator.gen_clear_fn(&spec);
    if !clear_code.is_empty() {
        out.push_str(&clear_code);
        out.push('\n');
    }

    out
}

/// Resolve the Plugin super-trait path from explicit config or extracted IR.
///
/// A fully-qualified `super_trait` config value is authoritative. A simple `Plugin`
/// value is resolved from the IR so generated code follows the source crate's actual
/// module layout. Missing IR is intentionally left unresolved instead of assuming a
/// `{source_crate}::plugins::Plugin` convention.
fn resolve_plugin_supertrait_path(api: &ApiSurface, bridge_config: &TraitBridgeConfig) -> Option<String> {
    let super_trait = bridge_config.super_trait.as_deref()?;
    if super_trait.contains("::") {
        return Some(super_trait.replace('-', "_"));
    }

    api.types
        .iter()
        .find(|t| t.is_trait && t.name == super_trait && !t.rust_path.is_empty())
        .map(|t| t.rust_path.replace('-', "_"))
}

/// Build a [`TraitBridgeSpec`] from inbound-wrapper context so the
/// [`SwiftBridgeGenerator`] can be called without duplicating field extraction.
fn build_bridge_spec<'a>(
    bridge_config: &'a TraitBridgeConfig,
    trait_def: &'a TypeDef,
    source_crate: &'a str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    error_constructor: &str,
) -> TraitBridgeSpec<'a> {
    TraitBridgeSpec {
        trait_def,
        bridge_config,
        core_import: source_crate,
        wrapper_prefix: "Swift",
        type_paths: type_paths.clone(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: error_type.to_string(),
        error_constructor: error_constructor.to_string(),
    }
}

/// Emit the shared helper functions used by every inbound wrapper:
///
/// - `plugin_error_from_string` — converts a stringified Swift error into the source crate's
///   configured error type.
/// - `decode_inbound_envelope` — deserialises a JSON envelope (`{"ok": <value>}` /
///   `{"err": "<message>"}`) returned from a fallible Swift trait method into a Rust `Result`.
///
/// We carry fallible results across the FFI as a JSON envelope rather than swift-bridge's
/// native `Result<T, E>` because swift-bridge 0.1.59's `Result<RustString, RustString>`
/// codegen has a bug (`error[E0609]: no field 'ok_or_err' on type '*mut RustString'`).
/// JSON envelopes also gives us a uniform way to ferry typed Ok values without per-method
/// FFI plumbing.
pub(crate) fn emit_plugin_error_helper(source_crate: &str, error_type: &str, error_constructor: &str) -> String {
    let error_type_path = error_type_path(source_crate, error_type);
    let plugin_error_constructor = error_constructor.replace("{msg}", "message");
    crate::backends::swift::template_env::render(
        "plugin_error_helper.rs.jinja",
        minijinja::context! {
            error_type_path => &error_type_path,
            plugin_error_constructor => &plugin_error_constructor,
        },
    )
}
