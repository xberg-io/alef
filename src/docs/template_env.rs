use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    ("front_matter.jinja", include_str!("templates/front_matter.jinja")),
    ("version_heading.jinja", include_str!("templates/version_heading.jinja")),
    ("heading.jinja", include_str!("templates/heading.jinja")),
    ("code_block.jinja", include_str!("templates/code_block.jinja")),
    ("param_row.jinja", include_str!("templates/param_row.jinja")),
    ("field_row.jinja", include_str!("templates/field_row.jinja")),
    ("variant_row.jinja", include_str!("templates/variant_row.jinja")),
    ("exception_row.jinja", include_str!("templates/exception_row.jinja")),
    (
        "wire_variant_row.jinja",
        include_str!("templates/wire_variant_row.jinja"),
    ),
    (
        "error_message_row.jinja",
        include_str!("templates/error_message_row.jinja"),
    ),
    ("returns.jinja", include_str!("templates/returns.jinja")),
    ("errors_phrase.jinja", include_str!("templates/errors_phrase.jinja")),
    ("base_class.jinja", include_str!("templates/base_class.jinja")),
    ("bold_heading.jinja", include_str!("templates/bold_heading.jinja")),
    ("since_badge.jinja", include_str!("templates/since_badge.jinja")),
    (
        "deprecated_notice.jinja",
        include_str!("templates/deprecated_notice.jinja"),
    ),
    (
        "reference_page_link.jinja",
        include_str!("templates/reference_page_link.jinja"),
    ),
    (
        "trait_bridge_row.jinja",
        include_str!("templates/trait_bridge_row.jinja"),
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
    let rendered = crate::core::keep_marker::strip_keep_markers(&rendered);
    if matches!(template_name, "heading.jinja" | "version_heading.jinja") && !rendered.ends_with("\n\n") {
        return format!("{rendered}\n");
    }
    rendered
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
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/docs/templates"));
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
