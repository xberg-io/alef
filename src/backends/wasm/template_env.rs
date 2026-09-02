use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "gen_unregistration_fn",
        include_str!("templates/gen_unregistration_fn.jinja"),
    ),
    ("gen_clear_fn", include_str!("templates/gen_clear_fn.jinja")),
    ("gen_constructor", include_str!("templates/gen_constructor.jinja")),
    (
        "gen_registration_fn",
        include_str!("templates/gen_registration_fn.jinja"),
    ),
    (
        "gen_sync_method_body",
        include_str!("templates/gen_sync_method_body.jinja"),
    ),
    (
        "gen_async_method_body",
        include_str!("templates/gen_async_method_body.jinja"),
    ),
    ("gen_visitor_bridge", include_str!("templates/gen_visitor_bridge.jinja")),
    (
        "gen_visitor_method_wasm",
        include_str!("templates/gen_visitor_method_wasm.jinja"),
    ),
    ("rustdoc", include_str!("templates/rustdoc.jinja")),
    (
        "gen_bridge_function",
        include_str!("templates/gen_bridge_function.jinja"),
    ),
    (
        "gen_options_field_bridge_body",
        include_str!("templates/gen_options_field_bridge_body.jinja"),
    ),
    ("gen_free_function", include_str!("templates/gen_free_function.jinja")),
    (
        "tagged_enum_unmapped_core_arm",
        include_str!("templates/tagged_enum_unmapped_core_arm.jinja"),
    ),
    (
        "gen_instance_method",
        include_str!("templates/gen_instance_method.jinja"),
    ),
    ("gen_static_method", include_str!("templates/gen_static_method.jinja")),
    ("gen_result_body", include_str!("templates/gen_result_body.jinja")),
    ("gen_direct_body", include_str!("templates/gen_direct_body.jinja")),
    ("env_shims", include_str!("templates/env_shims.jinja")),
    (
        "gen_unit_result_body",
        include_str!("templates/gen_unit_result_body.jinja"),
    ),
    (
        "serde_vec_named_from_optional",
        include_str!("templates/serde_vec_named_from_optional.jinja"),
    ),
    (
        "serde_vec_named_from_required",
        include_str!("templates/serde_vec_named_from_required.jinja"),
    ),
    (
        "lifetime_string_optional",
        include_str!("templates/lifetime_string_optional.jinja"),
    ),
    (
        "lifetime_string_required",
        include_str!("templates/lifetime_string_required.jinja"),
    ),
    (
        "lifetime_map_required",
        include_str!("templates/lifetime_map_required.jinja"),
    ),
    ("gen_opaque_struct", include_str!("templates/gen_opaque_struct.jinja")),
    ("gen_struct", include_str!("templates/gen_struct.jinja")),
    (
        "gen_visitor_handle_constructor",
        include_str!("templates/gen_visitor_handle_constructor.jinja"),
    ),
    (
        "serde_named_optional",
        include_str!("templates/serde_named_optional.jinja"),
    ),
    (
        "serde_named_required",
        include_str!("templates/serde_named_required.jinja"),
    ),
    (
        "wasm_typed_named_optional",
        include_str!("templates/wasm_typed_named_optional.jinja"),
    ),
    (
        "wasm_typed_named_required",
        include_str!("templates/wasm_typed_named_required.jinja"),
    ),
    (
        "serde_vec_named_optional",
        include_str!("templates/serde_vec_named_optional.jinja"),
    ),
    (
        "serde_vec_named_required",
        include_str!("templates/serde_vec_named_required.jinja"),
    ),
    (
        "serde_vec_string_refs_optional",
        include_str!("templates/serde_vec_string_refs_optional.jinja"),
    ),
    (
        "serde_vec_string_refs_required",
        include_str!("templates/serde_vec_string_refs_required.jinja"),
    ),
    (
        "serde_vec_nested_optional",
        include_str!("templates/serde_vec_nested_optional.jinja"),
    ),
    (
        "serde_vec_nested_required",
        include_str!("templates/serde_vec_nested_required.jinja"),
    ),
    (
        "serde_vec_tuple_optional",
        include_str!("templates/serde_vec_tuple_optional.jinja"),
    ),
    (
        "serde_vec_tuple_required",
        include_str!("templates/serde_vec_tuple_required.jinja"),
    ),
    (
        "serde_vec_unit_enum_optional",
        include_str!("templates/serde_vec_unit_enum_optional.jinja"),
    ),
    (
        "serde_vec_unit_enum_required",
        include_str!("templates/serde_vec_unit_enum_required.jinja"),
    ),
    ("gen_input_dto", include_str!("templates/gen_input_dto.jinja")),
    (
        "serde_config_required",
        include_str!("templates/serde_config_required.jinja"),
    ),
    (
        "serde_config_optional",
        include_str!("templates/serde_config_optional.jinja"),
    ),
    (
        "service_js_class_open.jinja",
        include_str!("templates/service_js_class_open.jinja"),
    ),
    (
        "service_js_constructor.jinja",
        include_str!("templates/service_js_constructor.jinja"),
    ),
    (
        "service_js_configurator.jinja",
        include_str!("templates/service_js_configurator.jinja"),
    ),
    (
        "service_js_direct_variant.jinja",
        include_str!("templates/service_js_direct_variant.jinja"),
    ),
    (
        "service_js_decorator_variant.jinja",
        include_str!("templates/service_js_decorator_variant.jinja"),
    ),
    ("ts_interface", include_str!("templates/ts_interface.jinja")),
    ("ts_inline_object", include_str!("templates/ts_inline_object.jinja")),
    ("ts_type_alias", include_str!("templates/ts_type_alias.jinja")),
    ("ts_custom_section", include_str!("templates/ts_custom_section.jinja")),
    (
        "ts_extern_value_type",
        include_str!("templates/ts_extern_value_type.jinja"),
    ),
    ("ts_bridged_setter", include_str!("templates/ts_bridged_setter.jinja")),
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
    let rendered = make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"));
    crate::core::keep_marker::strip_keep_markers(&rendered)
}

#[cfg(test)]
mod template_registration_tests {
    use super::TEMPLATES;
    use std::collections::HashSet;
    use std::path::Path;

    /// `render()` resolves names against `TEMPLATES`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into this array compiles fine
    /// (`include_str!` only runs for entries that are listed) and panics only once an
    /// emitter reaches it at generation time. Compare by content rather than by
    /// registered key: some backends register a file under a shortened or aliased name,
    /// which is fine, but every file's bytes must appear in `TEMPLATES` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/wasm/templates"));
        let registered_contents: HashSet<&str> = TEMPLATES.iter().map(|(_, content)| *content).collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATES: {unregistered:?}"
        );
    }

    fn collect_unregistered(
        root: &Path,
        dir: &Path,
        registered_contents: &HashSet<&str>,
        unregistered: &mut Vec<String>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read templates directory") {
            let entry = entry.expect("read templates directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect_unregistered(root, &path, registered_contents, unregistered);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jinja") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read template file");
            if !registered_contents.contains(content.as_str()) {
                let relative = path
                    .strip_prefix(root)
                    .expect("template path under templates root")
                    .to_str()
                    .expect("template path is valid UTF-8")
                    .replace('\\', "/");
                unregistered.push(relative);
            }
        }
    }
}
