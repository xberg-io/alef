//! Minijinja environment for packaging templates.

use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[(
    "elixir_checksums.jinja",
    include_str!("templates/elixir_checksums.jinja"),
)];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    crate::template::configure_env(&mut env);
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
    // ~keep `~keep` is a marker for alef's own `poly` uncomment pass, meaningful only in a tree
    // `poly` reads. This module renders into a CONSUMER's package tree, so a marker left in a
    // packaging template would ship verbatim into their checksum file. Every other built-in
    // `template_env` strips here; this one was written by hand and never inherited the call.
    crate::core::keep_marker::strip_keep_markers(&rendered)
}
