use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("type_header.jinja", include_str!("templates/type_header.jinja")),
    ("type_field.jinja", include_str!("templates/type_field.jinja")),
    (
        "enum_unit_header.jinja",
        include_str!("templates/enum_unit_header.jinja"),
    ),
    (
        "enum_unit_variant.jinja",
        include_str!("templates/enum_unit_variant.jinja"),
    ),
    (
        "enum_tagged_header.jinja",
        include_str!("templates/enum_tagged_header.jinja"),
    ),
    (
        "enum_variant_void.jinja",
        include_str!("templates/enum_variant_void.jinja"),
    ),
    (
        "enum_variant_single.jinja",
        include_str!("templates/enum_variant_single.jinja"),
    ),
    (
        "enum_variant_struct_header.jinja",
        include_str!("templates/enum_variant_struct_header.jinja"),
    ),
    (
        "enum_variant_struct_field.jinja",
        include_str!("templates/enum_variant_struct_field.jinja"),
    ),
    (
        "error_set_header.jinja",
        include_str!("templates/error_set_header.jinja"),
    ),
    ("error_doc_line.jinja", include_str!("templates/error_doc_line.jinja")),
    ("error_doc_block.jinja", include_str!("templates/error_doc_block.jinja")),
    (
        "error_set_variant.jinja",
        include_str!("templates/error_set_variant.jinja"),
    ),
    (
        "function_signature.jinja",
        include_str!("templates/function_signature.jinja"),
    ),
    (
        "function_call_unit.jinja",
        include_str!("templates/function_call_unit.jinja"),
    ),
    (
        "function_call_result.jinja",
        include_str!("templates/function_call_result.jinja"),
    ),
    (
        "function_error_check.jinja",
        include_str!("templates/function_error_check.jinja"),
    ),
    (
        "function_error_return.jinja",
        include_str!("templates/function_error_return.jinja"),
    ),
    (
        "function_free_bytes.jinja",
        include_str!("templates/function_free_bytes.jinja"),
    ),
    (
        "function_result_len.jinja",
        include_str!("templates/function_result_len.jinja"),
    ),
    ("function_return.jinja", include_str!("templates/function_return.jinja")),
    (
        "param_string_alloc.jinja",
        include_str!("templates/param_string_alloc.jinja"),
    ),
    (
        "param_string_line1.jinja",
        include_str!("templates/param_string_line1.jinja"),
    ),
    (
        "param_string_line2.jinja",
        include_str!("templates/param_string_line2.jinja"),
    ),
    (
        "param_optional_string_alloc.jinja",
        include_str!("templates/param_optional_string_alloc.jinja"),
    ),
    (
        "param_opaque_config_from_json.jinja",
        include_str!("templates/param_opaque_config_from_json.jinja"),
    ),
    (
        "param_struct_handle.jinja",
        include_str!("templates/param_struct_handle.jinja"),
    ),
    (
        "param_optional_struct_handle.jinja",
        include_str!("templates/param_optional_struct_handle.jinja"),
    ),
    ("param_free.jinja", include_str!("templates/param_free.jinja")),
    (
        "result_presence_gate.jinja",
        include_str!("templates/result_presence_gate.jinja"),
    ),
    (
        "result_presence_error_check.jinja",
        include_str!("templates/result_presence_error_check.jinja"),
    ),
    (
        "param_optional_free.jinja",
        include_str!("templates/param_optional_free.jinja"),
    ),
    (
        "param_struct_handle_free.jinja",
        include_str!("templates/param_struct_handle_free.jinja"),
    ),
    (
        "return_unwrap_slice.jinja",
        include_str!("templates/return_unwrap_slice.jinja"),
    ),
    (
        "return_unwrap_free.jinja",
        include_str!("templates/return_unwrap_free.jinja"),
    ),
    (
        "return_named_json_block.jinja",
        include_str!("templates/return_named_json_block.jinja"),
    ),
    (
        "return_optional_owned_bytes_block.jinja",
        include_str!("templates/return_optional_owned_bytes_block.jinja"),
    ),
    (
        "return_owned_bytes_block.jinja",
        include_str!("templates/return_owned_bytes_block.jinja"),
    ),
    ("c_import.jinja", include_str!("templates/c_import.jinja")),
    (
        "helper_free_string_doc1.jinja",
        include_str!("templates/helper_free_string_doc1.jinja"),
    ),
    (
        "helper_free_string_doc2.jinja",
        include_str!("templates/helper_free_string_doc2.jinja"),
    ),
    (
        "helper_free_string_doc3.jinja",
        include_str!("templates/helper_free_string_doc3.jinja"),
    ),
    (
        "helper_free_string_body.jinja",
        include_str!("templates/helper_free_string_body.jinja"),
    ),
    (
        "helper_last_error_code.jinja",
        include_str!("templates/helper_last_error_code.jinja"),
    ),
    (
        "helper_last_error_ctx.jinja",
        include_str!("templates/helper_last_error_ctx.jinja"),
    ),
    (
        "vtable_header_doc.jinja",
        include_str!("templates/vtable_header_doc.jinja"),
    ),
    (
        "vtable_impl_method.jinja",
        include_str!("templates/vtable_impl_method.jinja"),
    ),
    (
        "vtable_make_fn_header.jinja",
        include_str!("templates/vtable_make_fn_header.jinja"),
    ),
    (
        "vtable_instance_field.jinja",
        include_str!("templates/vtable_instance_field.jinja"),
    ),
    (
        "vtable_field_name_fn.jinja",
        include_str!("templates/vtable_field_name_fn.jinja"),
    ),
    (
        "vtable_field_version_fn.jinja",
        include_str!("templates/vtable_field_version_fn.jinja"),
    ),
    (
        "vtable_field_initialize_fn.jinja",
        include_str!("templates/vtable_field_initialize_fn.jinja"),
    ),
    (
        "vtable_field_shutdown_fn.jinja",
        include_str!("templates/vtable_field_shutdown_fn.jinja"),
    ),
    (
        "vtable_method_field.jinja",
        include_str!("templates/vtable_method_field.jinja"),
    ),
    (
        "vtable_free_user_data.jinja",
        include_str!("templates/vtable_free_user_data.jinja"),
    ),
    (
        "thunk_struct_header.jinja",
        include_str!("templates/thunk_struct_header.jinja"),
    ),
    (
        "thunk_fn_signature.jinja",
        include_str!("templates/thunk_fn_signature.jinja"),
    ),
    (
        "thunk_bytes_slice.jinja",
        include_str!("templates/thunk_bytes_slice.jinja"),
    ),
    (
        "thunk_discard_bytes_len.jinja",
        include_str!("templates/thunk_discard_bytes_len.jinja"),
    ),
    (
        "thunk_result_assign.jinja",
        include_str!("templates/thunk_result_assign.jinja"),
    ),
    (
        "thunk_if_fallible.jinja",
        include_str!("templates/thunk_if_fallible.jinja"),
    ),
    (
        "thunk_owned_string_result.jinja",
        include_str!("templates/thunk_owned_string_result.jinja"),
    ),
    (
        "thunk_error_result.jinja",
        include_str!("templates/thunk_error_result.jinja"),
    ),
    (
        "thunk_if_ok_result.jinja",
        include_str!("templates/thunk_if_ok_result.jinja"),
    ),
    ("thunk_if_error.jinja", include_str!("templates/thunk_if_error.jinja")),
    (
        "thunk_infallible_return.jinja",
        include_str!("templates/thunk_infallible_return.jinja"),
    ),
    (
        "trait_vtable_header.jinja",
        include_str!("templates/trait_vtable_header.jinja"),
    ),
    (
        "trait_struct_header.jinja",
        include_str!("templates/trait_struct_header.jinja"),
    ),
    (
        "trait_method_doc.jinja",
        include_str!("templates/trait_method_doc.jinja"),
    ),
    (
        "trait_method_doc_lines.jinja",
        include_str!("templates/trait_method_doc_lines.jinja"),
    ),
    (
        "trait_method_signature.jinja",
        include_str!("templates/trait_method_signature.jinja"),
    ),
    (
        "register_fn_doc1.jinja",
        include_str!("templates/register_fn_doc1.jinja"),
    ),
    (
        "register_fn_doc2.jinja",
        include_str!("templates/register_fn_doc2.jinja"),
    ),
    (
        "register_fn_signature.jinja",
        include_str!("templates/register_fn_signature.jinja"),
    ),
    (
        "register_fn_body.jinja",
        include_str!("templates/register_fn_body.jinja"),
    ),
    (
        "unregister_fn_doc.jinja",
        include_str!("templates/unregister_fn_doc.jinja"),
    ),
    (
        "unregister_fn_signature.jinja",
        include_str!("templates/unregister_fn_signature.jinja"),
    ),
    (
        "unregister_fn_configured_signature.jinja",
        include_str!("templates/unregister_fn_configured_signature.jinja"),
    ),
    (
        "unregister_fn_body.jinja",
        include_str!("templates/unregister_fn_body.jinja"),
    ),
    ("clear_fn_doc.jinja", include_str!("templates/clear_fn_doc.jinja")),
    (
        "clear_fn_signature.jinja",
        include_str!("templates/clear_fn_signature.jinja"),
    ),
    ("clear_fn_body.jinja", include_str!("templates/clear_fn_body.jinja")),
    (
        "make_vtable_doc_header.jinja",
        include_str!("templates/make_vtable_doc_header.jinja"),
    ),
    (
        "make_vtable_lifecycle_name.jinja",
        include_str!("templates/make_vtable_lifecycle_name.jinja"),
    ),
    (
        "make_vtable_lifecycle_version.jinja",
        include_str!("templates/make_vtable_lifecycle_version.jinja"),
    ),
    (
        "make_vtable_lifecycle_initialize.jinja",
        include_str!("templates/make_vtable_lifecycle_initialize.jinja"),
    ),
    (
        "make_vtable_lifecycle_shutdown.jinja",
        include_str!("templates/make_vtable_lifecycle_shutdown.jinja"),
    ),
    (
        "opaque_constructor_doc.jinja",
        include_str!("templates/opaque_constructor_doc.jinja"),
    ),
    (
        "opaque_constructor_signature.jinja",
        include_str!("templates/opaque_constructor_signature.jinja"),
    ),
    (
        "opaque_constructor_string_param.jinja",
        include_str!("templates/opaque_constructor_string_param.jinja"),
    ),
    (
        "opaque_constructor_body.jinja",
        include_str!("templates/opaque_constructor_body.jinja"),
    ),
    (
        "opaque_handle_header.jinja",
        include_str!("templates/opaque_handle_header.jinja"),
    ),
    (
        "opaque_static_signature.jinja",
        include_str!("templates/opaque_static_signature.jinja"),
    ),
    (
        "opaque_static_body.jinja",
        include_str!("templates/opaque_static_body.jinja"),
    ),
    (
        "opaque_param_enum_i32.jinja",
        include_str!("templates/opaque_param_enum_i32.jinja"),
    ),
    (
        "opaque_param_dupez.jinja",
        include_str!("templates/opaque_param_dupez.jinja"),
    ),
    (
        "opaque_param_optional_dupez.jinja",
        include_str!("templates/opaque_param_optional_dupez.jinja"),
    ),
    (
        "opaque_param_named_from_json.jinja",
        include_str!("templates/opaque_param_named_from_json.jinja"),
    ),
    (
        "opaque_param_optional_named_from_json.jinja",
        include_str!("templates/opaque_param_optional_named_from_json.jinja"),
    ),
    (
        "opaque_free_method.jinja",
        include_str!("templates/opaque_free_method.jinja"),
    ),
    (
        "opaque_stream_struct.jinja",
        include_str!("templates/opaque_stream_struct.jinja"),
    ),
    (
        "opaque_stream_method.jinja",
        include_str!("templates/opaque_stream_method.jinja"),
    ),
    (
        "opaque_method_signature.jinja",
        include_str!("templates/opaque_method_signature.jinja"),
    ),
    (
        "opaque_bytes_out_vars.jinja",
        include_str!("templates/opaque_bytes_out_vars.jinja"),
    ),
    (
        "opaque_method_call_discard.jinja",
        include_str!("templates/opaque_method_call_discard.jinja"),
    ),
    (
        "opaque_method_call_result.jinja",
        include_str!("templates/opaque_method_call_result.jinja"),
    ),
    (
        "opaque_method_error_check.jinja",
        include_str!("templates/opaque_method_error_check.jinja"),
    ),
    (
        "opaque_consumed_handle_invalidate.jinja",
        include_str!("templates/opaque_consumed_handle_invalidate.jinja"),
    ),
    (
        "opaque_bytes_return.jinja",
        include_str!("templates/opaque_bytes_return.jinja"),
    ),
    (
        "opaque_method_return.jinja",
        include_str!("templates/opaque_method_return.jinja"),
    ),
    (
        "opaque_method_unit_call.jinja",
        include_str!("templates/opaque_method_unit_call.jinja"),
    ),
    (
        "service_struct_open.jinja",
        include_str!("templates/service_struct_open.jinja"),
    ),
    ("service_init.jinja", include_str!("templates/service_init.jinja")),
    ("service_deinit.jinja", include_str!("templates/service_deinit.jinja")),
    (
        "service_registration_method.jinja",
        include_str!("templates/service_registration_method.jinja"),
    ),
    (
        "service_entrypoint_method.jinja",
        include_str!("templates/service_entrypoint_method.jinja"),
    ),
    (
        "trait_options_handle_from_vtable.jinja",
        include_str!("templates/trait_options_handle_from_vtable.jinja"),
    ),
    (
        "trait_bridge_alias.jinja",
        include_str!("templates/trait_bridge_alias.jinja"),
    ),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    crate::template::configure_env(&mut env);
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/zig/templates"));
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

