use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "visitor_nodecontext_helper.jinja",
        include_str!("../templates/visitor_nodecontext_helper.jinja"),
    ),
    (
        "visitor_zval_to_visitresult.jinja",
        include_str!("../templates/visitor_zval_to_visitresult.jinja"),
    ),
    (
        "visitor_bridge_struct.jinja",
        include_str!("../templates/visitor_bridge_struct.jinja"),
    ),
    (
        "bridge_constructor.jinja",
        include_str!("../templates/bridge_constructor.jinja"),
    ),
    (
        "bridge_unregister_fn.jinja",
        include_str!("../templates/bridge_unregister_fn.jinja"),
    ),
    (
        "bridge_clear_fn.jinja",
        include_str!("../templates/bridge_clear_fn.jinja"),
    ),
    (
        "bridge_registration_fn.jinja",
        include_str!("../templates/bridge_registration_fn.jinja"),
    ),
    (
        "sync_method_body.jinja",
        include_str!("../templates/sync_method_body.jinja"),
    ),
    (
        "async_method_body.jinja",
        include_str!("../templates/async_method_body.jinja"),
    ),
    (
        "bridge_sync_impl.jinja",
        include_str!("../templates/bridge_sync_impl.jinja"),
    ),
    (
        "bridge_registration_validation.jinja",
        include_str!("../templates/bridge_registration_validation.jinja"),
    ),
    (
        "bridge_registration_body.jinja",
        include_str!("../templates/bridge_registration_body.jinja"),
    ),
    (
        "php_named_let_binding.jinja",
        include_str!("../templates/php_named_let_binding.jinja"),
    ),
    (
        "php_vec_named_let_binding.jinja",
        include_str!("../templates/php_vec_named_let_binding.jinja"),
    ),
    (
        "php_sanitized_vec_let_binding.jinja",
        include_str!("../templates/php_sanitized_vec_let_binding.jinja"),
    ),
    (
        "php_vec_string_refs_let_binding.jinja",
        include_str!("../templates/php_vec_string_refs_let_binding.jinja"),
    ),
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
