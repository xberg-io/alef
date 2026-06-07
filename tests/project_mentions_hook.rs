use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn hook_path() -> PathBuf {
    repo_root().join("hooks/check_project_mentions.py")
}

fn run_hook(files: &[&Path]) -> Output {
    run_hook_with_args(&[], files)
}

fn run_hook_with_args(args: &[&str], files: &[&Path]) -> Output {
    let mut command = Command::new("python3");
    command.arg(hook_path());
    command.args(args);
    for file in files {
        command.arg(file);
    }
    command.output().expect("hook command must run")
}

fn forbidden(parts: &[&str], separator: &str) -> String {
    parts.join(separator)
}

#[test]
fn reports_case_insensitive_project_mentions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("code.rs");
    fs::write(
        &file,
        format!("const NAME: &str = {:?};\n", forbidden(&["Kreuz", "Berg"], "")),
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should reject forbidden mention");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("forbidden project mention"), "stderr: {stderr}");
    assert!(stderr.contains("Alef must stay project-agnostic"), "stderr: {stderr}");
}

#[test]
fn reports_dash_underscore_space_and_collapsed_variants() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dash = dir.path().join("dash.rs");
    let underscore = dir.path().join("underscore.rs");
    let spaced = dir.path().join("spaced.rs");
    let collapsed = dir.path().join("collapsed.rs");

    fs::write(&dash, forbidden(&["html", "to", "markdown"], "-")).expect("write dash fixture");
    fs::write(&underscore, forbidden(&["tree", "sitter", "language", "pack"], "_")).expect("write underscore fixture");
    fs::write(&spaced, forbidden(&["liter", "llm"], " ")).expect("write spaced fixture");
    fs::write(&collapsed, forbidden(&["ts", "pack"], "")).expect("write collapsed fixture");

    let output = run_hook(&[&dash, &underscore, &spaced, &collapsed]);

    assert!(!output.status.success(), "hook should reject all separator variants");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert_eq!(
        stderr.matches("forbidden project mention").count(),
        4,
        "stderr: {stderr}"
    );
}

#[test]
fn reports_downstream_sample_pattern_mentions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src").join("backends").join("sample");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    let llm = backend_dir.join("sample_llm.rs");
    let markdown = backend_dir.join("sample_markdown.rs");
    let crawler = backend_dir.join("sample_crawler.rs");

    fs::write(&llm, "let fallback = configured.unwrap_or(\"sample-llm\");\n").expect("write llm fixture");
    fs::write(
        &markdown,
        "if crate_name == \"SampleMarkdown\" { emit_special_case(); }\n",
    )
    .expect("write markdown fixture");
    fs::write(
        &crawler,
        "match fixture_name { \"sample_crawler\" => emit_special_case(), _ => {} }\n",
    )
    .expect("write crawler fixture");

    let output = run_hook(&[&llm, &markdown, &crawler]);

    assert!(
        !output.status.success(),
        "hook should reject downstream sample patterns"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream sample fixture mention `sample-llm`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden downstream sample fixture mention `sample-markdown`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden downstream sample fixture mention `sample-crawler`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_forbidden_names_embedded_in_identifiers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let facade = dir.path().join("facade.rs");
    let client = dir.path().join("client.rs");

    fs::write(&facade, "struct KreuzbergLib;\n").expect("write facade fixture");
    fs::write(&client, "struct LiterLlmClient;\n").expect("write client fixture");

    let output = run_hook(&[&facade, &client]);

    assert!(!output.status.success(), "hook should reject embedded identifiers");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden project mention `kreuzberg`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden project mention `liter-llm`"),
        "stderr: {stderr}"
    );
}

#[test]
fn accepts_clean_generic_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("code.rs");
    fs::write(&file, "let package_name = config.package_name.clone();\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reports_mentions_in_source_comments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("code.rs");
    fs::write(&file, format!("// {}\n", forbidden(&["spik", "ard"], ""))).expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should reject production-source comment");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden project mention `spikard`"),
        "stderr: {stderr}"
    );
}

