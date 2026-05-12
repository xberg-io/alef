use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "cargo_env_plain.jinja",
        include_str!("../templates/cargo_env_plain.jinja"),
    ),
    (
        "cargo_env_structured.jinja",
        include_str!("../templates/cargo_env_structured.jinja"),
    ),
    (
        "precommit_clippy_exclude.jinja",
        include_str!("../templates/precommit_clippy_exclude.jinja"),
    ),
    (
        "precommit_config.yaml.jinja",
        include_str!("../templates/precommit_config.yaml.jinja"),
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
    make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"))
}