#[cfg(test)]
mod collapsed_doc_comment_tests {
    use super::TEMPLATES;

    /// A `.jinja` template reflowed by a line-wrapping formatter loses the newlines between
    /// consecutive `///` doc lines, folding a whole declaration onto one comment line and
    /// leaving the remainder as bare code. Zig then fails to parse the emitted module
    /// outright (`error: expected ',' after field`), and because `e2e/zig/build.zig` points
    /// its module root at the same emitted file, one damaged template also takes out the
    /// entire e2e/zig fixture lane — surfacing in a directory that looks unrelated to the
    /// template at fault. This actually happened and was repaired wholesale in
    /// `cac280e66 fix(codegen): restore damaged templates`; the signature is a second `///`
    /// appearing *within* a line that already opened a doc comment. ~keep
    fn first_collapsed_doc_line(content: &str) -> Option<(usize, String)> {
        content.lines().enumerate().find_map(|(index, line)| {
            let trimmed = line.trim_start();
            let after_marker = trimmed.strip_prefix("///")?;
            after_marker.contains("///").then(|| (index + 1, line.to_string()))
        })
    }

    /// Proves the detector can actually fire, using the verbatim pre-repair content of
    /// `trait_options_handle_from_vtable.jinja` (the `-` side of `cac280e66`'s diff). Without
    /// this, a detector that silently matched nothing would look identical to a clean tree.
    #[test]
    fn detector_fires_on_the_historical_damaged_template() {
        let damaged = "/// Create the native visitor accepted by the generated options-field setter. \
                       /// Passing the returned handle to that\n\
                       setter transfers ownership. pub fn {{ snake }}_handle_from_callbacks(callbacks: {{ \
                       callbacks_type }}) ?*{{\n  native_handle_type\n}} { var _cb = callbacks; return {{ ctor_fn \
                       }}(&_cb); }\n";

        let (line_number, line) = first_collapsed_doc_line(damaged).expect("detector must flag the damaged template");

        assert_eq!(line_number, 1);
        assert!(line.contains("setter. /// Passing"), "{line}");
    }

    /// A well-formed multi-line doc block — the repaired form of the same template — must not
    /// trip the detector, or it would fire on every healthy template and be worthless.
    #[test]
    fn detector_ignores_a_well_formed_multi_line_doc_block() {
        let healthy = "/// Create the native visitor accepted by the generated options-field setter.\n\
                       /// Passing the returned handle to that setter transfers ownership.\n\
                       pub fn {{ snake }}_handle_from_callbacks() void {}\n";

        assert_eq!(first_collapsed_doc_line(healthy), None);
    }

    #[test]
    fn no_registered_zig_template_has_a_collapsed_doc_comment() {
        let damaged: Vec<String> = TEMPLATES
            .iter()
            .filter_map(|(name, content)| {
                first_collapsed_doc_line(content).map(|(line_number, line)| format!("{name}:{line_number}: {line}"))
            })
            .collect();

        assert!(
            damaged.is_empty(),
            "zig template(s) carry a collapsed doc comment, which makes the emitted module fail \
             to parse: {damaged:#?}"
        );
    }
}
