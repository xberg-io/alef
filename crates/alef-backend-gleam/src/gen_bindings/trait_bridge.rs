use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::nif_external::{gleam_type, resolve_gleam_error_type};

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
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    use heck::ToSnakeCase;

    let trait_name = &bridge_cfg.trait_name;
    let trait_snake = trait_name.to_snake_case();

    // Documentation comment
    out.push_str(&format!("/// Trait bridge shims for `{trait_name}`.\n"));
    out.push_str("///\n");
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
        out.push_str(&format!(
            "@external(erlang, \"{nif_module}\", \"{register_fn}\")\n"
        ));
        out.push_str(&format!(
            "pub fn register_{trait_snake}(pid: Dynamic, plugin_name: String) -> Nil\n"
        ));
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
            let ok_type = match &method.return_type {
                TypeRef::Unit => "Nil".to_string(),
                other => gleam_type(other, false, imports),
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
            out.push_str(&format!(
                "/// Send the `{method_snake}` response back to the Rustler reply-registry.\n"
            ));
            out.push_str("///\n");
            out.push_str(&format!(
                "/// Call this from your `handle_info/2` after processing a\n\
                 /// `{{:trait_call, \"{method_snake}\", args_json, call_id}}` message:\n"
            ));
            out.push_str("///\n");
            out.push_str(&format!(
                "/// ```gleam\n\
                 /// // pub fn handle_info(msg, state) {{\n\
                 /// //   case msg {{\n\
                 /// //     #(atom.create(\"{method_snake}\"), args_json, call_id) ->\n\
                 /// //       let result = do_{method_snake}(args_json)\n\
                 /// //       {nif_fn_name}(call_id, result)\n\
                 /// //       actor.continue(state)\n\
                 /// //     _ -> actor.continue(state)\n\
                 /// //   }}\n\
                 /// // }}\n\
                 /// ```\n"
            ));

            imports.insert("import gleam/dynamic.{type Dynamic}");
            out.push_str(&format!(
                "@external(erlang, \"{nif_module}\", \"{nif_fn_name}\")\n"
            ));
            out.push_str(&format!(
                "pub fn {nif_fn_name}(call_id: Dynamic, result: Result({ok_type}, {err_type})) -> Nil\n"
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
    out.push_str("/// Complete a pending trait call with a successful JSON result.\n");
    out.push_str("/// Call this from your GenServer after processing a trait_call message.\n");
    out.push_str(&format!(
        "@external(erlang, \"{nif_module}\", \"complete_trait_call\")\n"
    ));
    out.push_str("pub fn complete_trait_call(reply_id: Int, result_json: String) -> Nil\n");
    out.push('\n');

    out.push_str("/// Fail a pending trait call with an error message.\n");
    out.push_str("/// Call this from your GenServer when processing a trait_call message fails.\n");
    out.push_str(&format!(
        "@external(erlang, \"{nif_module}\", \"fail_trait_call\")\n"
    ));
    out.push_str("pub fn fail_trait_call(reply_id: Int, error_message: String) -> Nil\n");
    out.push('\n');
}
