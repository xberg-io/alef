use minijinja::{Environment, UndefinedBehavior};

static TEMPLATES: &[(&str, &str)] = &[
    (
        "converted_field_assignment.jinja",
        include_str!("templates/converted_field_assignment.jinja"),
    ),
    (
        "adjacent_enum_namespace.rs.jinja",
        include_str!("templates/adjacent_enum_namespace.rs.jinja"),
    ),
    (
        "unregistration_fn.jinja",
        include_str!("templates/unregistration_fn.jinja"),
    ),
    ("clear_fn.jinja", include_str!("templates/clear_fn.jinja")),
    ("registration_fn.jinja", include_str!("templates/registration_fn.jinja")),
    (
        "sync_method_unit_return.jinja",
        include_str!("templates/sync_method_unit_return.jinja"),
    ),
    (
        "trait_bridge_constructor.jinja",
        include_str!("templates/trait_bridge_constructor.jinja"),
    ),
    ("bridge_function.jinja", include_str!("templates/bridge_function.jinja")),
    (
        "gen_tagged_enum_binding_to_core.jinja",
        include_str!("templates/gen_tagged_enum_binding_to_core.jinja"),
    ),
    (
        "gen_tagged_enum_core_to_binding.jinja",
        include_str!("templates/gen_tagged_enum_core_to_binding.jinja"),
    ),
    ("visitor_bridge.jinja", include_str!("templates/visitor_bridge.jinja")),
    ("visitor_method.jinja", include_str!("templates/visitor_method.jinja")),
    (
        "visitor_context_arg_expr.jinja",
        include_str!("templates/visitor_context_arg_expr.jinja"),
    ),
    (
        "clone_impl_header.jinja",
        include_str!("templates/clone_impl_header.jinja"),
    ),
    (
        "clone_impl_field.jinja",
        include_str!("templates/clone_impl_field.jinja"),
    ),
    (
        "vec_f32_conversion_binding.jinja",
        include_str!("templates/vec_f32_conversion_binding.jinja"),
    ),
    (
        "buffer_conversion_binding.jinja",
        include_str!("templates/buffer_conversion_binding.jinja"),
    ),
    (
        "buffer_conversion_binding_optional.jinja",
        include_str!("templates/buffer_conversion_binding_optional.jinja"),
    ),
    (
        "trait_bridge_fn_wrapper.jinja",
        include_str!("templates/trait_bridge_fn_wrapper.jinja"),
    ),
    (
        "bridge_optional_wrap.jinja",
        include_str!("templates/bridge_optional_wrap.jinja"),
    ),
    (
        "bridge_required_wrap.jinja",
        include_str!("templates/bridge_required_wrap.jinja"),
    ),
    (
        "bridge_function_body_error.jinja",
        include_str!("templates/bridge_function_body_error.jinja"),
    ),
    (
        "bridge_function_body_error_mapped.jinja",
        include_str!("templates/bridge_function_body_error_mapped.jinja"),
    ),
    (
        "bridge_function_body_plain.jinja",
        include_str!("templates/bridge_function_body_plain.jinja"),
    ),
    (
        "named_core_binding_optional.jinja",
        include_str!("templates/named_core_binding_optional.jinja"),
    ),
    (
        "named_core_binding_required.jinja",
        include_str!("templates/named_core_binding_required.jinja"),
    ),
    (
        "function_wrapper.jinja",
        include_str!("templates/function_wrapper.jinja"),
    ),
    (
        "options_visitor_extract_optional.jinja",
        include_str!("templates/options_visitor_extract_optional.jinja"),
    ),
    (
        "options_visitor_extract_required.jinja",
        include_str!("templates/options_visitor_extract_required.jinja"),
    ),
    (
        "options_field_bridge_body.jinja",
        include_str!("templates/options_field_bridge_body.jinja"),
    ),
    (
        "service_ts_preamble.jinja",
        include_str!("templates/service_ts_preamble.jinja"),
    ),
    (
        "service_ts_class_header.jinja",
        include_str!("templates/service_ts_class_header.jinja"),
    ),
    (
        "service_ts_static_new.jinja",
        include_str!("templates/service_ts_static_new.jinja"),
    ),
    (
        "service_ts_constructor.jinja",
        include_str!("templates/service_ts_constructor.jinja"),
    ),
    (
        "service_ts_config_field.jinja",
        include_str!("templates/service_ts_config_field.jinja"),
    ),
    (
        "service_ts_configurator.jinja",
        include_str!("templates/service_ts_configurator.jinja"),
    ),
    (
        "service_ts_configurator_store.jinja",
        include_str!("templates/service_ts_configurator_store.jinja"),
    ),
    (
        "service_ts_configurator_config_forward.jinja",
        include_str!("templates/service_ts_configurator_config_forward.jinja"),
    ),
    (
        "service_ts_entrypoint_run.jinja",
        include_str!("templates/service_ts_entrypoint_run.jinja"),
    ),
    (
        "service_ts_entrypoint_finalize.jinja",
        include_str!("templates/service_ts_entrypoint_finalize.jinja"),
    ),
    (
        "service_ts_registration_method.jinja",
        include_str!("templates/service_ts_registration_method.jinja"),
    ),
    (
        "service_ts_registration_direct_method.jinja",
        include_str!("templates/service_ts_registration_direct_method.jinja"),
    ),
    (
        "service_ts_variant_direct.jinja",
        include_str!("templates/service_ts_variant_direct.jinja"),
    ),
    (
        "service_ts_variant_decorator.jinja",
        include_str!("templates/service_ts_variant_decorator.jinja"),
    ),
    (
        "service_ts_variant_hybrid.jinja",
        include_str!("templates/service_ts_variant_hybrid.jinja"),
    ),
    (
        "service_rs_preamble.jinja",
        include_str!("templates/service_rs_preamble.jinja"),
    ),
    (
        "service_rs_impl_block.jinja",
        include_str!("templates/service_rs_impl_block.jinja"),
    ),
    (
        "service_rs_handler_bridge_header.jinja",
        include_str!("templates/service_rs_handler_bridge_header.jinja"),
    ),
    (
        "service_rs_handler_bridge_impl.jinja",
        include_str!("templates/service_rs_handler_bridge_impl.jinja"),
    ),
    (
        "service_rs_run_function_header.jinja",
        include_str!("templates/service_rs_run_function_header.jinja"),
    ),
    (
        "service_rs_owner_ctor.jinja",
        include_str!("templates/service_rs_owner_ctor.jinja"),
    ),
    (
        "service_rs_registration_arm_header.jinja",
        include_str!("templates/service_rs_registration_arm_header.jinja"),
    ),
    (
        "service_rs_metadata_binding.jinja",
        include_str!("templates/service_rs_metadata_binding.jinja"),
    ),
    (
        "service_rs_registration_call.jinja",
        include_str!("templates/service_rs_registration_call.jinja"),
    ),
    (
        "service_rs_variant_method_header.jinja",
        include_str!("templates/service_rs_variant_method_header.jinja"),
    ),
    (
        "service_rs_base_registration_method_header.jinja",
        include_str!("templates/service_rs_base_registration_method_header.jinja"),
    ),
    (
        "service_rs_wrapper_ctor.jinja",
        include_str!("templates/service_rs_wrapper_ctor.jinja"),
    ),
    (
        "service_rs_handler_arc.jinja",
        include_str!("templates/service_rs_handler_arc.jinja"),
    ),
    (
        "service_rs_base_registration_call.jinja",
        include_str!("templates/service_rs_base_registration_call.jinja"),
    ),
    (
        "service_rs_inner_accessor.jinja",
        include_str!("templates/service_rs_inner_accessor.jinja"),
    ),
    (
        "service_rs_unit_ok_footer.jinja",
        include_str!("templates/service_rs_unit_ok_footer.jinja"),
    ),
    (
        "service_rs_entrypoint_method_header.jinja",
        include_str!("templates/service_rs_entrypoint_method_header.jinja"),
    ),
    (
        "service_rs_take_owner.jinja",
        include_str!("templates/service_rs_take_owner.jinja"),
    ),
    (
        "service_rs_entrypoint_call.jinja",
        include_str!("templates/service_rs_entrypoint_call.jinja"),
    ),
    (
        "service_rs_configurator_method_header.jinja",
        include_str!("templates/service_rs_configurator_method_header.jinja"),
    ),
    (
        "service_rs_configurator_parse.jinja",
        include_str!("templates/service_rs_configurator_parse.jinja"),
    ),
    (
        "service_rs_configurator_apply.jinja",
        include_str!("templates/service_rs_configurator_apply.jinja"),
    ),
    (
        "service_rs_configurator_replace.jinja",
        include_str!("templates/service_rs_configurator_replace.jinja"),
    ),
    (
        "capsule_type_tag_constant.jinja",
        include_str!("templates/capsule_type_tag_constant.jinja"),
    ),
    (
        "struct_static_method_wrapper.jinja",
        include_str!("templates/struct_static_method_wrapper.jinja"),
    ),
    (
        "struct_wither_serde_body.jinja",
        include_str!("templates/struct_wither_serde_body.jinja"),
    ),
    (
        "napi_bridge_struct.jinja",
        include_str!("templates/napi_bridge_struct.jinja"),
    ),
    (
        "config_opaque_wrapper.rs.jinja",
        include_str!("templates/config_opaque_wrapper.rs.jinja"),
    ),
    (
        "enum_default_impl_cascade.jinja",
        include_str!("templates/enum_default_impl_cascade.jinja"),
    ),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    // Strict: referencing a context key that was never passed is a render-time error
    // instead of minijinja's default Lenient behavior of silently treating it as falsy/empty.
    // Lenient mode let `{% if missing_key %}` branches take the `else` arm unnoticed: the async
    // trait-bridge templates once branched on `wrapper`/`has_error`/`has_default_impl` keys the
    // generator never passed, and nothing caught it.
    env.set_undefined_behavior(UndefinedBehavior::Strict);
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/napi/templates"));
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
