fn emit_runtime_helpers(out: &mut String) {
    out.push_str(&template_env::render("runtime_helpers.rs.jinja", context! {}));
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
