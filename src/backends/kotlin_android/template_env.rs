use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("kt_file.jinja", include_str!("templates/kt_file.jinja")),
    (
        "trait_bridge_object.jinja",
        include_str!("templates/trait_bridge_object.jinja"),
    ),
    (
        "trait_bridge_dispatcher.jinja",
        include_str!("templates/trait_bridge_dispatcher.jinja"),
    ),
    (
        "handle_wrapper_header.jinja",
        include_str!("templates/handle_wrapper_header.jinja"),
    ),
    (
        "android_streaming_method.jinja",
        include_str!("templates/android_streaming_method.jinja"),
    ),
    (
        "android_jni_libs_task.jinja",
        include_str!("templates/android_jni_libs_task.jinja"),
    ),
    (
        "module_object_header.jinja",
        include_str!("templates/module_object_header.jinja"),
    ),
    (
        "android_facade_dto_method.jinja",
        include_str!("templates/android_facade_dto_method.jinja"),
    ),
    (
        "android_facade_generic_method.jinja",
        include_str!("templates/android_facade_generic_method.jinja"),
    ),
    (
        "android_facade_async_method.jinja",
        include_str!("templates/android_facade_async_method.jinja"),
    ),
    (
        "android_facade_expr_method.jinja",
        include_str!("templates/android_facade_expr_method.jinja"),
    ),
    (
        "android_facade_jackson_config.jinja",
        include_str!("templates/android_facade_jackson_config.jinja"),
    ),
    (
        "android_facade_dto_async_impl.jinja",
        include_str!("templates/android_facade_dto_async_impl.jinja"),
    ),
    (
        "android_facade_generic_async_impl.jinja",
        include_str!("templates/android_facade_generic_async_impl.jinja"),
    ),
    (
        "android_streaming_mapper.jinja",
        include_str!("templates/android_streaming_mapper.jinja"),
    ),
    (
        "capsule_function_wrapper.jinja",
        include_str!("templates/capsule_function_wrapper.jinja"),
    ),
    (
        "generation_error.jinja",
        include_str!("templates/generation_error.jinja"),
    ),
    (
        "trait_interface_header.jinja",
        include_str!("templates/trait_interface_header.jinja"),
    ),
    (
        "trait_method_return_line.jinja",
        include_str!("templates/trait_method_return_line.jinja"),
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
        let templates_dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/backends/kotlin_android/templates"
        ));
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
