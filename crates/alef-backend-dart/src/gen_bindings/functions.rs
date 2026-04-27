use alef_core::ir::{FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use super::render_type::{format_param, render_type};

pub(super) fn emit_function(f: &FunctionDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !f.doc.is_empty() {
        for line in f.doc.lines() {
            out.push_str("  /// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&format!("  /// throws {error_ty} on failure\n"));
    }

    let fn_name = f.name.to_lower_camel_case();
    let params: Vec<String> = f.params.iter().map(|p| format_param(p, imports)).collect();
    let call_args: Vec<String> = f.params.iter().map(|p| p.name.to_lower_camel_case()).collect();
    let call_args_str = call_args.join(", ");

    if f.is_async {
        let return_ty = if matches!(f.return_type, TypeRef::Unit) {
            "Future<void>".to_string()
        } else {
            format!("Future<{}>", render_type(&f.return_type, imports))
        };
        out.push_str(&format!(
            "  static {return_ty} {fn_name}({}) async {{\n",
            params.join(", ")
        ));
        out.push_str(&format!("    return await rust_bridge.{fn_name}({call_args_str});\n"));
        out.push_str("  }\n");
    } else {
        let return_ty = render_type(&f.return_type, imports);
        out.push_str(&format!(
            "  static {return_ty} {fn_name}({}) {{\n",
            params.join(", ")
        ));
        out.push_str(&format!("    return rust_bridge.{fn_name}({call_args_str});\n"));
        out.push_str("  }\n");
    }
}
