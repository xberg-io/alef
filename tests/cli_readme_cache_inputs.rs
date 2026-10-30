use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn fixture(root: &Path, readme_options: &str, python_options: &str) {
    write(root, "src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 { a + b }\n");
    write(
        root,
        "Cargo.toml",
        "[package]\nname='readme-cache-probe'\nversion='0.1.0'\nedition='2024'\n",
    );
    write(root, "manual/python/hello.py", "print('ORIGINAL_SNIPPET')\n");
    write(root, "templates/package.md", "ORIGINAL_TEMPLATE\n");
    write(
        root,
        "alef.toml",
        &format!(
            "[workspace]\nlanguages=['python','go']\n\
             [[crates]]\nname='readme-cache-probe'\ncore_import='readme_cache_probe'\n\
             sources=['src/lib.rs']\nversion_from='Cargo.toml'\n\
             [crates.readme]\ntemplate_dir='templates'\nsnippets_dir='manual'\n{readme_options}\n\
             [crates.readme.languages.python]\ntemplate='package.md'\noutput_path='README.md'\n\
             package_name='readme-cache-probe'\n{python_options}\n\
             [crates.readme.languages.go]\ntemplate='package.md'\noutput_path='GO.md'\n\
             package_name='readme-cache-probe'\n"
        ),
    );
}

fn run(root: &Path, languages: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .arg("readme")
        .args(languages)
        .current_dir(root)
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{log}");
    log
}

fn assert_readme(root: &Path, path: &str, expected: &str) {
    let content = fs::read_to_string(root.join(path)).unwrap();
    assert!(content.contains(expected), "expected {expected:?} in {path}: {content}");
}

#[test]
fn unchanged_readme_dependencies_reuse_the_stage_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    let first = run(root, &["--lang", "python"]);
    assert!(!first.contains("READMEs up to date (cached)"), "{first}");
    assert_readme(root, "README.md", "ORIGINAL_TEMPLATE");
    let before = fs::read(root.join("README.md")).unwrap();
    let second = run(root, &["--lang", "python"]);
    assert!(second.contains("READMEs up to date (cached)"), "{second}");
    assert_eq!(fs::read(root.join("README.md")).unwrap(), before);
}

#[test]
fn changing_a_readme_template_invalidates_the_stage_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "ORIGINAL_TEMPLATE");
    write(root, "templates/package.md", "UPDATED_TEMPLATE\n");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "UPDATED_TEMPLATE");
}

#[test]
fn changing_an_included_partial_invalidates_the_stage_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    write(root, "templates/package.md", "{% include 'parts/example.md' %}\n");
    write(root, "templates/parts/example.md", "ORIGINAL_PARTIAL\n");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "ORIGINAL_PARTIAL");
    write(root, "templates/parts/example.md", "UPDATED_PARTIAL\n");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "UPDATED_PARTIAL");
}

#[test]
fn changing_snippets_from_each_supported_root_invalidates_the_stage_cache() {
    for (options, snippet_path) in [
        ("", "manual/python/hello.py"),
        ("snippets_dir='language-snippets'", "language-snippets/python/hello.py"),
        (
            "snippets={hello={path='hello.py',root='mapped-snippets'}}",
            "mapped-snippets/python/hello.py",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fixture(root, "", options);
        write(root, snippet_path, "print('ORIGINAL_SNIPPET')\n");
        let template = if options.starts_with("snippets=") {
            "{{ snippets.hello | include_snippet('python') }}\n"
        } else {
            "{{ 'hello.py' | include_snippet('python') }}\n"
        };
        write(root, "templates/package.md", template);
        run(root, &["--lang", "python"]);
        assert_readme(root, "README.md", "ORIGINAL_SNIPPET");
        write(root, snippet_path, "print('UPDATED_SNIPPET')\n");
        run(root, &["--lang", "python"]);
        assert_readme(root, "README.md", "UPDATED_SNIPPET");
    }
}

#[test]
fn selecting_another_language_does_not_reuse_a_partial_readme_run() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "ORIGINAL_TEMPLATE");
    assert!(!root.join("GO.md").exists());
    run(root, &["--lang", "go"]);
    assert_readme(root, "GO.md", "ORIGINAL_TEMPLATE");
}

fn regenerate_fixture_snippet(root: &Path, value: i32) {
    use alef::core::config::NewAlefConfig;
    use alef::core::config::e2e::{E2eConfig, SnippetConfig};
    use alef::e2e::fixture::Fixture;

    let config: NewAlefConfig = toml::from_str(&fs::read_to_string(root.join("alef.toml")).unwrap()).unwrap();
    let resolved = config.resolve().unwrap().remove(0);
    let e2e: E2eConfig = toml::from_str(
        "[call]\nfunction='add'\nmodule='readme_cache_probe'\nreturns_result=false\n\
         [[call.args]]\nname='a'\nfield='a'\ntype='int'\n\
         [[call.args]]\nname='b'\nfield='b'\ntype='int'\n",
    )
    .unwrap();
    let fixture_json = serde_json::json!({
        "id": "hello", "description": "Add two values", "input": {"a": value, "b": 2},
        "docs": {"topic": "api", "stem": "hello"}
    });
    write(
        root,
        "fixtures/hello.json",
        &serde_json::to_string(&fixture_json).unwrap(),
    );
    let fixture: Fixture = serde_json::from_slice(&fs::read(root.join("fixtures/hello.json")).unwrap()).unwrap();
    let snippets = SnippetConfig {
        output: "manual".into(),
        ..SnippetConfig::default()
    };
    let files = alef::e2e::snippets::generate_snippets(
        &[fixture],
        &["python".into()],
        &e2e,
        &snippets,
        &resolved,
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, Path::new("manual/python/api/hello.md"));
    let count = alef::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();
    assert_eq!(count, 1);
    assert_readme(root, "manual/python/api/hello.md", &value.to_string());
}

#[test]
fn readme_refreshes_after_the_fixture_snippet_producer_runs_later() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    write(
        root,
        "templates/package.md",
        "{{ 'api/hello.md' | include_snippet('python') }}\n",
    );
    regenerate_fixture_snippet(root, 1001);
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "1001");
    regenerate_fixture_snippet(root, 2002);
    assert_readme(root, "README.md", "1001");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "2002");
}

#[test]
fn changing_legacy_external_yaml_invalidates_the_stage_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "config='readme.yaml'", "");
    let config = fs::read_to_string(root.join("alef.toml")).unwrap();
    write(
        root,
        "alef.toml",
        config.split("[crates.readme.languages.python]").next().unwrap(),
    );
    write(root, "templates/package.md", "{{ package_name }}\n");
    for name in ["ORIGINAL_YAML", "UPDATED_YAML"] {
        write(
            root,
            "readme.yaml",
            &format!(
                "languages:\n  python:\n    template: package.md\n    output_path: README.md\n    package_name: {name}\n"
            ),
        );
        run(root, &["--lang", "python"]);
        assert_readme(root, "README.md", name);
    }
}

#[test]
fn migration_file_presence_invalidates_the_stage_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "", "");
    write(
        root,
        "templates/package.md",
        "{% if has_migration('python', '0.1.0') %}PRESENT_MIGRATION{% else %}ABSENT_MIGRATION{% endif %}\n",
    );
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "ABSENT_MIGRATION");
    write(root, "docs/migrations/python/0.1.0.md", "Migration notes\n");
    run(root, &["--lang", "python"]);
    assert_readme(root, "README.md", "PRESENT_MIGRATION");
}
