use minijinja::Environment;

mod core_templates;
mod enum_error_templates;
mod rust_crate_templates;
mod streaming_templates;
mod swift_templates;
mod wrapper_templates;

static TEMPLATE_GROUPS: &[&[(&str, &str)]] = &[
    core_templates::TEMPLATES,
    enum_error_templates::TEMPLATES,
    wrapper_templates::TEMPLATES,
    rust_crate_templates::TEMPLATES,
    swift_templates::TEMPLATES,
    streaming_templates::TEMPLATES,
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    crate::template::configure_env(&mut env);
    for templates in TEMPLATE_GROUPS {
        for (name, src) in *templates {
            env.add_template(name, src).expect("built-in template is valid");
        }
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
mod tests {
    use super::{TEMPLATE_GROUPS, render};

    /// The raw-pointer wrapper template must be registered in `TEMPLATE_GROUPS`.
    /// It is rendered from `gen_rust_crate::service_app_wrappers` for services
    /// with wrapper constructors; a template added to the directory but omitted
    /// from a group renders fine in unit tests that never hit its call site, then
    /// panics at generation time (`template <name> not found`). Rendering it here
    /// fails fast if it is ever dropped from the registry.
    #[test]
    fn raw_ptr_wrapper_template_is_registered_and_renders() {
        let out = render(
            "rust_wrapper_raw_ptr_fn.rs.jinja",
            crate::alef_context! { wrapper_type => "RouteBuilder", fn_snake => "route_builder_raw_ptr" },
        );
        assert!(out.contains("route_builder_raw_ptr"), "fn name must be rendered: {out}");
        assert!(out.contains("RouteBuilder"), "wrapper type must be rendered: {out}");
    }

    /// `render()` resolves names against `TEMPLATE_GROUPS`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into one of the group submodules
    /// compiles fine (`include_str!` only runs for entries that are listed) and panics only
    /// once an emitter reaches it at generation time. Compare by content rather than by
    /// registered key: some backends register a file under a shortened or aliased name,
    /// which is fine, but every file's bytes must appear in `TEMPLATE_GROUPS` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/swift/templates"));
        let registered_contents: std::collections::HashSet<&str> = TEMPLATE_GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .map(|(_, content)| *content)
            .collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATE_GROUPS: {unregistered:?}"
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
