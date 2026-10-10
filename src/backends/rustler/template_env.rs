use minijinja::Environment;

mod files;
mod inline;

const TEMPLATE_GROUPS: &[&[(&str, &str)]] = &[inline::TEMPLATES, files::TEMPLATES];

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

/// Escaping for the target language happens in Rust, before a value enters a template, so the
/// template engine must not touch it again. Minijinja picks an autoescape mode from the template
/// NAME, and every template here ends in `.jinja`, which selects no escaping -- but that is a
/// default, not a declaration, and a value that arrived already escaped for Elixir would be
/// silently corrupted (`"` -> `&quot;`) if it ever changed. These tests pin it. ~keep
#[cfg(test)]
mod autoescape_tests {
    #[test]
    fn rendering_a_text_template_does_not_autoescape() {
        let mut env = super::make_env();
        env.add_template("autoescape_probe.ex.jinja", "{{ probe }}")
            .expect("probe template is valid");
        let rendered = env
            .get_template("autoescape_probe.ex.jinja")
            .expect("probe template is registered")
            .render(crate::alef_context! { probe => "a<b>&\"c\"'d'" })
            .expect("probe template renders");
        assert_eq!(
            rendered, "a<b>&\"c\"'d'",
            "the environment must emit text verbatim; HTML-escaping here would corrupt every \
             value the backends escape for their own target language before rendering"
        );
    }

    /// The same property on the real template rather than a probe: an Elixir-escaped tag and
    /// wire value must land in the output exactly as Rust produced them -- neither HTML-escaped
    /// nor escaped a second time (`\#` becoming `\\#`, which would emit a literal backslash).
    #[test]
    fn the_tagged_enum_encoder_template_passes_escaped_values_through_unchanged() {
        let rendered = super::render(
            "elixir_tagged_enum_encoder.ex.jinja",
            crate::alef_context! {
                fn_name => "encode_e",
                enum_name => "E",
                tag => "ta\\#{1}g",
                variants => vec![crate::alef_context! {
                    atom => "a",
                    wire => "wi\\#{1}re",
                    is_unit => true,
                    field_renames => Vec::<minijinja::Value>::new(),
                }],
            },
        );
        assert!(
            rendered.contains("defp encode_e(:a), do: %{\"ta\\#{1}g\" => \"wi\\#{1}re\"}"),
            "escaped values must survive rendering byte for byte; got:\n{rendered}"
        );
    }
}

#[cfg(test)]
mod template_registration_tests {
    use super::TEMPLATE_GROUPS;
    use std::collections::HashSet;
    use std::path::Path;

    /// `render()` resolves names against `TEMPLATE_GROUPS`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into `files::TEMPLATES` (or given
    /// an inline entry in `inline::TEMPLATES`) compiles fine (`include_str!` only runs for
    /// entries that are listed) and panics only once an emitter reaches it at generation
    /// time. Compare by content rather than by registered key: some entries in
    /// `inline::TEMPLATES` are inline literal strings with no backing file at all, and a
    /// registered key may be aliased away from the filename, but every file's bytes must
    /// appear in `TEMPLATE_GROUPS` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/rustler/templates"));
        let registered_contents: HashSet<&str> = TEMPLATE_GROUPS
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
