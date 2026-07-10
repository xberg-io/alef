use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{TypeDef, TypeRef};
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
    let trait_name = &bridge_cfg.trait_name;
    let trait_snake = gleam_public_member_name(trait_name);

    out.push_str(&crate::backends::gleam::template_env::render(
        "trait_bridge_doc_header.jinja",
        minijinja::context! {
            trait_name => trait_name,
        },
    ));
    if let Some(ty) = trait_type {
        if !ty.doc.is_empty() {
            out.push_str(&crate::backends::gleam::template_env::render(
                "trait_type_doc_lines.jinja",
                minijinja::context! {
                    doc_lines => ty.doc.lines().collect::<Vec<_>>(),
                },
            ));
            out.push_str(&crate::backends::gleam::template_env::render(
                "trait_bridge_empty_comment_line.jinja",
                minijinja::context! {},
            ));
        }
    }
    out.push_str(&crate::backends::gleam::template_env::render(
        "trait_scope_cap.jinja",
        minijinja::context! {},
    ));

    if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
        imports.insert("import gleam/dynamic.{type Dynamic}");
        out.push_str(&crate::backends::gleam::template_env::render(
            "register_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                register_fn => register_fn,
                trait_snake => &trait_snake,
            },
        ));
        out.push('\n');
    }

    if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
        out.push_str(&crate::backends::gleam::template_env::render(
            "unregister_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                unregister_fn => unregister_fn,
            },
        ));
        out.push('\n');
    }

    if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
        out.push_str(&crate::backends::gleam::template_env::render(
            "clear_fn.jinja",
            minijinja::context! {
                nif_module => nif_module,
                clear_fn => clear_fn,
            },
        ));
        out.push('\n');
    }

    if let Some(trait_ty) = trait_type {
        for method in &trait_ty.methods {
            let method_snake = gleam_public_member_name(&method.name);
            let nif_fn_name = format!("{trait_snake}_{method_snake}_response");

            let ok_type = match &method.return_type {
                TypeRef::Unit => "Nil".to_string(),
                other => {
                    let substituted = substitute_invisible_named(other, visible_type_names);
                    gleam_type(&substituted, false, imports)
                }
            };

            let err_type = method
                .error_type
                .as_deref()
                .map(|e| resolve_gleam_error_type(e, declared_errors))
                .unwrap_or_else(|| "String".to_string());

            out.push_str(&crate::backends::gleam::template_env::render(
                "method_doc_header.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                },
            ));
            out.push_str(&crate::backends::gleam::template_env::render(
                "method_doc_usage.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                    nif_fn_name => &nif_fn_name,
                },
            ));

            imports.insert("import gleam/dynamic.{type Dynamic}");
            out.push_str(&crate::backends::gleam::template_env::render(
                "method_external.jinja",
                minijinja::context! {
                    nif_module => nif_module,
                    nif_fn_name => &nif_fn_name,
                },
            ));
            out.push_str(&crate::backends::gleam::template_env::render(
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

fn gleam_public_member_name(name: &str) -> String {
    crate::codegen::naming::public_host_identifier(
        crate::core::config::Language::Gleam,
        crate::codegen::naming::PublicIdentifierKind::Function,
        name,
    )
}

/// Emit the shared `complete_trait_call` and `fail_trait_call` support NIF shims.
///
/// These are emitted once per module regardless of how many bridges are active,
/// because the Rustler side registers them as module-level NIFs used by all bridges.
pub(crate) fn emit_trait_support_nifs(nif_module: &str, out: &mut String) {
    out.push_str(&crate::backends::gleam::template_env::render(
        "support_nif_doc.jinja",
        minijinja::context! {},
    ));
    out.push('\n');
    out.push_str(&crate::backends::gleam::template_env::render(
        "support_nif_complete.jinja",
        minijinja::context! {
            nif_module => nif_module,
        },
    ));
    out.push('\n');
    out.push('\n');

    out.push_str(&crate::backends::gleam::template_env::render(
        "support_nif_fail_doc.jinja",
        minijinja::context! {},
    ));
    out.push_str(&crate::backends::gleam::template_env::render(
        "support_nif_fail.jinja",
        minijinja::context! {
            nif_module => nif_module,
        },
    ));
    out.push('\n');
}
