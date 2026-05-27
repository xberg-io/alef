use crate::core::ir::ErrorDef;

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
        out.push_str(&crate::backends::zig::template_env::render(
            "error_doc_block.jinja",
            minijinja::context! {
                error_doc_lines => error.doc.lines().collect::<Vec<_>>(),
            },
        ));
    }
    out.push_str(&crate::backends::zig::template_env::render(
        "error_set_header.jinja",
        minijinja::context! {
            error_name => &error.name,
        },
    ));
    for variant in &error.variants {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => to_pascal_case(&variant.name),
            },
        ));
    }
    // OutOfMemory is always included so allocator failures can be propagated
    // without a `||error{OutOfMemory}` concat on every return type.
    // Only emit if not already present as a user-defined variant.
    if !error.variants.iter().any(|v| to_pascal_case(&v.name) == "OutOfMemory") {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => "OutOfMemory",
            },
        ));
    }
    out.push_str("};\n");
}

/// Map a Rust error_type (e.g. `"anyhow::Error"`, `"SampleCrateError"`) to a
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
