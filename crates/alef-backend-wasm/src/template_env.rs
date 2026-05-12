use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "gen_unregistration_fn",
        include_str!("../templates/gen_unregistration_fn.jinja"),
    ),
    ("gen_clear_fn", include_str!("../templates/gen_clear_fn.jinja")),
    ("gen_constructor", include_str!("../templates/gen_constructor.jinja")),
    (
        "gen_registration_fn",
        include_str!("../templates/gen_registration_fn.jinja"),
    ),
    (
        "gen_sync_method_body",
        include_str!("../templates/gen_sync_method_body.jinja"),
    ),
    (
        "gen_async_method_body",
        include_str!("../templates/gen_async_method_body.jinja"),
    ),
    (
        "gen_visitor_bridge",
        include_str!("../templates/gen_visitor_bridge.jinja"),
    ),
    (
        "gen_visitor_method_wasm",
        include_str!("../templates/gen_visitor_method_wasm.jinja"),
    ),
    ("rustdoc", include_str!("../templates/rustdoc.jinja")),
    (
        "gen_bridge_function",
        include_str!("../templates/gen_bridge_function.jinja"),
    ),
    (
        "gen_opaque_struct",
        include_str!("../templates/gen_opaque_struct.jinja"),
    ),
    ("gen_struct", include_str!("../templates/gen_struct.jinja")),
    (
        "gen_visitor_handle_constructor",
        include_str!("../templates/gen_visitor_handle_constructor.jinja"),
    ),
    (
        "serde_named_optional",
        include_str!("../templates/serde_named_optional.jinja"),
    ),
    (
        "serde_named_required",
        include_str!("../templates/serde_named_required.jinja"),
    ),
    (
        "serde_vec_named_optional",
        include_str!("../templates/serde_vec_named_optional.jinja"),
    ),
    (
        "serde_vec_named_required",
        include_str!("../templates/serde_vec_named_required.jinja"),
    ),
    (
        "serde_vec_string_refs_optional",
        include_str!("../templates/serde_vec_string_refs_optional.jinja"),
    ),
    (
        "serde_vec_string_refs_required",
        include_str!("../templates/serde_vec_string_refs_required.jinja"),
    ),
    (
        "serde_vec_nested_optional",
        include_str!("../templates/serde_vec_nested_optional.jinja"),
    ),
    (
        "serde_vec_nested_required",
        include_str!("../templates/serde_vec_nested_required.jinja"),
    ),
    (
        "serde_vec_tuple_optional",
        include_str!("../templates/serde_vec_tuple_optional.jinja"),
    ),
    (
        "serde_vec_tuple_required",
        include_str!("../templates/serde_vec_tuple_required.jinja"),
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
