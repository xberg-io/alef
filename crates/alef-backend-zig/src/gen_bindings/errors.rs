use alef_core::ir::ErrorDef;

fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in name.chars() {
        if ch == '_' {
            capitalize = true;
        } else if capitalize {
            out.push(ch.to_uppercase().next().unwrap());
            capitalize = false;
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn emit_error_set(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        for line in error.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("pub const {} = error {{\n", error.name));
    for variant in &error.variants {
        out.push_str(&format!("    {},\n", to_pascal_case(&variant.name)));
    }
    out.push_str("};\n");
}

/// Map a Rust error_type (e.g. `"anyhow::Error"`, `"KreuzbergError"`) to a
/// Zig error-set identifier. If the path's last segment matches a declared
/// error set, use it; otherwise fall back to the first declared error set
/// (the project's main error type).
pub(crate) fn resolve_zig_error_type(error_type: &str, declared: &[String]) -> String {
    let last = error_type.rsplit("::").next().unwrap_or(error_type);
    if declared.iter().any(|d| d == last) {
        return last.to_string();
    }
    declared.first().cloned().unwrap_or_else(|| "anyerror".to_string())
}
