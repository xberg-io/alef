use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("lib_header.rs.jinja", include_str!("templates/lib_header.rs.jinja")),
    (
        "cfg_attribute.rs.jinja",
        include_str!("templates/cfg_attribute.rs.jinja"),
    ),
    (
        "runtime_helpers.rs.jinja",
        include_str!("templates/runtime_helpers.rs.jinja"),
    ),
    (
        "trait_bridge_constructor.rs.jinja",
        include_str!("templates/trait_bridge_constructor.rs.jinja"),
    ),
    (
        "trait_bridge_method_body.rs.jinja",
        include_str!("templates/trait_bridge_method_body.rs.jinja"),
    ),
    (
        "trait_bridge_register_shim.rs.jinja",
        include_str!("templates/trait_bridge_register_shim.rs.jinja"),
    ),
    (
        "trait_unregister_shim.rs.jinja",
        include_str!("templates/trait_unregister_shim.rs.jinja"),
    ),
    (
        "trait_clear_shim.rs.jinja",
        include_str!("templates/trait_clear_shim.rs.jinja"),
    ),
    (
        "function_shim_open.rs.jinja",
        include_str!("templates/function_shim_open.rs.jinja"),
    ),
    (
        "call_result_body.rs.jinja",
        include_str!("templates/call_result_body.rs.jinja"),
    ),
    (
        "method_shim_open.rs.jinja",
        include_str!("templates/method_shim_open.rs.jinja"),
    ),
    (
        "method_client_handle.rs.jinja",
        include_str!("templates/method_client_handle.rs.jinja"),
    ),
    (
        "value_method_shim_open.rs.jinja",
        include_str!("templates/value_method_shim_open.rs.jinja"),
    ),
    (
        "value_method_receiver.rs.jinja",
        include_str!("templates/value_method_receiver.rs.jinja"),
    ),
    (
        "streaming_shims.rs.jinja",
        include_str!("templates/streaming_shims.rs.jinja"),
    ),
    (
        "constructor_shim.rs.jinja",
        include_str!("templates/constructor_shim.rs.jinja"),
    ),
    (
        "destructor_shim.rs.jinja",
        include_str!("templates/destructor_shim.rs.jinja"),
    ),
    (
        "service_header.rs.jinja",
        include_str!("templates/service_header.rs.jinja"),
    ),
    (
        "service_opaque.rs.jinja",
        include_str!("templates/service_opaque.rs.jinja"),
    ),
    (
        "handler_bridge_struct.rs.jinja",
        include_str!("templates/handler_bridge_struct.rs.jinja"),
    ),
    (
        "handler_bridge_impl.rs.jinja",
        include_str!("templates/handler_bridge_impl.rs.jinja"),
    ),
    (
        "registration_variant.rs.jinja",
        include_str!("templates/registration_variant.rs.jinja"),
    ),
    (
        "registration_function.rs.jinja",
        include_str!("templates/registration_function.rs.jinja"),
    ),
    (
        "entrypoint_run.rs.jinja",
        include_str!("templates/entrypoint_run.rs.jinja"),
    ),
    (
        "entrypoint_finalize.rs.jinja",
        include_str!("templates/entrypoint_finalize.rs.jinja"),
    ),
    ("param_decl.rs.jinja", include_str!("templates/param_decl.rs.jinja")),
    (
        "service_param_decl.rs.jinja",
        include_str!("templates/service_param_decl.rs.jinja"),
    ),
    (
        "string_unmarshal.rs.jinja",
        include_str!("templates/string_unmarshal.rs.jinja"),
    ),
    (
        "byte_array_unmarshal.rs.jinja",
        include_str!("templates/byte_array_unmarshal.rs.jinja"),
    ),
    (
        "base64_bytes_unmarshal.rs.jinja",
        include_str!("templates/base64_bytes_unmarshal.rs.jinja"),
    ),
    (
        "opaque_handle_unmarshal.rs.jinja",
        include_str!("templates/opaque_handle_unmarshal.rs.jinja"),
    ),
    (
        "complex_unmarshal.rs.jinja",
        include_str!("templates/complex_unmarshal.rs.jinja"),
    ),
    (
        "request_string_unmarshal.rs.jinja",
        include_str!("templates/request_string_unmarshal.rs.jinja"),
    ),
    (
        "request_map_unmarshal.rs.jinja",
        include_str!("templates/request_map_unmarshal.rs.jinja"),
    ),
    (
        "request_map_param_unmarshal.rs.jinja",
        include_str!("templates/request_map_param_unmarshal.rs.jinja"),
    ),
    (
        "vec_string_refs.rs.jinja",
        include_str!("templates/vec_string_refs.rs.jinja"),
    ),
    (
        "path_unmarshal.rs.jinja",
        include_str!("templates/path_unmarshal.rs.jinja"),
    ),
    (
        "vec_string_unmarshal.rs.jinja",
        include_str!("templates/vec_string_unmarshal.rs.jinja"),
    ),
    (
        "request_string_value_unmarshal.rs.jinja",
        include_str!("templates/request_string_value_unmarshal.rs.jinja"),
    ),
    (
        "json_value_unmarshal.rs.jinja",
        include_str!("templates/json_value_unmarshal.rs.jinja"),
    ),
    (
        "wrapper_setup.rs.jinja",
        include_str!("templates/wrapper_setup.rs.jinja"),
    ),
    (
        "stream_request_unmarshal.rs.jinja",
        include_str!("templates/stream_request_unmarshal.rs.jinja"),
    ),
    (
        "stream_call_block.rs.jinja",
        include_str!("templates/stream_call_block.rs.jinja"),
    ),
    ("return_bool.rs.jinja", include_str!("templates/return_bool.rs.jinja")),
    (
        "return_byte_array.rs.jinja",
        include_str!("templates/return_byte_array.rs.jinja"),
    ),
    (
        "return_optional_byte_array.rs.jinja",
        include_str!("templates/return_optional_byte_array.rs.jinja"),
    ),
    (
        "return_primitive.rs.jinja",
        include_str!("templates/return_primitive.rs.jinja"),
    ),
    (
        "return_string.rs.jinja",
        include_str!("templates/return_string.rs.jinja"),
    ),
    (
        "return_optional_string.rs.jinja",
        include_str!("templates/return_optional_string.rs.jinja"),
    ),
    (
        "method_capsule_return.rs.jinja",
        include_str!("templates/method_capsule_return.rs.jinja"),
    ),
    ("return_json.rs.jinja", include_str!("templates/return_json.rs.jinja")),
];

