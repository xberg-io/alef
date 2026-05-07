use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "java_file_header.jinja",
        include_str!("../templates/java_file_header.jinja"),
    ),
    (
        "facade_class.jinja",
        include_str!("../templates/facade_class.jinja"),
    ),
    (
        "facade_file.jinja",
        include_str!("../templates/facade_file.jinja"),
    ),
    (
        "native_lib.jinja",
        include_str!("../templates/native_lib.jinja"),
    ),
    (
        "visitor_bridge.jinja",
        include_str!("../templates/visitor_bridge.jinja"),
    ),
    (
        "trait_interface.jinja",
        include_str!("../templates/trait_interface.jinja"),
    ),
    (
        "convert_with_visitor.jinja",
        include_str!("../templates/convert_with_visitor.jinja"),
    ),
    (
        "handle_method.jinja",
        include_str!("../templates/handle_method.jinja"),
    ),
    (
        "helper_check_last_error.jinja",
        include_str!("../templates/helper_check_last_error.jinja"),
    ),
    (
        "helper_object_mapper.jinja",
        include_str!("../templates/helper_object_mapper.jinja"),
    ),
    (
        "helper_read_bytes.jinja",
        include_str!("../templates/helper_read_bytes.jinja"),
    ),
    (
        "helper_read_cstring.jinja",
        include_str!("../templates/helper_read_cstring.jinja"),
    ),
    (
        "helper_read_json_list.jinja",
        include_str!("../templates/helper_read_json_list.jinja"),
    ),
    (
        "native_lib_visitor_handles.jinja",
        include_str!("../templates/native_lib_visitor_handles.jinja"),
    ),
    (
        "visitor_files.jinja",
        include_str!("../templates/visitor_files.jinja"),
    ),
    (
        "exception_class.jinja",
        include_str!("../templates/exception_class.jinja"),
    ),
    (
        "infrastructure_exception.jinja",
        include_str!("../templates/infrastructure_exception.jinja"),
    ),
    (
        "node_context.jinja",
        include_str!("../templates/node_context.jinja"),
    ),
    (
        "visit_result.jinja",
        include_str!("../templates/visit_result.jinja"),
    ),
    (
        "visitor_interface.jinja",
        include_str!("../templates/visitor_interface.jinja"),
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
