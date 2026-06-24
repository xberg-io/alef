use super::native_args::build_native_args;
use crate::codegen::generators::trait_bridge::{
    bridge_param_type as param_type, native_marshalled_struct_params, visitor_param_type,
};
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashSet;

/// Parameters for [`gen_visitor_bridge`], grouped to keep argument count under the lint limit.
pub(super) struct VisitorBridgeCtx<'a> {
    pub(super) trait_type: &'a TypeDef,
    pub(super) struct_name: &'a str,
    pub(super) trait_path: &'a str,
    pub(super) core_crate: &'a str,
    pub(super) type_paths: &'a std::collections::HashMap<String, String>,
    pub(super) bridge_cfg: &'a crate::core::config::TraitBridgeConfig,
    pub(super) api: &'a ApiSurface,
}

/// Generate a visitor-style bridge wrapping a `rustler::OwnedEnv` + `rustler::Term`.
///
/// This generates an async message-passing bridge. When `convert_with_visitor` is called,
/// it spawns a system thread that runs the conversion. Each visitor callback sends a
/// `{:visitor_callback, ref_id, callback_name, args_map}` message to the calling Elixir
/// process — `args_map` is a NATIVE Erlang term map (built inside `send_and_clear`), not a
/// JSON string — and blocks on a channel waiting for the reply from `visitor_reply/2`.
/// When conversion finishes, the thread sends `{:ok, result_json}` or `{:error, reason}`
/// to the caller.
pub(super) fn gen_visitor_bridge(out: &mut String, ctx: &VisitorBridgeCtx<'_>) -> anyhow::Result<()> {
    let VisitorBridgeCtx {
        trait_type,
        struct_name,
        trait_path,
        core_crate,
        type_paths,
        bridge_cfg,
        api,
    } = ctx;
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Rustler,
    )?;
    // Helper: convert configured visitor context to a Rustler NifMap term inside an OwnedEnv
    let ctx_helper = minijinja::context! {
        context_type_path => context_helper.type_path,
        context_field_lines => context_helper.field_lines,
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_bridge_helper.rs.jinja",
        ctx_helper,
    ));

    // Global channel registry: maps ref_id -> SyncSender so visitor_reply can unblock the bridge.
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_bridge_globals.rs.jinja",
        minijinja::context! {},
    ));

    // Bridge struct: holds the caller PID and the visitor term in its OwnedEnv.
    // Both OwnedEnv and SavedTerm are Send, so the bridge can be moved to a system thread.
    let ctx_struct = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_bridge_struct.rs.jinja",
        ctx_struct,
    ));

    // Manual Debug impl for visitor traits that require std::fmt::Debug.
    let ctx_debug = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_bridge_debug.rs.jinja",
        ctx_debug,
    ));

    // Constructor (called from BEAM thread — saves visitor term into an OwnedEnv)
    let ctx_constructors = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_bridge_constructors.rs.jinja",
        ctx_constructors,
    ));

    // Helper: send a visitor callback message and block waiting for the reply.
    // Encoded as: {:visitor_callback, ref_id, callback_atom, args_map} where args_map is a
    // native Erlang term map built inside the dispatch closure.
    let ctx_send_wait = minijinja::context! {
        struct_name => struct_name
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_send_and_wait.rs.jinja",
        ctx_send_wait,
    ));

    // visitor_reply NIF: called by Elixir to unblock a waiting visitor callback.
    // Returns () which Rustler encodes as :ok.
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_reply_nif.rs.jinja",
        minijinja::context! {},
    ));

    // Trait impl — each method sends callback message and waits for reply.
    out.push_str(&crate::backends::rustler::template_env::render(
        "trait_impl_header.jinja",
        minijinja::context! {
            trait_path => trait_path,
            struct_name => struct_name,
        },
    ));
    // Classify which callback params marshal to the host as the binding's native struct term,
    // using the SHARED allowlist — identical to the plugin path and every other backend.
    let struct_param_types = native_marshalled_struct_params(trait_type, api);
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method_async(
            out,
            method,
            type_paths,
            struct_name,
            &result_metadata,
            &struct_param_types,
        );
    }
    out.push_str("}\n");
    out.push('\n');
    Ok(())
}

/// Generate a single async visitor method that sends a callback message to the Elixir
/// process and blocks on an mpsc channel waiting for the reply from `visitor_reply/2`.
///
/// Each argument is materialised into an OWNED, `Encoder`-able value before the dispatch
/// closure, then encoded into a NATIVE Erlang term map inside `send_and_clear` — so the
/// Elixir host receives native terms (structs/maps), not a JSON string. Serde-struct params
/// are built as the binding `NifStruct` via the shared allowlist (`struct_param_types`);
/// other args encode as their natural native terms.
fn gen_visitor_method_async(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<String, String>,
    _struct_name: &str,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
    struct_param_types: &HashSet<String>,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    // Convert method name from visit_* to handle_* for Elixir convention.
    // E.g., "visit_audio" -> "handle_audio"
    let handle_name = if let Some(suffix) = name.strip_prefix("visit_") {
        format!("handle_{suffix}")
    } else {
        name.clone()
    };

    // Build native-arg descriptors: each arg is materialised into an owned binding before the
    // dispatch closure and encoded into the native term map inside `send_and_clear`.
    let args: Vec<minijinja::Value> = build_native_args(&method.params, struct_param_types)
        .into_iter()
        .map(|a| {
            minijinja::context! {
                key => a.key,
                binding => a.binding,
                owned_expr => a.owned_expr,
            }
        })
        .collect();

    let ctx = minijinja::context! {
        method_name => name,
        sig => sig,
        ret_ty => ret_ty,
        default_result_expr => crate::codegen::visitor_result::default_result_expr(&ret_ty, result_metadata),
        unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
            &ret_ty,
            result_metadata,
            "s",
        ),
        unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
        payload_result_variants => crate::codegen::visitor_result::variant_contexts(
            &result_metadata.string_payload_variants,
        ),
        handle_name => handle_name,
        args => args
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "visitor_method.rs.jinja",
        ctx,
    ));
}
