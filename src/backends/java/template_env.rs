use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "java_file_header.jinja",
        include_str!("templates/java_file_header.jinja"),
    ),
    ("facade_class.jinja", include_str!("templates/facade_class.jinja")),
    ("facade_file.jinja", include_str!("templates/facade_file.jinja")),
    ("native_lib.jinja", include_str!("templates/native_lib.jinja")),
    ("visitor_bridge.jinja", include_str!("templates/visitor_bridge.jinja")),
    (
        "visitor_context_layout.jinja",
        include_str!("templates/visitor_context_layout.jinja"),
    ),
    (
        "visitor_context_decode.jinja",
        include_str!("templates/visitor_context_decode.jinja"),
    ),
    ("trait_interface.jinja", include_str!("templates/trait_interface.jinja")),
    ("trait_bridge.jinja", include_str!("templates/trait_bridge.jinja")),
    (
        "trait_adapter_bridge.jinja",
        include_str!("templates/trait_adapter_bridge.jinja"),
    ),
    (
        "convert_with_visitor.jinja",
        include_str!("templates/convert_with_visitor.jinja"),
    ),
    ("handle_method.jinja", include_str!("templates/handle_method.jinja")),
    (
        "helper_check_last_error.jinja",
        include_str!("templates/helper_check_last_error.jinja"),
    ),
    (
        "helper_last_error_exception.jinja",
        include_str!("templates/helper_last_error_exception.jinja"),
    ),
    (
        "helper_object_mapper.jinja",
        include_str!("templates/helper_object_mapper.jinja"),
    ),
    (
        "json_mapper_factory.jinja",
        include_str!("templates/json_mapper_factory.jinja"),
    ),
    ("javadoc_lines.jinja", include_str!("templates/javadoc_lines.jinja")),
    (
        "helper_read_bytes.jinja",
        include_str!("templates/helper_read_bytes.jinja"),
    ),
    (
        "helper_read_cstring.jinja",
        include_str!("templates/helper_read_cstring.jinja"),
    ),
    (
        "helper_read_json_list.jinja",
        include_str!("templates/helper_read_json_list.jinja"),
    ),
    (
        "helper_native_resources.jinja",
        include_str!("templates/helper_native_resources.jinja"),
    ),
    (
        "native_lib_visitor_handles.jinja",
        include_str!("templates/native_lib_visitor_handles.jinja"),
    ),
    (
        "native_lib_options_set_visitor_handle.jinja",
        include_str!("templates/native_lib_options_set_visitor_handle.jinja"),
    ),
    ("visitor_files.jinja", include_str!("templates/visitor_files.jinja")),
    ("exception_class.jinja", include_str!("templates/exception_class.jinja")),
    ("json_util.jinja", include_str!("templates/json_util.jinja")),
    (
        "infrastructure_exception.jinja",
        include_str!("templates/infrastructure_exception.jinja"),
    ),
    (
        "untagged_union_wrapper.jinja",
        include_str!("templates/untagged_union_wrapper.jinja"),
    ),
    (
        "sealed_union_serializer.jinja",
        include_str!("templates/sealed_union_serializer.jinja"),
    ),
    (
        "sealed_union_deserializer.jinja",
        include_str!("templates/sealed_union_deserializer.jinja"),
    ),
    ("visit_result.jinja", include_str!("templates/visit_result.jinja")),
    (
        "visitor_interface.jinja",
        include_str!("templates/visitor_interface.jinja"),
    ),
    (
        "bridge_unregister_method.jinja",
        include_str!("templates/bridge_unregister_method.jinja"),
    ),
    (
        "ffi_main_class_header.jinja",
        include_str!("templates/ffi_main_class_header.jinja"),
    ),
    (
        "ffi_main_class_footer.jinja",
        include_str!("templates/ffi_main_class_footer.jinja"),
    ),
    (
        "ffi_method_signature.jinja",
        include_str!("templates/ffi_method_signature.jinja"),
    ),
    (
        "ffi_try_finally_block_start.jinja",
        include_str!("templates/ffi_try_finally_block_start.jinja"),
    ),
    ("ffi_imports.jinja", include_str!("templates/ffi_imports.jinja")),
    ("ffi_null_check.jinja", include_str!("templates/ffi_null_check.jinja")),
    (
        "method_handle_nullable.jinja",
        include_str!("templates/method_handle_nullable.jinja"),
    ),
    (
        "method_handle_normal.jinja",
        include_str!("templates/method_handle_normal.jinja"),
    ),
    (
        "method_handle_to_json.jinja",
        include_str!("templates/method_handle_to_json.jinja"),
    ),
    (
        "method_handle_free.jinja",
        include_str!("templates/method_handle_free.jinja"),
    ),
    (
        "method_handle_free_bytes.jinja",
        include_str!("templates/method_handle_free_bytes.jinja"),
    ),
    (
        "bytes_result_call.jinja",
        include_str!("templates/bytes_result_call.jinja"),
    ),
    (
        "method_handle_from_json.jinja",
        include_str!("templates/method_handle_from_json.jinja"),
    ),
    (
        "method_handle_len.jinja",
        include_str!("templates/method_handle_len.jinja"),
    ),
    (
        "method_handle_register.jinja",
        include_str!("templates/method_handle_register.jinja"),
    ),
    (
        "method_handle_unregister.jinja",
        include_str!("templates/method_handle_unregister.jinja"),
    ),
    (
        "method_handle_clear.jinja",
        include_str!("templates/method_handle_clear.jinja"),
    ),
    (
        "bridge_clear_method.jinja",
        include_str!("templates/bridge_clear_method.jinja"),
    ),
    ("marshal_string.jinja", include_str!("templates/marshal_string.jinja")),
    ("marshal_path.jinja", include_str!("templates/marshal_path.jinja")),
    (
        "marshal_opaque_handle.jinja",
        include_str!("templates/marshal_opaque_handle.jinja"),
    ),
    (
        "marshal_named_type.jinja",
        include_str!("templates/marshal_named_type.jinja"),
    ),
    ("marshal_bytes.jinja", include_str!("templates/marshal_bytes.jinja")),
    (
        "marshal_optional_string.jinja",
        include_str!("templates/marshal_optional_string.jinja"),
    ),
    (
        "marshal_optional_path.jinja",
        include_str!("templates/marshal_optional_path.jinja"),
    ),
    (
        "marshal_optional_bytes.jinja",
        include_str!("templates/marshal_optional_bytes.jinja"),
    ),
    (
        "marshal_optional_opaque_handle.jinja",
        include_str!("templates/marshal_optional_opaque_handle.jinja"),
    ),
    (
        "marshal_optional_named_type.jinja",
        include_str!("templates/marshal_optional_named_type.jinja"),
    ),
    (
        "marshal_optional_primitive.jinja",
        include_str!("templates/marshal_optional_primitive.jinja"),
    ),
    ("marshal_vec_map.jinja", include_str!("templates/marshal_vec_map.jinja")),
    (
        "gen_helper_methods_header.jinja",
        include_str!("templates/gen_helper_methods_header.jinja"),
    ),
    (
        "ffi_typed_rethrow_catch.jinja",
        include_str!("templates/ffi_typed_rethrow_catch.jinja"),
    ),
    (
        "ffi_visitor_operation_open.jinja",
        include_str!("templates/ffi_visitor_operation_open.jinja"),
    ),
    (
        "ffi_alef_error_comment.jinja",
        include_str!("templates/ffi_alef_error_comment.jinja"),
    ),
    (
        "ffi_return_new_instance.jinja",
        include_str!("templates/ffi_return_new_instance.jinja"),
    ),
    (
        "ffi_invoke_json_ptr.jinja",
        include_str!("templates/ffi_invoke_json_ptr.jinja"),
    ),
    (
        "ffi_writeback_return.jinja",
        include_str!("templates/ffi_writeback_return.jinja"),
    ),
    (
        "ffi_return_read_json_list_optional.jinja",
        include_str!("templates/ffi_return_read_json_list_optional.jinja"),
    ),
    (
        "ffi_invoke_primitive_result.jinja",
        include_str!("templates/ffi_invoke_primitive_result.jinja"),
    ),
    (
        "ffi_async_method_signature.jinja",
        include_str!("templates/ffi_async_method_signature.jinja"),
    ),
    (
        "convert_with_visitor_signature.jinja",
        include_str!("templates/convert_with_visitor_signature.jinja"),
    ),
    (
        "ffi_visitor_create.jinja",
        include_str!("templates/ffi_visitor_create.jinja"),
    ),
    (
        "ffi_throw_on_null.jinja",
        include_str!("templates/ffi_throw_on_null.jinja"),
    ),
    (
        "ffi_options_set_visitor.jinja",
        include_str!("templates/ffi_options_set_visitor.jinja"),
    ),
    (
        "ffi_options_free_conditional.jinja",
        include_str!("templates/ffi_options_free_conditional.jinja"),
    ),
    (
        "ffi_result_to_json.jinja",
        include_str!("templates/ffi_result_to_json.jinja"),
    ),
    (
        "ffi_visitor_cleanup.jinja",
        include_str!("templates/ffi_visitor_cleanup.jinja"),
    ),
    (
        "ffi_options_free.jinja",
        include_str!("templates/ffi_options_free.jinja"),
    ),
    (
        "ffi_result_ptr_call.jinja",
        include_str!("templates/ffi_result_ptr_call.jinja"),
    ),
    ("ffi_invoke_void.jinja", include_str!("templates/ffi_invoke_void.jinja")),
    ("ffi_return_expr.jinja", include_str!("templates/ffi_return_expr.jinja")),
    (
        "ffi_return_optional_expr.jinja",
        include_str!("templates/ffi_return_optional_expr.jinja"),
    ),
    (
        "ffi_return_mapper_read.jinja",
        include_str!("templates/ffi_return_mapper_read.jinja"),
    ),
    (
        "ffi_return_mapper_read_optional.jinja",
        include_str!("templates/ffi_return_mapper_read_optional.jinja"),
    ),
    (
        "ffi_return_read_json_list_plain.jinja",
        include_str!("templates/ffi_return_read_json_list_plain.jinja"),
    ),
    (
        "ffi_return_new_handle.jinja",
        include_str!("templates/ffi_return_new_handle.jinja"),
    ),
    (
        "ffi_return_primitive_result.jinja",
        include_str!("templates/ffi_return_primitive_result.jinja"),
    ),
    (
        "stream_method_null_check.jinja",
        include_str!("templates/stream_method_null_check.jinja"),
    ),
    (
        "stream_method_string_param.jinja",
        include_str!("templates/stream_method_string_param.jinja"),
    ),
    (
        "stream_method_optional_string_param.jinja",
        include_str!("templates/stream_method_optional_string_param.jinja"),
    ),
    (
        "stream_method_optional_named_param.jinja",
        include_str!("templates/stream_method_optional_named_param.jinja"),
    ),
    (
        "stream_method_enum_param.jinja",
        include_str!("templates/stream_method_enum_param.jinja"),
    ),
    (
        "stream_method_named_param.jinja",
        include_str!("templates/stream_method_named_param.jinja"),
    ),
    (
        "stream_method_unsupported_param.jinja",
        include_str!("templates/stream_method_unsupported_param.jinja"),
    ),
    (
        "stream_method_bytes_result.jinja",
        include_str!("templates/stream_method_bytes_result.jinja"),
    ),
    (
        "stream_method_named_result.jinja",
        include_str!("templates/stream_method_named_result.jinja"),
    ),
    (
        "stream_method_opaque_handle_result.jinja",
        include_str!("templates/stream_method_opaque_handle_result.jinja"),
    ),
    (
        "stream_method_string_result.jinja",
        include_str!("templates/stream_method_string_result.jinja"),
    ),
    (
        "stream_method_optional_string_result.jinja",
        include_str!("templates/stream_method_optional_string_result.jinja"),
    ),
    (
        "stream_method_primitive_result.jinja",
        include_str!("templates/stream_method_primitive_result.jinja"),
    ),
    (
        "stream_method_optional_primitive_result.jinja",
        include_str!("templates/stream_method_optional_primitive_result.jinja"),
    ),
    (
        "stream_method_unit_result.jinja",
        include_str!("templates/stream_method_unit_result.jinja"),
    ),
    (
        "stream_method_unsupported_return.jinja",
        include_str!("templates/stream_method_unsupported_return.jinja"),
    ),
    (
        "streaming_iterator_method.jinja",
        include_str!("templates/streaming_iterator_method.jinja"),
    ),
    (
        "streaming_helpers.jinja",
        include_str!("templates/streaming_helpers.jinja"),
    ),
    (
        "stream_method_catch.jinja",
        include_str!("templates/stream_method_catch.jinja"),
    ),
    (
        "stream_method_catch_unchecked.jinja",
        include_str!("templates/stream_method_catch_unchecked.jinja"),
    ),
    (
        "registration_variant.java.jinja",
        include_str!("templates/registration_variant.java.jinja"),
    ),
    (
        "service_binding_doc_entrypoint.jinja",
        include_str!("templates/service_binding_doc_entrypoint.jinja"),
    ),
    (
        "service_binding_doc_registration.jinja",
        include_str!("templates/service_binding_doc_registration.jinja"),
    ),
    (
        "service_class_header.jinja",
        include_str!("templates/service_class_header.jinja"),
    ),
    (
        "service_constructor.jinja",
        include_str!("templates/service_constructor.jinja"),
    ),
    (
        "service_registration_method.jinja",
        include_str!("templates/service_registration_method.jinja"),
    ),
    (
        "service_entrypoint_method.jinja",
        include_str!("templates/service_entrypoint_method.jinja"),
    ),
    ("service_close.jinja", include_str!("templates/service_close.jinja")),
    (
        "service_callable_interface.jinja",
        include_str!("templates/service_callable_interface.jinja"),
    ),
    (
        "service_metadata_param_doc.jinja",
        include_str!("templates/service_metadata_param_doc.jinja"),
    ),
    (
        "service_metadata_signature_param.jinja",
        include_str!("templates/service_metadata_signature_param.jinja"),
    ),
    (
        "opaque_resource_declaration.jinja",
        include_str!("templates/opaque_resource_declaration.jinja"),
    ),
    (
        "opaque_cleanup_lease.jinja",
        include_str!("templates/opaque_cleanup_lease.jinja"),
    ),
    (
        "opaque_cleanup_handle.jinja",
        include_str!("templates/opaque_cleanup_handle.jinja"),
    ),
    (
        "opaque_unsupported_param.jinja",
        include_str!("templates/opaque_unsupported_param.jinja"),
    ),
    (
        "opaque_param_lease_assignment.jinja",
        include_str!("templates/opaque_param_lease_assignment.jinja"),
    ),
    (
        "record_declaration.jinja",
        include_str!("templates/record_declaration.jinja"),
    ),
    (
        "record_builder_factory.jinja",
        include_str!("templates/record_builder_factory.jinja"),
    ),
    (
        "record_compact_constructor.jinja",
        include_str!("templates/record_compact_constructor.jinja"),
    ),
    (
        "simple_enum_class.jinja",
        include_str!("templates/simple_enum_class.jinja"),
    ),
    (
        "opaque_handle_header.jinja",
        include_str!("templates/opaque_handle_header.jinja"),
    ),
    (
        "opaque_handle_close.jinja",
        include_str!("templates/opaque_handle_close.jinja"),
    ),
    (
        "static_factory_return_handle.jinja",
        include_str!("templates/static_factory_return_handle.jinja"),
    ),
    (
        "byte_array_serializer.jinja",
        include_str!("templates/byte_array_serializer.jinja"),
    ),
    (
        "duration_millis_serializer.jinja",
        include_str!("templates/duration_millis_serializer.jinja"),
    ),
    (
        "duration_millis_deserializer.jinja",
        include_str!("templates/duration_millis_deserializer.jinja"),
    ),
    (
        "method_handle_presence.jinja",
        include_str!("templates/method_handle_presence.jinja"),
    ),
    (
        "presence_capture.jinja",
        include_str!("templates/presence_capture.jinja"),
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/java/templates"));
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
