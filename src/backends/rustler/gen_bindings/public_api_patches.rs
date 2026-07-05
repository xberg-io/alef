use crate::core::backend::GeneratedFile;

/// Post-process the generated `{App}.Native` NIF-stub module in place.
///
/// The stub module is produced by `gen_native_ex` (helpers/nif_service.rs), which is
/// outside this module's permitted edit scope, so the load-critical fix is applied
/// here on its already-rendered content:
///
/// Declare the always-emitted `set_env/2` support NIF. The native codegen emits the
/// `#[rustler::nif] set_env` function unconditionally and `rustler::init!` discovers it,
/// so without a matching Elixir stub `:erlang.load_nif` aborts the whole module.
///
/// This edit is idempotent, so it stays correct if `gen_native_ex` is later fixed to
/// emit this directly.
pub(super) fn patch_native_stub_module(
    files: &mut [GeneratedFile],
    _config: &crate::core::config::ResolvedCrateConfig,
) {
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
}
