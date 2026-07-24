use minijinja::{AutoEscape, Environment};

static TEMPLATES: &[(&str, &str)] = &[
    ("cargo_env_plain.jinja", include_str!("templates/cargo_env_plain.jinja")),
    (
        "cargo_env_structured.jinja",
        include_str!("templates/cargo_env_structured.jinja"),
    ),
    ("java_pom.xml.jinja", include_str!("templates/java_pom.xml.jinja")),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    env.set_auto_escape_callback(|_name| AutoEscape::None);
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
