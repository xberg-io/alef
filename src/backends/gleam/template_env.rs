use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("type_opaque.jinja", include_str!("templates/type_opaque.jinja")),
    (
        "type_opaque_resource.jinja",
        include_str!("templates/type_opaque_resource.jinja"),
    ),
    (
        "resource_method_external.jinja",
        include_str!("templates/resource_method_external.jinja"),
    ),
    ("type_header.jinja", include_str!("templates/type_header.jinja")),
    ("field_labeled.jinja", include_str!("templates/field_labeled.jinja")),
    (
        "field_positional.jinja",
        include_str!("templates/field_positional.jinja"),
    ),
    ("variant_simple.jinja", include_str!("templates/variant_simple.jinja")),
    (
        "variant_with_fields.jinja",
        include_str!("templates/variant_with_fields.jinja"),
    ),
    ("enum_header.jinja", include_str!("templates/enum_header.jinja")),
    ("error_header.jinja", include_str!("templates/error_header.jinja")),
    (
        "function_external.jinja",
        include_str!("templates/function_external.jinja"),
    ),
    (
        "function_signature.jinja",
        include_str!("templates/function_signature.jinja"),
    ),
    (
        "trait_bridge_doc_header.jinja",
        include_str!("templates/trait_bridge_doc_header.jinja"),
    ),
    ("register_fn.jinja", include_str!("templates/register_fn.jinja")),
    ("unregister_fn.jinja", include_str!("templates/unregister_fn.jinja")),
    ("clear_fn.jinja", include_str!("templates/clear_fn.jinja")),
    (
        "method_doc_header.jinja",
        include_str!("templates/method_doc_header.jinja"),
    ),
    (
        "method_doc_usage.jinja",
        include_str!("templates/method_doc_usage.jinja"),
    ),
    ("method_external.jinja", include_str!("templates/method_external.jinja")),
    (
        "method_signature.jinja",
        include_str!("templates/method_signature.jinja"),
    ),
    (
        "trait_type_doc_lines.jinja",
        include_str!("templates/trait_type_doc_lines.jinja"),
    ),
    ("import_line.jinja", include_str!("templates/import_line.jinja")),
    (
        "trait_bridge_doc_line.jinja",
        include_str!("templates/trait_bridge_doc_line.jinja"),
    ),
    (
        "trait_bridge_empty_comment_line.jinja",
        include_str!("templates/trait_bridge_empty_comment_line.jinja"),
    ),
    ("trait_scope_cap.jinja", include_str!("templates/trait_scope_cap.jinja")),
    ("support_nif_doc.jinja", include_str!("templates/support_nif_doc.jinja")),
    (
        "support_nif_complete.jinja",
        include_str!("templates/support_nif_complete.jinja"),
    ),
    (
        "support_nif_fail.jinja",
        include_str!("templates/support_nif_fail.jinja"),
    ),
    (
        "support_nif_fail_doc.jinja",
        include_str!("templates/support_nif_fail_doc.jinja"),
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/gleam/templates"));
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
