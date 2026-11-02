use super::*;
use std::fs;

const SKILL_ROOTS: [&str; 4] = [".agents/skills", ".claude/skills", ".codex/skills", ".github/skills"];
const SHARED_REFERENCES: [&str; 3] = ["configuration.md", "types.md", "errors.md"];

fn catalog_config(root: &Path) -> ResolvedCrateConfig {
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/api.jinja"),
        "# API catalog\n\n{% for language in languages %}language: {{ language }}\n{% endfor %}\n\
         {% for reference in api_references %}reference: {{ reference.path }}\n{% endfor %}",
    )
    .unwrap();
    config_from_toml(
        r#"
[workspace]
languages = ["java", "rust"]

[workspace.docs]
reference_output = "docs/reference"

[workspace.docs.skills]
outputs = [".agents/skills", ".claude/skills", ".codex/skills", ".github/skills"]

[workspace.docs.skills.templates.api]
template = "templates/api.jinja"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
}

fn write_catalog_stage(root: &Path, config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<PathBuf> {
    let mut api = make_minimal_api("1.0.0");
    api.functions = vec![make_function(
        "answer",
        vec![],
        TypeRef::Primitive(PrimitiveType::I32),
        false,
        None,
    )];
    let (files, result) = generate_docs_stage_without_snippet_compile_validation(&api, config, languages, None, root);
    result.unwrap();
    assert_eq!(
        files.len(),
        languages.len() + SHARED_REFERENCES.len() + SKILL_ROOTS.len()
    );
    crate::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();
    files.into_iter().map(|file| file.path).collect()
}

fn catalog_entries(content: &str, prefix: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| line.strip_prefix(prefix).map(str::to_string))
        .collect()
}

#[test]
fn should_preserve_shared_api_catalogs_when_rendering_one_configured_language() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let config = catalog_config(root);
    let default_languages = crate::bin_cli::helpers::resolve_doc_languages(&config, None).unwrap();
    assert_eq!(default_languages, [Language::Java, Language::Rust]);
    write_catalog_stage(root, &config, &default_languages);
    let java_page = root.join("docs/reference/api-java.md");
    let original_java = fs::read(&java_page).unwrap();
    assert!(
        String::from_utf8_lossy(&original_java)
            .to_ascii_lowercase()
            .contains("answer")
    );
    let catalogs = SKILL_ROOTS.map(|directory| root.join(directory).join("api/SKILL.md"));
    let original_catalog = fs::read_to_string(&catalogs[0]).unwrap();
    assert_eq!(catalog_entries(&original_catalog, "language: "), ["java", "rust"]);
    assert_eq!(
        catalog_entries(&original_catalog, "reference: "),
        [
            "docs/reference/api-java.md",
            "docs/reference/api-rust.md",
            "docs/reference/configuration.md",
            "docs/reference/types.md",
            "docs/reference/errors.md",
        ]
    );
    for catalog in &catalogs {
        assert_eq!(fs::read_to_string(catalog).unwrap(), original_catalog);
    }

    let scoped_paths = write_catalog_stage(root, &config, &[Language::Rust]);

    assert!(!scoped_paths.contains(&PathBuf::from("docs/reference/api-java.md")));
    assert_eq!(fs::read(java_page).unwrap(), original_java);
    for catalog in catalogs {
        assert_eq!(
            fs::read_to_string(&catalog).unwrap(),
            original_catalog,
            "scoped docs must preserve the full shared catalog at {}",
            catalog.display()
        );
    }
}

#[test]
fn should_retain_requested_catalog_when_no_languages_are_configured() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let mut config = catalog_config(root);
    config.languages.clear();

    write_catalog_stage(root, &config, &[Language::Java, Language::Rust]);

    for directory in SKILL_ROOTS {
        let catalog = fs::read_to_string(root.join(directory).join("api/SKILL.md")).unwrap();
        assert_eq!(catalog_entries(&catalog, "language: "), ["java", "rust"]);
    }
}

#[test]
fn should_catalog_the_ffi_reference_once_when_internal_targets_are_configured() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let mut config = catalog_config(root);
    config.languages = vec![Language::Ffi, Language::C, Language::Jni];

    write_catalog_stage(root, &config, &[Language::Ffi]);

    for directory in SKILL_ROOTS {
        let catalog = fs::read_to_string(root.join(directory).join("api/SKILL.md")).unwrap();
        assert_eq!(
            catalog_entries(&catalog, "reference: "),
            [
                "docs/reference/api-c.md",
                "docs/reference/api-rust.md",
                "docs/reference/configuration.md",
                "docs/reference/types.md",
                "docs/reference/errors.md",
            ]
        );
    }
}

#[test]
fn should_preserve_source_rust_reference_when_binding_config_does_not_list_rust() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let mut config = catalog_config(root);
    config.languages = vec![Language::Java];
    let default_languages = crate::bin_cli::helpers::resolve_doc_languages(&config, None).unwrap();
    assert_eq!(default_languages, [Language::Java, Language::Rust]);
    write_catalog_stage(root, &config, &default_languages);
    let catalog_path = root.join(".agents/skills/api/SKILL.md");
    let original_catalog = fs::read_to_string(&catalog_path).unwrap();
    assert_eq!(catalog_entries(&original_catalog, "language: "), ["java", "rust"]);

    write_catalog_stage(root, &config, &[Language::Java]);

    for directory in SKILL_ROOTS {
        let catalog = fs::read_to_string(root.join(directory).join("api/SKILL.md")).unwrap();
        assert_eq!(catalog, original_catalog);
    }
}
