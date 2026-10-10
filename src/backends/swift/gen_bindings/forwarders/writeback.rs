//! Forwarder emission for free functions with a single `&mut T` DTO writeback parameter.
//!
//! Split out of `forwarders.rs` (already at the file-size cap) instead of growing it. See
//! `crate::codegen::mut_writeback` for the policy this backs: a Rust value type crossing the
//! swift-bridge boundary has no shared identity to mutate through, so the generated Rust
//! bridge (`shims::emit_function_shim`) now returns the updated value instead of `()`. The
//! function's own IR `return_type` still records the original `Unit` — extraction is
//! unchanged — so `emit_single_free_function_forwarder`'s and
//! `emit_async_free_function_forwarder`'s return-shape dispatch would never pick the
//! DTO-return template on their own. This module forces that template unconditionally, using
//! the same DTO round-trip machinery (`intoRust()` / `T(_rb)`) every other DTO-returning
//! forwarder already relies on. ~keep

use super::*;

/// Emit the free-function forwarder for `func`, whose bridged Rust return type is
/// `writeback_type_name` rather than the IR's own `func.return_type` (`Unit`).
pub(super) fn emit_free_function_forwarder(
    func: &FunctionDef,
    swift_name: &str,
    writeback_type_name: &str,
    known_dto_names: &HashSet<String>,
    out: &mut String,
) {
    let (sig, args, conversion_body) = build_params(func, known_dto_names);
    let struct_name = swift_ident(writeback_type_name);

    if !func.doc.is_empty() {
        emit_doc_comment(&func.doc, "", out);
    }

    if func.is_async {
        let bridge_call = format!("try await RustBridge.{swift_name}({args})");
        let return_stmt = format!("        return try {struct_name}(_rb_obj)");
        let body = crate::backends::swift::template_env::render(
            "swift_forwarder_dto_return_body.swift.jinja",
            crate::alef_context! {
                bridge_call => &bridge_call,
                return_statement => &return_stmt,
            },
        );
        out.push_str(&render_async_forwarder(
            swift_name,
            &sig,
            &format!(" -> {struct_name}"),
            &conversion_body,
            &body,
        ));
        out.push('\n');
    } else {
        let body = crate::backends::swift::template_env::render(
            "swift_sync_forwarder_dto_return_body.swift.jinja",
            crate::alef_context! {
                bridge_call_try => if func.error_type.is_some() { "try " } else { "" },
                function_name => swift_name,
                args => &args,
                dto_name => &struct_name,
            },
        );
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_sync_forwarder.swift.jinja",
            crate::alef_context! {
                function_name => swift_name,
                params => &sig,
                throws_clause => " throws",
                return_clause => format!(" -> {struct_name}"),
                conversion_lines => conversion_body,
                body => body,
            },
        ));
    }
}

/// Build the parameter signature, call-site arguments, and rendered conversion-line block
/// shared by both the sync and async writeback forwarder shapes.
fn build_params(func: &FunctionDef, known_dto_names: &HashSet<String>) -> (String, String, String) {
    let mut sig_params: Vec<String> = Vec::with_capacity(func.params.len());
    let mut conversion_lines: Vec<String> = Vec::new();
    let mut call_args: Vec<String> = Vec::with_capacity(func.params.len());

    for param in &func.params {
        let swift_param_name = swift_ident(&param.name.to_lower_camel_case());
        let (swift_ty, local_expr) =
            forwarder_param_signature(&param.ty, &swift_param_name, param.optional, known_dto_names);
        let param_default = if param.optional { " = nil" } else { "" };
        sig_params.push(format!("{swift_param_name}: {swift_ty}{param_default}"));
        if let Some(line) = local_expr.setup_line.clone() {
            conversion_lines.push(line);
        }
        call_args.push(local_expr.arg_expr);
    }

    let mut conversion_body = String::new();
    for line in &conversion_lines {
        conversion_body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_conversion_line.swift.jinja",
            crate::alef_context! { line => line, },
        ));
    }

    (sig_params.join(", "), call_args.join(", "), conversion_body)
}