pub(crate) fn render(name: &str, context: minijinja::Value) -> String {
    let env = make_env();
    let rendered = env
        .get_template(name)
        .unwrap_or_else(|err| panic!("missing JNI template {name}: {err}"))
        .render(context)
        .unwrap_or_else(|err| panic!("failed to render JNI template {name}: {err}"));
    crate::core::keep_marker::strip_keep_markers(&rendered)
}

fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    crate::template::configure_env(&mut env);
    for (name, source) in TEMPLATES {
        env.add_template(name, source)
            .unwrap_or_else(|err| panic!("failed to register JNI template {name}: {err}"));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructor_template_uses_jni_panic_boundary() {
        let output = render(
            "destructor_shim.rs.jinja",
            crate::alef_context! { symbol => "Java_dev_sample_free", type_name => "Sample" },
        );

        assert!(output.contains("run_or_throw(env, |_env|"));
    }

    #[test]
    fn crate_headers_contain_implementation_only_unsafe_lints() {
        let header = render(
            "lib_header.rs.jinja",
            crate::alef_context! { core_crate => "sample", error_class => "SampleError", crate_attributes => Vec::<String>::new() },
        );
        let service = render("service_header.rs.jinja", crate::alef_context! {});
        for output in [header, service] {
            assert!(output.contains("unsafe_op_in_unsafe_fn"), "{output}");
            assert!(output.contains("unsafe_attr_outside_unsafe"), "{output}");
        }
    }

    /// `render()` resolves names against `TEMPLATES`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into this array compiles fine
    /// (`include_str!` only runs for entries that are listed) and panics only once an
    /// emitter reaches it at generation time. Compare by content rather than by
    /// registered key: some backends register a file under a shortened or aliased name,
    /// which is fine, but every file's bytes must appear in `TEMPLATES` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/jni/templates"));
        let registered_contents: std::collections::HashSet<&str> =
            TEMPLATES.iter().map(|(_, content)| *content).collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATES: {unregistered:?}"
        );
    }

    fn collect_unregistered(
        root: &std::path::Path,
        dir: &std::path::Path,
        registered_contents: &std::collections::HashSet<&str>,
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
