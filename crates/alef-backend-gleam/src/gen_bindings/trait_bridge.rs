use alef_core::config::TraitBridgeConfig;
use alef_core::ir::TypeDef;
use std::collections::BTreeSet;

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
