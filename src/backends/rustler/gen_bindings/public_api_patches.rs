use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use heck::ToSnakeCase;

/// Post-process the generated `{App}.Native` NIF-stub module in place.
///
/// The stub module is produced by `gen_native_ex` (helpers/nif_service.rs), which is
/// outside this module's permitted edit scope, so the two load-critical fixes are applied
/// here on its already-rendered content:
///
/// 1. Declare the always-emitted `set_env/2` support NIF. The native codegen emits the
///    `#[rustler::nif] set_env` function unconditionally and `rustler::init!` discovers it,
///    so without a matching Elixir stub `:erlang.load_nif` aborts the whole module.
/// 2. Force plugin-`register_*` stub arities to `(_pid, _name)`. These register NIFs take
///    `(pid, name)`, but they also surface as one-argument public-API functions whose
///    one-arg stub shadows the bridge's two-arg stub via name-based dedup, leaving a `/1`
///    stub that does not match the `/2` NIF (another `load_nif` abort).
///
/// Both edits are idempotent, so they stay correct if `gen_native_ex` is later fixed to
/// emit these directly.
pub(super) fn patch_native_stub_module(files: &mut [GeneratedFile], config: &ResolvedCrateConfig) {
    let Some(native) = files
        .iter_mut()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("native.ex"))
    else {
        return;
    };

    if !native.content.contains("def set_env(") {
        const ANCHOR: &str = "or Mix.env() in [:dev]\n";
        if let Some(pos) = native.content.find(ANCHOR) {
            let insert_at = pos + ANCHOR.len();
            native.content.insert_str(
                insert_at,
                "\n  @doc false\n  def set_env(_key, _value), do: :erlang.nif_error(:nif_not_loaded)\n",
            );
        }
    }

    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler") {
            continue;
        }
        let Some(register_fn) = bridge.register_fn.as_deref() else {
            continue;
        };
        let needle = format!("def {}(", register_fn.to_snake_case());
        let Some(start) = native.content.find(&needle) else {
            continue;
        };
        let args_start = start + needle.len();
        let Some(rel_end) = native.content[args_start..].find(')') else {
            continue;
        };
        let args_end = args_start + rel_end;
        if &native.content[args_start..args_end] != "_pid, _name" {
            native.content.replace_range(args_start..args_end, "_pid, _name");
        }
    }
}
