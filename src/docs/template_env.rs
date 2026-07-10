use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("front_matter.jinja", include_str!("templates/front_matter.jinja")),
    ("version_heading.jinja", include_str!("templates/version_heading.jinja")),
    ("heading.jinja", include_str!("templates/heading.jinja")),
    ("code_block.jinja", include_str!("templates/code_block.jinja")),
    ("param_row.jinja", include_str!("templates/param_row.jinja")),
    ("field_row.jinja", include_str!("templates/field_row.jinja")),
    ("variant_row.jinja", include_str!("templates/variant_row.jinja")),
    ("exception_row.jinja", include_str!("templates/exception_row.jinja")),
    (
        "wire_variant_row.jinja",
        include_str!("templates/wire_variant_row.jinja"),
    ),
    (
        "error_message_row.jinja",
        include_str!("templates/error_message_row.jinja"),
    ),
    ("returns.jinja", include_str!("templates/returns.jinja")),
    ("errors_phrase.jinja", include_str!("templates/errors_phrase.jinja")),
    ("base_class.jinja", include_str!("templates/base_class.jinja")),
    ("bold_heading.jinja", include_str!("templates/bold_heading.jinja")),
    ("since_badge.jinja", include_str!("templates/since_badge.jinja")),
    (
        "deprecated_notice.jinja",
        include_str!("templates/deprecated_notice.jinja"),
    ),
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
    let rendered = make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"));
    if matches!(template_name, "heading.jinja" | "version_heading.jinja") && !rendered.ends_with("\n\n") {
        return format!("{rendered}\n");
    }
    rendered
}