#[test]
fn accepts_explicit_alef_owned_infrastructure_mentions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("publish.yaml");
    fs::write(
        &file,
        concat!(
            "uses: kreuzberg-dev/actions/setup-rust@v1\n",
            "repo: kreuzberg-dev/alef\n",
            "tap: kreuzberg-dev/homebrew-tap\n",
            "docs_host: docs.<repo>.kreuzberg.dev\n",
        ),
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reports_downstream_repos_in_infrastructure_namespace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("config.toml");
    fs::write(
        &file,
        format!(
            "remote = {:?}\n",
            format!("github.com/kreuzberg-dev/{}", forbidden(&["kreuz", "berg"], ""))
        ),
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should reject downstream repo name");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden project mention `kreuzberg`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_downstream_domain_types_in_production_generator_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src").join("backends").join("swift");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    let file = backend_dir.join("gen_visitor.rs");
    fs::write(&file, "if type_name == \"InternalDocument\" { emit_special_case(); }\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        !output.status.success(),
        "hook should reject downstream domain type special-casing"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `InternalDocument`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("must not hard-code downstream domain types"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_conversion_options_visitor_special_paths_in_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src").join("codegen");
    fs::create_dir_all(&codegen_dir).expect("create codegen dir");
    let file = codegen_dir.join("visitor.rs");
    fs::write(
        &file,
        "if visitor_options == \"ConversionOptions\" { emit_special_case(); }\n",
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        !output.status.success(),
        "hook should reject conversion visitor special paths"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `ConversionOptions`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_conversion_and_extraction_result_special_paths_in_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src").join("codegen");
    fs::create_dir_all(&codegen_dir).expect("create codegen dir");
    let conversion_file = codegen_dir.join("conversion.rs");
    let extraction_file = codegen_dir.join("extraction.rs");
    fs::write(
        &conversion_file,
        "if return_type == \"ConversionResult\" { emit_special_case(); }\n",
    )
    .expect("write conversion fixture");
    fs::write(
        &extraction_file,
        "if return_type == \"ExtractionResult\" { emit_special_case(); }\n",
    )
    .expect("write extraction fixture");

    let output = run_hook(&[&conversion_file, &extraction_file]);

    assert!(!output.status.success(), "hook should reject result type special paths");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `ConversionResult`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden downstream domain type `ExtractionResult`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_split_concatenated_template_literals_in_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let template_dir = dir
        .path()
        .join("src")
        .join("backends")
        .join("typescript")
        .join("templates");
    fs::create_dir_all(&template_dir).expect("create template dir");
    let file = template_dir.join("types.ts.jinja");
    fs::write(&file, "export class {{ \"Conversion\" ~ \"Options\" }} {}\n").expect("write template fixture");

    let output = run_hook(&[&file]);

    assert!(
        !output.status.success(),
        "hook should reject split concatenated domain type literals"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `ConversionOptions`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_embedded_visitor_bridge_class_names_in_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src").join("backends").join("java");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    let file = backend_dir.join("ffi_class.rs");
    fs::write(&file, r#"out.push_str("new JsHtmlVisitorBridge(config.hook())");"#).expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        !output.status.success(),
        "hook should reject embedded visitor bridge class names"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `HtmlVisitor`"),
        "stderr: {stderr}"
    );
}

#[test]
fn scans_ai_rulez_source_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rules_dir = dir.path().join(".ai-rulez").join("rules");
    fs::create_dir_all(&rules_dir).expect("create rule dir");
    let file = rules_dir.join("project.md");
    fs::write(&file, forbidden(&["liter", "llm"], "-")).expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should scan .ai-rulez sources");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden project mention `liter-llm`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_string_literal_fallbacks_in_e2e_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src").join("e2e").join("codegen");
    fs::create_dir_all(&codegen_dir).expect("create e2e codegen dir");
    let file = codegen_dir.join("php.rs");
    fs::write(&file, "let options = configured.unwrap_or(\"ConversionOptions\");\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should reject string-literal fallback");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `ConversionOptions`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_plain_downstream_sample_literals_in_e2e_codegen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src").join("e2e").join("codegen");
    fs::create_dir_all(&codegen_dir).expect("create e2e codegen dir");
    let file = codegen_dir.join("zig.rs");
    fs::write(&file, "out.push_str(\"sample-crawler\");\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        !output.status.success(),
        "hook should reject plain downstream sample literals in production e2e codegen"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream sample fixture mention `sample-crawler`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_downstream_domain_types_in_scaffold_and_publish_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scaffold_dir = dir.path().join("src").join("scaffold");
    let publish_dir = dir.path().join("src").join("publish");
    fs::create_dir_all(&scaffold_dir).expect("create scaffold dir");
    fs::create_dir_all(&publish_dir).expect("create publish dir");
    let scaffold_file = scaffold_dir.join("template.rs");
    let publish_file = publish_dir.join("metadata.rs");

    fs::write(
        &scaffold_file,
        "if type_name == \"EmbeddingConfig\" { emit_special_case(); }\n",
    )
    .expect("write scaffold fixture");
    fs::write(
        &publish_file,
        "let default_type = configured.unwrap_or(\"ExtractionConfig\");\n",
    )
    .expect("write publish fixture");

    let output = run_hook(&[&scaffold_file, &publish_file]);

    assert!(
        !output.status.success(),
        "hook should reject downstream domain types in production scaffold and publish code"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `EmbeddingConfig`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden downstream domain type `ExtractionConfig`"),
        "stderr: {stderr}"
    );
}

#[test]
fn accepts_downstream_domain_type_names_in_scaffold_cfg_test_modules() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scaffold_dir = dir.path().join("src").join("scaffold");
    fs::create_dir_all(&scaffold_dir).expect("create scaffold dir");
    let file = scaffold_dir.join("template.rs");
    fs::write(
        &file,
        concat!(
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    #[test]\n",
            "    fn accepts_policy_fixture() {\n",
            "        assert_eq!(\"InternalDocument\", \"InternalDocument\");\n",
            "    }\n",
            "}\n",
        ),
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn accepts_downstream_domain_type_names_in_comments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src").join("e2e").join("codegen");
    fs::create_dir_all(&codegen_dir).expect("create e2e codegen dir");
    let file = codegen_dir.join("go.rs");
    fs::write(&file, "// regression mentions InternalDocument in prose only\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn accepts_downstream_domain_type_names_outside_production_generator_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cli_dir = dir.path().join("src").join("cli");
    fs::create_dir_all(&cli_dir).expect("create cli dir");
    let file = cli_dir.join("help.rs");
    fs::write(&file, "let example = \"InternalDocument\";\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn accepts_opinionated_generic_capability_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("capabilities.rs");
    fs::write(
        &file,
        "enum Capability { HostedDocs, PackageName, RepositoryMetadata }\n",
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn skips_tests_fixtures_snapshots_and_prose_documentation_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let test_file = dir.path().join("tests").join("code.rs");
    let fixture_file = dir.path().join("fixtures").join("input.toml");
    let snapshot_file = dir.path().join("snapshots").join("output.snap");
    let docs_file = dir.path().join("notes.md");
    fs::create_dir_all(test_file.parent().expect("test parent")).expect("create test dir");
    fs::create_dir_all(fixture_file.parent().expect("fixture parent")).expect("create fixture dir");
    fs::create_dir_all(snapshot_file.parent().expect("snapshot parent")).expect("create snapshot dir");
    fs::write(&test_file, forbidden(&["spik", "ard"], "")).expect("write test fixture");
    fs::write(&fixture_file, forbidden(&["h2", "m"], "")).expect("write input fixture");
    fs::write(&snapshot_file, forbidden(&["ll", "lm"], "")).expect("write snapshot fixture");
    fs::write(&docs_file, forbidden(&["html", "to", "markdown"], "-")).expect("write docs fixture");

    let output = run_hook(&[&test_file, &fixture_file, &snapshot_file, &docs_file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn strict_mode_reports_mentions_in_snapshots_and_docs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let snapshot_dir = dir.path().join("tests").join("snapshots");
    fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
    let snapshot = snapshot_dir.join("fixture.snap");
    let doc = snapshot_dir.join("guide.md");

    fs::write(&snapshot, forbidden(&["sample", "crawler"], "-")).expect("write snapshot fixture");
    fs::write(&doc, forbidden(&["liter", "llm"], "-")).expect("write doc fixture");

    let output = run_hook_with_args(&["--strict"], &[&snapshot, &doc]);

    assert!(!output.status.success(), "strict hook should reject snapshots and docs");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream sample fixture mention `sample-crawler`"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("forbidden project mention `liter-llm`"),
        "stderr: {stderr}"
    );
}

#[test]
fn strict_mode_reports_domain_types_in_prose() {
    let dir = tempfile::tempdir().expect("tempdir");
    let snapshot_dir = dir.path().join("tests").join("snapshots");
    fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
    let doc = snapshot_dir.join("guide.md");

    fs::write(&doc, "Regression note for ConversionOptions.\n").expect("write doc fixture");

    let output = run_hook_with_args(&["--strict"], &[&doc]);

    assert!(!output.status.success(), "strict hook should reject prose domain types");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("forbidden downstream domain type `ConversionOptions`"),
        "stderr: {stderr}"
    );
}

#[test]
fn reports_multiple_files_with_line_numbers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = dir.path().join("first.toml");
    let second = dir.path().join("second.rs");
    fs::write(&first, format!("name = {:?}\n", forbidden(&["h2", "m"], ""))).expect("write first fixture");
    fs::write(&second, format!("\n{}\n", forbidden(&["ll", "lm"], ""))).expect("write second fixture");

    let output = run_hook(&[&first, &second]);

    assert!(!output.status.success(), "hook should reject both files");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("first.toml:1:"), "stderr: {stderr}");
    assert!(stderr.contains("second.rs:2:"), "stderr: {stderr}");
}
