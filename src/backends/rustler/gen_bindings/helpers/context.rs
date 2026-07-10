use crate::core::config::ResolvedCrateConfig;
use heck::ToPascalCase;

/// Get module name and prefix from config or derive from crate name.
pub(in crate::backends::rustler::gen_bindings) fn get_module_info(
    _api: &crate::core::ir::ApiSurface,
    config: &ResolvedCrateConfig,
) -> (String, String) {
    let app_name = config.elixir_app_name();
    let module_prefix = app_name.to_pascal_case();
    (app_name, module_prefix)
}

/// Emit an Elixir documentation attribute (`@moduledoc`, `@typedoc`, `@doc`).
///
/// Multi-line docs use the heredoc form `@attr """ ... """`. Single-line docs use the
/// inline quoted form `@attr "..."`. Empty docs emit nothing (caller is expected to
/// fall back to `@moduledoc false` separately when wanted).
///
/// The `indent` string is prepended to every emitted line so the helper can be used
/// inside `defmodule` blocks (2-space indent) or at module scope.
pub(super) fn emit_elixir_doc_attr(out: &mut String, attr: &str, doc: &str, indent: &str) {
    if doc.trim().is_empty() {
        return;
    }
    let trimmed = doc.trim_end_matches('\n');
    if !trimmed.contains('\n') {
        let escaped = trimmed.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(indent);
        out.push('@');
        out.push_str(attr);
        out.push_str(" \"");
        out.push_str(&escaped);
        out.push_str("\"\n");
        return;
    }
    out.push_str(indent);
    out.push('@');
    out.push_str(attr);
    out.push_str(" \"\"\"\n");
    for line in trimmed.lines() {
        let safe = line.replace("\"\"\"", "\"\" \"");
        if safe.is_empty() {
            out.push('\n');
        } else {
            out.push_str(indent);
            out.push_str(&safe);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str("\"\"\"\n");
}
