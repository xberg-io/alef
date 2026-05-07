use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("namespace_decl.jinja", include_str!("../templates/namespace_decl.jinja")),
    ("class_header.jinja", include_str!("../templates/class_header.jinja")),
    ("async_task_return_type.jinja", include_str!("../templates/async_task_return_type.jinja")),
    ("param_doc.jinja", include_str!("../templates/param_doc.jinja")),
    ("param_decl_optional.jinja", include_str!("../templates/param_decl_optional.jinja")),
    ("param_decl_required.jinja", include_str!("../templates/param_decl_required.jinja")),
    ("null_check.jinja", include_str!("../templates/null_check.jinja")),
    ("error_dispatch.jinja", include_str!("../templates/error_dispatch.jinja")),
    ("native_call_start.jinja", include_str!("../templates/native_call_start.jinja")),
    ("sealed_class_header.jinja", include_str!("../templates/sealed_class_header.jinja")),
    ("safe_handle_class.jinja", include_str!("../templates/safe_handle_class.jinja")),
    ("safe_handle_ctor.jinja", include_str!("../templates/safe_handle_ctor.jinja")),
    ("release_handle_method.jinja", include_str!("../templates/release_handle_method.jinja")),
    ("safehandle_field.jinja", include_str!("../templates/safehandle_field.jinja")),
    ("opaque_class_ctor.jinja", include_str!("../templates/opaque_class_ctor.jinja")),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    for (name, src) in TEMPLATES {
        env.add_template(name, src).expect("built-in template is valid");
    }
    env
}

pub(crate) fn render(template_name: &str, ctx: minijinja::Value) -> String {
    make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"))
}
