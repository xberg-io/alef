use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("kt_file.jinja", include_str!("templates/kt_file.jinja")),
    (
        "trait_bridge_object.jinja",
        include_str!("templates/trait_bridge_object.jinja"),
    ),
    (
        "trait_bridge_dispatcher.jinja",
        include_str!("templates/trait_bridge_dispatcher.jinja"),
    ),
    (
        "handle_wrapper_header.jinja",
        include_str!("templates/handle_wrapper_header.jinja"),
    ),
    (
        "android_streaming_method.jinja",
        include_str!("templates/android_streaming_method.jinja"),
    ),
    (
        "module_object_header.jinja",
        include_str!("templates/module_object_header.jinja"),
    ),
    (
        "android_facade_dto_method.jinja",
        include_str!("templates/android_facade_dto_method.jinja"),
    ),
    (
        "android_facade_generic_method.jinja",
        include_str!("templates/android_facade_generic_method.jinja"),
    ),
    (
        "android_facade_async_method.jinja",
        include_str!("templates/android_facade_async_method.jinja"),
    ),
    (
        "android_facade_expr_method.jinja",
        include_str!("templates/android_facade_expr_method.jinja"),
    ),
    (
        "trait_interface_header.jinja",
        include_str!("templates/trait_interface_header.jinja"),
    ),
    (
        "trait_method_return_line.jinja",
        include_str!("templates/trait_method_return_line.jinja"),
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
