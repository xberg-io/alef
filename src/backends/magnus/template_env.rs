use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "opaque_struct.rs.jinja",
        include_str!("templates/opaque_struct.rs.jinja"),
    ),
    ("struct_def.rs.jinja", include_str!("templates/struct_def.rs.jinja")),
    ("enum_def.rs.jinja", include_str!("templates/enum_def.rs.jinja")),
    ("enum_magnus.rs.jinja", include_str!("templates/enum_magnus.rs.jinja")),
    (
        "enum_variant_constructor.rs.jinja",
        include_str!("templates/enum_variant_constructor.rs.jinja"),
    ),
    (
        "rbs_enum_variant_constructor.jinja",
        include_str!("templates/rbs_enum_variant_constructor.jinja"),
    ),
    (
        "rbs_enum_variant_constructor_param.jinja",
        include_str!("templates/rbs_enum_variant_constructor_param.jinja"),
    ),
    (
        "visitor_bridge_struct.rs.jinja",
        include_str!("templates/visitor_bridge_struct.rs.jinja"),
    ),
    (
        "visitor_method.rs.jinja",
        include_str!("templates/visitor_method.rs.jinja"),
    ),
    (
        "bridge_struct_impl.rs.jinja",
        include_str!("templates/bridge_struct_impl.rs.jinja"),
    ),
    (
        "visitor_bridge_wrapper.rs.jinja",
        include_str!("templates/visitor_bridge_wrapper.rs.jinja"),
    ),
    (
        "visitor_bridge.rs.jinja",
        include_str!("templates/visitor_bridge.rs.jinja"),
    ),
    (
        "visitor_method.rs.jinja",
        include_str!("templates/visitor_method.rs.jinja"),
    ),
    (
        "main_rb_wrapper.rb.jinja",
        include_str!("templates/main_rb_wrapper.rb.jinja"),
    ),
    (
        "native_rb_wrapper.rb.jinja",
        include_str!("templates/native_rb_wrapper.rb.jinja"),
    ),
    (
        "version_rb_wrapper.rb.jinja",
        include_str!("templates/version_rb_wrapper.rb.jinja"),
    ),
    (
        "sync_method_body.rs.jinja",
        include_str!("templates/sync_method_body.rs.jinja"),
    ),
    (
        "trait_bridge_async_method_body.rs.jinja",
        include_str!("templates/trait_bridge_async_method_body.rs.jinja"),
    ),
    (
        "trait_bridge_constructor.rs.jinja",
        include_str!("templates/trait_bridge_constructor.rs.jinja"),
    ),
    (
        "trait_bridge_runtime_dispatcher.rs.jinja",
        include_str!("templates/trait_bridge_runtime_dispatcher.rs.jinja"),
    ),
    (
        "trait_bridge_registration_fn.rs.jinja",
        include_str!("templates/trait_bridge_registration_fn.rs.jinja"),
    ),
    (
        "trait_bridge_return_conversion.rs.jinja",
        include_str!("templates/trait_bridge_return_conversion.rs.jinja"),
    ),
    (
        "function_scan_args_call.rs.jinja",
        include_str!("templates/function_scan_args_call.rs.jinja"),
    ),
    (
        "function_scan_args_destructure.rs.jinja",
        include_str!("templates/function_scan_args_destructure.rs.jinja"),
    ),
    (
        "function_optional_string_scan_arg.rs.jinja",
        include_str!("templates/function_optional_string_scan_arg.rs.jinja"),
    ),
    (
        "function_named_binding.rs.jinja",
        include_str!("templates/function_named_binding.rs.jinja"),
    ),
    (
        "function_async_body.rs.jinja",
        include_str!("templates/function_async_body.rs.jinja"),
    ),
    (
        "function_result_body.rs.jinja",
        include_str!("templates/function_result_body.rs.jinja"),
    ),
    (
        "function_variadic_ok_body.rs.jinja",
        include_str!("templates/function_variadic_ok_body.rs.jinja"),
    ),
    (
        "function_wrapper.rs.jinja",
        include_str!("templates/function_wrapper.rs.jinja"),
    ),
    (
        "function_serde_named_binding.rs.jinja",
        include_str!("templates/function_serde_named_binding.rs.jinja"),
    ),
    (
        "function_vec_refs_binding.rs.jinja",
        include_str!("templates/function_vec_refs_binding.rs.jinja"),
    ),
    ("rbs_doc_block.jinja", include_str!("templates/rbs_doc_block.jinja")),
    (
        "function_sanitized_vec_binding.rs.jinja",
        include_str!("templates/function_sanitized_vec_binding.rs.jinja"),
    ),
    (
        "function_named_vec_binding.rs.jinja",
        include_str!("templates/function_named_vec_binding.rs.jinja"),
    ),
    (
        "function_unimplemented_error.rs.jinja",
        include_str!("templates/function_unimplemented_error.rs.jinja"),
    ),
    (
        "function_unimplemented_panic.rs.jinja",
        include_str!("templates/function_unimplemented_panic.rs.jinja"),
    ),
    (
        "module_define.rs.jinja",
        include_str!("templates/module_define.rs.jinja"),
    ),
    (
        "module_class_define.rs.jinja",
        include_str!("templates/module_class_define.rs.jinja"),
    ),
    (
        "module_function_register.rs.jinja",
        include_str!("templates/module_function_register.rs.jinja"),
    ),
    (
        "cfg_attribute.rs.jinja",
        include_str!("templates/cfg_attribute.rs.jinja"),
    ),
    (
        "module_class_singleton_method_register.rs.jinja",
        include_str!("templates/module_class_singleton_method_register.rs.jinja"),
    ),
    (
        "module_class_method_register.rs.jinja",
        include_str!("templates/module_class_method_register.rs.jinja"),
    ),
    (
        "module_class_include_enumerable.rs.jinja",
        include_str!("templates/module_class_include_enumerable.rs.jinja"),
    ),
    (
        "service_rb_header.rb.jinja",
        include_str!("templates/service_rb_header.rb.jinja"),
    ),
    (
        "service_rb_class_header.rb.jinja",
        include_str!("templates/service_rb_class_header.rb.jinja"),
    ),
    (
        "service_rb_initialize.rb.jinja",
        include_str!("templates/service_rb_initialize.rb.jinja"),
    ),
    (
        "service_rb_configurator.rb.jinja",
        include_str!("templates/service_rb_configurator.rb.jinja"),
    ),
    (
        "service_rb_entrypoint.rb.jinja",
        include_str!("templates/service_rb_entrypoint.rb.jinja"),
    ),
    (
        "service_rb_registration_method.rb.jinja",
        include_str!("templates/service_rb_registration_method.rb.jinja"),
    ),
    (
        "service_rb_direct_registration_method.rb.jinja",
        include_str!("templates/service_rb_direct_registration_method.rb.jinja"),
    ),
    (
        "service_rb_registration_variant.rb.jinja",
        include_str!("templates/service_rb_registration_variant.rb.jinja"),
    ),
    (
        "service_rs_header.rs.jinja",
        include_str!("templates/service_rs_header.rs.jinja"),
    ),
    (
        "service_rs_handler_bridge.rs.jinja",
        include_str!("templates/service_rs_handler_bridge.rs.jinja"),
    ),
    (
        "service_rs_ruby_proc_gvl_helpers.rs.jinja",
        include_str!("templates/service_rs_ruby_proc_gvl_helpers.rs.jinja"),
    ),
    (
        "service_rs_meta_array_extract.rs.jinja",
        include_str!("templates/service_rs_meta_array_extract.rs.jinja"),
    ),
    (
        "service_rs_metadata_extract_entry.rs.jinja",
        include_str!("templates/service_rs_metadata_extract_entry.rs.jinja"),
    ),
    (
        "service_rs_metadata_extract_try_convert.rs.jinja",
        include_str!("templates/service_rs_metadata_extract_try_convert.rs.jinja"),
    ),
    (
        "service_rs_run_function_header.rs.jinja",
        include_str!("templates/service_rs_run_function_header.rs.jinja"),
    ),
    (
        "service_rs_registration_match_arm_header.rs.jinja",
        include_str!("templates/service_rs_registration_match_arm_header.rs.jinja"),
    ),
    (
        "service_rs_variant_match_arm_header.rs.jinja",
        include_str!("templates/service_rs_variant_match_arm_header.rs.jinja"),
    ),
    (
        "service_rs_owner_call.rs.jinja",
        include_str!("templates/service_rs_owner_call.rs.jinja"),
    ),
    (
        "service_rs_wrapper_owner_call.rs.jinja",
        include_str!("templates/service_rs_wrapper_owner_call.rs.jinja"),
    ),
    (
        "service_rs_run_function_footer.rs.jinja",
        include_str!("templates/service_rs_run_function_footer.rs.jinja"),
    ),
    (
        "service_rs_async_entrypoint_call.rs.jinja",
        include_str!("templates/service_rs_async_entrypoint_call.rs.jinja"),
    ),
    (
        "tagged_enum_marker_module.rb.jinja",
        include_str!("templates/tagged_enum_marker_module.rb.jinja"),
    ),
    (
        "tagged_enum_marker_doc.rb.jinja",
        include_str!("templates/tagged_enum_marker_doc.rb.jinja"),
    ),
    (
        "tagged_enum_dispatch_arm.rb.jinja",
        include_str!("templates/tagged_enum_dispatch_arm.rb.jinja"),
    ),
    (
        "tagged_enum_variant_class.rb.jinja",
        include_str!("templates/tagged_enum_variant_class.rb.jinja"),
    ),
    (
        "tagged_enum_variant_doc.rb.jinja",
        include_str!("templates/tagged_enum_variant_doc.rb.jinja"),
    ),
    (
        "tagged_enum_field_accessor.rb.jinja",
        include_str!("templates/tagged_enum_field_accessor.rb.jinja"),
    ),
    (
        "tagged_enum_predicate_method.rb.jinja",
        include_str!("templates/tagged_enum_predicate_method.rb.jinja"),
    ),
    (
        "method_named_ref_preamble.rs.jinja",
        include_str!("templates/method_named_ref_preamble.rs.jinja"),
    ),
    (
        "method_optional_named_ref_preamble.rs.jinja",
        include_str!("templates/method_optional_named_ref_preamble.rs.jinja"),
    ),
    (
        "method_string_vec_ref_preamble.rs.jinja",
        include_str!("templates/method_string_vec_ref_preamble.rs.jinja"),
    ),
    (
        "method_bytes_ref_preamble.rs.jinja",
        include_str!("templates/method_bytes_ref_preamble.rs.jinja"),
    ),
    (
        "method_optional_string_vec_ref_preamble.rs.jinja",
        include_str!("templates/method_optional_string_vec_ref_preamble.rs.jinja"),
    ),
    (
        "method_named_vec_binding.rs.jinja",
        include_str!("templates/method_named_vec_binding.rs.jinja"),
    ),
    (
        "method_optional_named_vec_binding.rs.jinja",
        include_str!("templates/method_optional_named_vec_binding.rs.jinja"),
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/magnus/templates"));
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
