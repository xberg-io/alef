use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "opaque_struct.rs.jinja",
        include_str!("../templates/opaque_struct.rs.jinja"),
    ),
    ("struct_def.rs.jinja", include_str!("../templates/struct_def.rs.jinja")),
    ("enum_def.rs.jinja", include_str!("../templates/enum_def.rs.jinja")),
    ("enum_magnus.rs.jinja", include_str!("../templates/enum_magnus.rs.jinja")),
    (
        "visitor_bridge_struct.rs.jinja",
        include_str!("../templates/visitor_bridge_struct.rs.jinja"),
    ),
    (
        "visitor_method.rs.jinja",
        include_str!("../templates/visitor_method.rs.jinja"),
    ),
    (
        "bridge_struct_impl.rs.jinja",
        include_str!("../templates/bridge_struct_impl.rs.jinja"),
    ),
    (
        "visitor_bridge_wrapper.rs.jinja",
        include_str!("../templates/visitor_bridge_wrapper.rs.jinja"),
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
