use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("type_opaque.jinja", include_str!("../templates/type_opaque.jinja")),
    (
        "type_opaque_resource.jinja",
        include_str!("../templates/type_opaque_resource.jinja"),
    ),
    (
        "resource_method_external.jinja",
        include_str!("../templates/resource_method_external.jinja"),
    ),
    ("type_header.jinja", include_str!("../templates/type_header.jinja")),
    ("field_labeled.jinja", include_str!("../templates/field_labeled.jinja")),
    (
        "field_positional.jinja",
        include_str!("../templates/field_positional.jinja"),
    ),
    (
        "variant_simple.jinja",
        include_str!("../templates/variant_simple.jinja"),
    ),
    (
        "variant_with_fields.jinja",
        include_str!("../templates/variant_with_fields.jinja"),
    ),
    ("enum_header.jinja", include_str!("../templates/enum_header.jinja")),
    ("error_header.jinja", include_str!("../templates/error_header.jinja")),
    (
        "function_external.jinja",
        include_str!("../templates/function_external.jinja"),
    ),
    (
        "function_signature.jinja",
        include_str!("../templates/function_signature.jinja"),
    ),
    (
        "trait_bridge_doc_header.jinja",
        include_str!("../templates/trait_bridge_doc_header.jinja"),
    ),
    ("register_fn.jinja", include_str!("../templates/register_fn.jinja")),
    ("unregister_fn.jinja", include_str!("../templates/unregister_fn.jinja")),
    ("clear_fn.jinja", include_str!("../templates/clear_fn.jinja")),
    (
        "method_doc_header.jinja",
        include_str!("../templates/method_doc_header.jinja"),
    ),
    (
        "method_doc_usage.jinja",
        include_str!("../templates/method_doc_usage.jinja"),
    ),
    (
        "method_external.jinja",
        include_str!("../templates/method_external.jinja"),
    ),
    (
        "method_signature.jinja",
        include_str!("../templates/method_signature.jinja"),
    ),
    (
        "trait_type_doc_lines.jinja",
        include_str!("../templates/trait_type_doc_lines.jinja"),
    ),
    ("import_line.jinja", include_str!("../templates/import_line.jinja")),
    (
        "trait_bridge_doc_line.jinja",
        include_str!("../templates/trait_bridge_doc_line.jinja"),
    ),
    (
        "trait_bridge_empty_comment_line.jinja",
        include_str!("../templates/trait_bridge_empty_comment_line.jinja"),
    ),
    (
        "trait_scope_cap.jinja",
        include_str!("../templates/trait_scope_cap.jinja"),
    ),
    (
        "support_nif_doc.jinja",
        include_str!("../templates/support_nif_doc.jinja"),
    ),
    (
        "support_nif_complete.jinja",
        include_str!("../templates/support_nif_complete.jinja"),
    ),
    (
        "support_nif_fail.jinja",
        include_str!("../templates/support_nif_fail.jinja"),
    ),
    (
        "support_nif_fail_doc.jinja",
        include_str!("../templates/support_nif_fail_doc.jinja"),
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
