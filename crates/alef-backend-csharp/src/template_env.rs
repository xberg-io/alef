#![allow(dead_code)]
// Scaffolding for the C# backend's planned migration to minijinja templates.
// Items are unused until the first templates are ported in; suppress dead_code
// rather than scaffold-and-delete-and-re-add as each template lands.

use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    // Templates will be added here as migration progresses
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
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
