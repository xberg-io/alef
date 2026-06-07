/// Emit a `{var} = {body}` line, guarded by `if {pname} is not None else None`
/// when the parameter is optional.
pub(in crate::backends::pyo3::gen_bindings) fn emit_param_conversion(
    out: &mut String,
    var: &str,
    pname: &str,
    body: &str,
    optional: bool,
) {
    if optional {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "param_conversion_optional.jinja",
            minijinja::context! {
                var => var,
                body => body,
                pname => pname,
            },
        ));
    } else {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "param_conversion.jinja",
            minijinja::context! {
                var => var,
                body => body,
            },
        ));
    }
    out.push('\n');
}
