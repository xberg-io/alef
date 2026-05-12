use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "package_declaration.jinja",
        include_str!("../templates/package_declaration.jinja"),
    ),
    (
        "object_declaration.jinja",
        include_str!("../templates/object_declaration.jinja"),
    ),
    (
        "expect_object_declaration.jinja",
        include_str!("../templates/expect_object_declaration.jinja"),
    ),
    (
        "actual_object_declaration.jinja",
        include_str!("../templates/actual_object_declaration.jinja"),
    ),
    ("empty_class.jinja", include_str!("../templates/empty_class.jinja")),
    (
        "data_class_header.jinja",
        include_str!("../templates/data_class_header.jinja"),
    ),
    ("class_field.jinja", include_str!("../templates/class_field.jinja")),
    (
        "enum_class_header.jinja",
        include_str!("../templates/enum_class_header.jinja"),
    ),
    ("enum_variant.jinja", include_str!("../templates/enum_variant.jinja")),
    (
        "sealed_class_header.jinja",
        include_str!("../templates/sealed_class_header.jinja"),
    ),
    (
        "sealed_object_variant.jinja",
        include_str!("../templates/sealed_object_variant.jinja"),
    ),
    (
        "variant_data_class_header.jinja",
        include_str!("../templates/variant_data_class_header.jinja"),
    ),
    (
        "variant_class_field.jinja",
        include_str!("../templates/variant_class_field.jinja"),
    ),
    ("variant_close.jinja", include_str!("../templates/variant_close.jinja")),
    (
        "error_sealed_class_header.jinja",
        include_str!("../templates/error_sealed_class_header.jinja"),
    ),
    (
        "error_object_variant.jinja",
        include_str!("../templates/error_object_variant.jinja"),
    ),
    ("error_field.jinja", include_str!("../templates/error_field.jinja")),
    (
        "error_variant_close.jinja",
        include_str!("../templates/error_variant_close.jinja"),
    ),
    (
        "typealias_trait.jinja",
        include_str!("../templates/typealias_trait.jinja"),
    ),
    (
        "typealias_type.jinja",
        include_str!("../templates/typealias_type.jinja"),
    ),
    (
        "typealias_error.jinja",
        include_str!("../templates/typealias_error.jinja"),
    ),
    (
        "function_signature.jinja",
        include_str!("../templates/function_signature.jinja"),
    ),
    (
        "expect_function_signature.jinja",
        include_str!("../templates/expect_function_signature.jinja"),
    ),
    ("doc_comment.jinja", include_str!("../templates/doc_comment.jinja")),
    (
        "bridge_call_with_dispatch.jinja",
        include_str!("../templates/bridge_call_with_dispatch.jinja"),
    ),
    (
        "bridge_call_unit.jinja",
        include_str!("../templates/bridge_call_unit.jinja"),
    ),
    (
        "bridge_call_return.jinja",
        include_str!("../templates/bridge_call_return.jinja"),
    ),
    (
        "native_function_header.jinja",
        include_str!("../templates/native_function_header.jinja"),
    ),
    (
        "native_result_assign.jinja",
        include_str!("../templates/native_result_assign.jinja"),
    ),
    (
        "native_error_code_check.jinja",
        include_str!("../templates/native_error_code_check.jinja"),
    ),
    (
        "native_error_message.jinja",
        include_str!("../templates/native_error_message.jinja"),
    ),
    (
        "native_call_only.jinja",
        include_str!("../templates/native_call_only.jinja"),
    ),
    (
        "native_return_expr.jinja",
        include_str!("../templates/native_return_expr.jinja"),
    ),
    (
        "native_param_cstr_conversion.jinja",
        include_str!("../templates/native_param_cstr_conversion.jinja"),
    ),
    (
        "native_param_bytes_conversion.jinja",
        include_str!("../templates/native_param_bytes_conversion.jinja"),
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
