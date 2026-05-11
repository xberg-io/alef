use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{TypeDef, TypeRef};
use std::collections::{BTreeSet, HashSet};

use super::nif_external::{gleam_type, resolve_gleam_error_type};

/// Recursively substitute `TypeRef::Named` nodes whose name is not in
/// `visible_type_names` with `TypeRef::String`. Used to prevent excluded
/// internal types (e.g. `InternalDocument`) from leaking into generated
/// public Gleam type signatures and docstrings.
fn substitute_invisible_named(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if !visible_type_names.contains(name.as_str()) => TypeRef::String,
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_invisible_named(inner, visible_type_names))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_invisible_named(inner, visible_type_names))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(substitute_invisible_named(k, visible_type_names)),
            Box::new(substitute_invisible_named(v, visible_type_names)),
        ),
        other => other.clone(),
    }
}

/// Emit Gleam shim functions for a single trait bridge.
///
/// Emits:
/// - A documentation comment explaining the trait bridge and scope cap.
/// - A `register_<trait_snake>` pub fn calling the Rustler NIF registration function
///   (when `register_fn` is configured).
///
/// Scope cap: real callback round-trips require the Gleam/Elixir module to implement
/// a GenServer `handle_info/2` that responds to `{:trait_call, method, args_json, reply_id}`
/// messages and calls `complete_trait_call/2` or `fail_trait_call/2` when done.
/// Gleam emits these function shims; users wire their callback module via the existing
/// Elixir/Rustler GenServer registration pattern.
pub(crate) fn emit_trait_bridge_shims(
    bridge_cfg: &TraitBridgeConfig,
    trait_type: Option<&TypeDef>,
    nif_module: &str,
    declared_errors: &[String],
    visible_type_names: &HashSet<&str>,
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    use heck::ToSnakeCase;

    let trait_name = &bridge_cfg.trait_name;
    let trait_snake = trait_name.to_snake_case();

    // Documentation comment
    out.push_str(&crate::template_env::render(
        "trait_bridge_doc_header.jinja",
        minijinja::context! {
            trait_name => trait_name,
        },
    ));
    if let Some(ty) = trait_type {
        if !ty.doc.is_empty() {
            for line in ty.doc.lines() {
                out.push_str("/// ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("///\n");
        }
    }
    out.push_str(
        "/// # Scope cap\n\
         ///\n\
         /// Real callback round-trips require the caller to register a GenServer PID\n\
         /// that implements `handle_info/2` to handle\n\
         /// `{:trait_call, method, args_json, reply_id}` messages and then calls\n\
         /// `complete_trait_call` or `fail_trait_call` with the reply.\n\
         /// Gleam emits the registration and reply shims here; wiring the callback\n\
         /// module is done via the Elixir/Rustler side (existing GenServer pattern).\n",
    );

    // Registration function — only when register_fn is configured.
    // The PID is passed as Dynamic because Gleam's type system does not have a native
    // Pid type; Dynamic lets callers pass the Erlang PID term directly.
    if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
        imports.insert("import gleam/dynamic.{type Dynamic}");
        out.push_str(&crate::template_env::render(
            "register_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                register_fn => register_fn,
                trait_snake => &trait_snake,
            },
        ));
        out.push('\n');
    }

    // Unregistration function — only when unregister_fn is configured.
    // Takes a `name: String` identifying the plugin to remove and returns
    // `Result(Nil, String)` so callers can handle unknown-name errors.
    if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
        out.push_str(&crate::template_env::render(
            "unregister_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                unregister_fn => unregister_fn,
            },
        ));
        out.push('\n');
    }

    // Clear function — only when clear_fn is configured.
    // Takes no arguments and returns `Result(Nil, String)`.
    // Typically used in test teardown to remove all registered plugins.
    if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
        out.push_str(&crate::template_env::render(
            "clear_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                clear_fn => clear_fn,
            },
        ));
        out.push('\n');
    }

    // Per-method response shims.
    //
    // For every method defined on the trait, emit a typed helper that the consumer's
    // callback module calls to send the result back through the Rustler reply-registry.
    // The NIF name follows the convention: `{trait_snake}_{method_snake}_response`.
    //
    // `call_id` is Dynamic because Gleam has no native Erlang reference type;
    // callers pass the opaque reference term received in the trait_call message.
    if let Some(trait_ty) = trait_type {
        for method in &trait_ty.methods {
            let method_snake = method.name.to_snake_case();
            let nif_fn_name = format!("{trait_snake}_{method_snake}_response");

            // Build Gleam return type for the ok branch (Nil when Unit).
            // Excluded/internal types (e.g. `InternalDocument`) are not represented as
            // generated Gleam types — substitute them with `String` so the signature
            // does not reference a non-existent symbol.
            let ok_type = match &method.return_type {
                TypeRef::Unit => "Nil".to_string(),
                other => {
                    let substituted = substitute_invisible_named(other, visible_type_names);
                    gleam_type(&substituted, false, imports)
                }
            };

            // Build Gleam error type: resolve via declared errors list so that
            // external types like `anyhow::Error` fall back to the module's own
            // error type (or String when no errors are declared).
            let err_type = method
                .error_type
                .as_deref()
                .map(|e| resolve_gleam_error_type(e, declared_errors))
                .unwrap_or_else(|| "String".to_string());

            // Doc comment with usage guidance.
            out.push_str(&crate::template_env::render(
                "method_doc_header.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                },
            ));
            out.push_str(&crate::template_env::render(
                "method_doc_usage.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                    nif_fn_name => &nif_fn_name,
                },
            ));

            imports.insert("import gleam/dynamic.{type Dynamic}");
            out.push_str(&crate::template_env::render(
                "method_external.jinja",
                minijinja::context! {
                    nif_module => nif_module,
                    nif_fn_name => &nif_fn_name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "method_signature.jinja",
                minijinja::context! {
                    nif_fn_name => &nif_fn_name,
                    ok_type => &ok_type,
                    err_type => &err_type,
                },
            ));
            out.push('\n');
        }
    }
}

/// Emit the shared `complete_trait_call` and `fail_trait_call` support NIF shims.
///
/// These are emitted once per module regardless of how many bridges are active,
/// because the Rustler side registers them as module-level NIFs used by all bridges.
pub(crate) fn emit_trait_support_nifs(nif_module: &str, out: &mut String) {
    out.push_str(&crate::template_env::render(
        "support_nif_doc.jinja",
        minijinja::context! {},
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "support_nif_complete.jinja",
        minijinja::context! {
            nif_module => nif_module,
        },
    ));
    out.push('\n');
    out.push('\n');

    out.push_str("/// Fail a pending trait call with an error message.\n");
    out.push_str("/// Call this from your GenServer when processing a trait_call message fails.\n");
    out.push_str(&crate::template_env::render(
        "support_nif_fail.jinja",
        minijinja::context! {
            nif_module => nif_module,
        },
    ));
    out.push('\n');
}
