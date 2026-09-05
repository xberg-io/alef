use super::*;
use std::fs;
use std::path::Path;

#[test]
fn docs_config_merges_workspace_and_crate_layers() {
    let cfg = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs]
reference_output = "docs/reference"

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.skills]
template_dir = "templates/skills"
outputs = [".codex/skills"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.docs.llms]
output = "llms.txt"
adopt_existing = true
"#,
    );

    let docs = cfg.docs.expect("docs config resolved");
    assert_eq!(
        docs.reference_output.as_deref(),
        Some(std::path::Path::new("docs/reference"))
    );
    assert_eq!(
        docs.llms.as_ref().and_then(|llms| llms.template.as_deref()),
        Some(std::path::Path::new("templates/llms.txt.jinja"))
    );
    assert_eq!(
        docs.llms.as_ref().and_then(|llms| llms.output.as_deref()),
        Some(std::path::Path::new("llms.txt"))
    );
    assert!(docs.llms.unwrap().adopt_existing);
    assert_eq!(
        docs.skills.unwrap().outputs,
        vec![std::path::PathBuf::from(".codex/skills")]
    );
}

#[test]
fn generate_docs_stage_renders_reference_llms_and_skills_from_templates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates/skills/api")).unwrap();
    fs::create_dir_all(root.join("templates/skills/cli")).unwrap();
    fs::create_dir_all(root.join("templates/skills/mcp")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/python")).unwrap();

    fs::write(
        root.join("src/cli.rs"),
        r#"
        use clap::{Parser, Subcommand};
        #[derive(Parser)]
        #[command(name = "mylib", about = "My CLI")]
        struct Cli {
            #[command(subcommand)]
            command: Commands,
        }
        #[derive(Subcommand)]
        enum Commands {
            /// Run the thing.
            Run {
                /// Input file
                input: String,
                /// Output file
                #[arg(short, long, value_name = "FILE")]
                output: Option<String>,
            },
        }
        "#,
    )
    .unwrap();
    fs::write(
        root.join("src/mcp.rs"),
        r#"
        struct Server;
        #[tool_router]
        impl Server {
            #[tool(description = "Run MCP work", annotations(title = "Run Work", read_only_hint = true))]
            async fn run_work(&self, Parameters(params): Parameters<crate::Params>) {}
        }
        "#,
    )
    .unwrap();
    fs::write(
        root.join("templates/llms.txt.jinja"),
        "Project {{ krate.name }} has {{ cli.commands | length }} CLI root and {{ mcp.tools | length }} MCP tools.\n{{ \"example.md\" | include_snippet(\"python\") }}\n",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/api/SKILL.md.jinja"),
        "# API Skill\n\n{% for ref in api_references %}- {{ ref.path }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/cli/SKILL.md.jinja"),
        "# CLI Skill\n\n{% for command in cli.commands %}- {{ command.path }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/mcp/SKILL.md.jinja"),
        "# MCP Skill\n\n{% for tool in mcp.tools %}- {{ tool.name }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("docs/snippets/python/example.md"),
        "```python\nprint('ok')\n```\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.cli]
sources = ["src/cli.rs"]

[workspace.docs.mcp]
sources = ["src/mcp.rs"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.skills]
template_dir = "templates/skills"
outputs = [".codex/skills"]

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    result.unwrap();
    let paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(paths.contains(&"docs/reference/cli.md".to_string()));
    assert!(paths.contains(&"docs/reference/mcp.md".to_string()));
    assert!(paths.contains(&"docs/llms.txt".to_string()));
    assert!(paths.contains(&".codex/skills/api/SKILL.md".to_string()));
    assert!(paths.contains(&".codex/skills/cli/SKILL.md".to_string()));
    assert!(paths.contains(&".codex/skills/mcp/SKILL.md".to_string()));

    for group in ["api", "cli", "mcp"] {
        let path = PathBuf::from(format!(".codex/skills/{group}/SKILL.md"));
        let skill = files.iter().find(|file| file.path == path).unwrap();
        assert!(skill.content.starts_with(&format!("---\nname: \"{group}\"\n")));
        assert!(skill.content.contains("description: \"Use mylib"));
        assert!(skill.content.contains("auto-generated by alef"));
    }

    let llms = files
        .iter()
        .find(|file| file.path == Path::new("docs/llms.txt"))
        .unwrap();
    assert!(llms.content.contains("Project mylib has 1 CLI root and 1 MCP tools"));
    assert!(llms.content.contains("print('ok')"));
    assert!(llms.content.contains("auto-generated by alef"));

    let cli_doc = files
        .iter()
        .find(|file| file.path == Path::new("docs/reference/cli.md"))
        .unwrap();
    assert!(cli_doc.content.starts_with("---\ntitle: \"CLI Reference\"\n---"));
    assert!(
        cli_doc
            .content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"))
    );
    assert!(cli_doc.content.contains("`mylib run`"));
    assert!(cli_doc.content.contains("`--output`"));

    let api_doc = files
        .iter()
        .find(|file| file.path == Path::new("docs/reference/api-python.md"))
        .unwrap();
    assert!(api_doc.content.starts_with("---\n"));
    assert!(
        api_doc
            .content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"))
    );
}

#[test]
fn docs_mcp_declared_entries_render_alongside_attribute_derived_ones_in_mcp_md() {
    // Reproduces the reported gap end to end: a prompt/resource built at runtime via a
    // constructor call (not a `#[prompt]`/`#[resource]` attribute) is invisible to static
    // extraction. `docs.mcp.declared` is the config fallback for exactly that surface, and
    // this proves it actually reaches the rendered `mcp.md` page, additively alongside the
    // one attribute-derived tool. ~keep
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/mcp.rs"),
        r#"
        struct Server;
        #[tool_router]
        impl Server {
            #[tool(description = "Run MCP work")]
            async fn run_work(&self, Parameters(params): Parameters<crate::Params>) {}
        }
        impl Server {
            fn build_prompts(&self) -> Vec<Prompt> {
                vec![Prompt::new("summarize", "Summarize the input")]
            }
        }
        "#,
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.mcp]
sources = ["src/mcp.rs"]

[[workspace.docs.mcp.declared]]
kind = "prompt"
name = "summarize"
description = "Summarize the input"

[[workspace.docs.mcp.declared]]
kind = "resource"
name = "config"
title = "App Config"
description = "App configuration"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    result.unwrap();

    let mcp_doc = files
        .iter()
        .find(|file| file.path == Path::new("docs/reference/mcp.md"))
        .expect("mcp.md must be emitted");

    assert!(
        mcp_doc.content.contains("run_work"),
        "attribute-derived tool must still render"
    );
    assert!(
        mcp_doc.content.contains("summarize"),
        "declared prompt built at runtime must render into mcp.md, not be silently omitted"
    );
    assert!(
        mcp_doc.content.contains("config") && mcp_doc.content.contains("App Config"),
        "declared resource built at runtime must render into mcp.md, not be silently omitted"
    );
}

#[test]
fn should_error_when_configured_docs_snippets_dir_does_not_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("templates/llms.txt.jinja"), "Project {{ krate.name }}\n").unwrap();
    // Note: `docs/snippets` is intentionally never created, mirroring the
    // shipped bug where `docs.snippets.dirs` pointed at a nonexistent path and
    // was silently filtered out instead of failing the build. ~keep

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.expect_err("a configured docs.snippets.dirs entry that does not exist must be a hard error");
    let message = err.to_string();
    assert!(
        message.contains("docs.snippets.dirs"),
        "error must name the config key, got: {message}"
    );
    assert!(
        message.contains("docs/snippets"),
        "error must name the offending configured path, got: {message}"
    );
    // The regression this whole change fixes: a snippet-stage failure must not discard the API
    // reference pages that were already fully rendered before snippet discovery ever ran. ~keep
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "already-rendered API reference pages must survive a later stage failure, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

/// A tree exercising every emitter the docs stage has past the API reference pages: CLI, MCP,
/// llms and skills. Deliberately the full set — a fixture that only reaches some of them makes
/// [`every_page_the_docs_stage_emits_survives_a_later_validation_failure`] pass vacuously. ~keep
fn write_full_docs_stage_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates/skills/api")).unwrap();
    fs::create_dir_all(root.join("templates/skills/cli")).unwrap();
    fs::create_dir_all(root.join("templates/skills/mcp")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/python")).unwrap();

    fs::write(
        root.join("src/cli.rs"),
        r#"
        use clap::{Parser, Subcommand};
        #[derive(Parser)]
        #[command(name = "mylib", about = "My CLI")]
        struct Cli {
            #[command(subcommand)]
            command: Commands,
        }
        #[derive(Subcommand)]
        enum Commands {
            /// Run the thing.
            Run {
                /// Input file
                input: String,
            },
        }
        "#,
    )
    .unwrap();
    fs::write(
        root.join("src/mcp.rs"),
        r#"
        struct Server;
        #[tool_router]
        impl Server {
            #[tool(description = "Run MCP work", annotations(title = "Run Work", read_only_hint = true))]
            async fn run_work(&self, Parameters(params): Parameters<crate::Params>) {}
        }
        "#,
    )
    .unwrap();
    fs::write(root.join("templates/llms.txt.jinja"), "Project {{ krate.name }}\n").unwrap();
    fs::write(root.join("templates/skills/api/SKILL.md.jinja"), "# API Skill\n").unwrap();
    fs::write(root.join("templates/skills/cli/SKILL.md.jinja"), "# CLI Skill\n").unwrap();
    fs::write(root.join("templates/skills/mcp/SKILL.md.jinja"), "# MCP Skill\n").unwrap();
    fs::write(
        root.join("docs/snippets/python/example.md"),
        "```python\nprint('ok')\n```\n",
    )
    .unwrap();
}

/// The fixture's config. `extra_snippet_keys` is spliced into `[workspace.docs.snippets]` so a
/// caller can turn snippet *validation* on or off without touching anything that decides which
/// pages get emitted — which is what makes the two runs below a controlled comparison. ~keep
fn full_docs_stage_config(extra_snippet_keys: &str) -> ResolvedCrateConfig {
    config_from_toml(&format!(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.cli]
sources = ["src/cli.rs"]

[workspace.docs.mcp]
sources = ["src/mcp.rs"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.skills]
template_dir = "templates/skills"
outputs = [".codex/skills"]

[workspace.docs.snippets]
dirs = ["docs/snippets"]
{extra_snippet_keys}
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#
    ))
}

fn emitted_paths(files: &[crate::core::backend::GeneratedFile]) -> Vec<String> {
    let mut paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

/// THE INVARIANT: the set of pages the docs stage emits does not depend on whether a later,
/// non-emitting step succeeded.
///
/// The returned `Vec<GeneratedFile>` is the record every downstream consumer reads to decide what
/// alef owns (`.alef-ownership.toml` via `write_scaffold_files_report`, the orphan sweep's `keep`
/// set, `alef verify`'s frozen report, `alef adopt`'s candidate set), and absence from it is read
/// as "alef no longer emits this". So a page dropped because snippet validation bailed does not
/// merely go unwritten — it is actively reported, everywhere downstream, as an orphan.
///
/// Asserted differentially rather than against a literal page list on purpose. There is no way to
/// derive "every page this stage emits" in a test without re-implementing the stage, and a list of
/// names (`cli.md`, `mcp.md`, ...) would pin today's emitters and say nothing about the next one
/// added after the fallible step — which is exactly how the current gap arose. Comparing a clean
/// run against a run that differs *only* in whether snippet validation fails catches any such
/// page, named or not.
///
/// What it does not catch: a page dropped by a failure *before* the emitters (a nonexistent
/// `docs.snippets.dirs` root still bails in discovery, which llms/skills genuinely depend on), and
/// anything about whether the emitted paths are subsequently written or recorded — that happens in
/// `cli::pipeline::generate`, past this stage's boundary. ~keep
#[test]
fn every_page_the_docs_stage_emits_survives_a_later_validation_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_full_docs_stage_fixture(root);
    let api = make_minimal_api("1.0.0");

    let clean_config = full_docs_stage_config("");
    let (clean_files, clean_result) = generate_docs_stage(&api, &clean_config, &[Language::Python], None, root);
    clean_result.expect("the fixture must validate cleanly when no snippet gap is required");

    // `required_languages` makes gap detection run and fail (the corpus has a python variant and
    // no go one). It is a pure validation knob: it reaches no emitter, so any difference in the
    // emitted set between these two runs is the bug, not the fixture. ~keep
    let failing_config = full_docs_stage_config("required_languages = [\"go\"]\n");
    let (failing_files, failing_result) = generate_docs_stage(&api, &failing_config, &[Language::Python], None, root);
    failing_result.expect_err("a required-language snippet gap must still fail the docs stage");

    // Apparatus check, not the invariant: if the fixture reached no emitter past the API reference
    // pages, both sets would be equal for the wrong reason and the assertion below would be
    // vacuous. Compared against `generate_docs` rather than a page count so it stays honest as the
    // reference-page set changes. ~keep
    let reference_pages =
        generate_docs(&api, &clean_config, &[Language::Python], "docs/reference").expect("reference pages render");
    assert!(
        clean_files.len() > reference_pages.len(),
        "fixture must exercise emitters past the API reference pages, or this test proves nothing: \
         {} emitted vs {} reference pages",
        clean_files.len(),
        reference_pages.len()
    );

    assert_eq!(
        emitted_paths(&clean_files),
        emitted_paths(&failing_files),
        "a snippet-validation failure must not change which pages the docs stage emits -- every \
         page missing from the failing run is reported downstream as an orphan alef no longer emits"
    );
}

#[test]
fn should_error_with_actionable_message_when_snippet_reference_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/python")).unwrap();
    fs::write(
        root.join("templates/llms.txt.jinja"),
        "Project {{ krate.name }}\n{{ \"missing.py\" | include_snippet(\"python\") }}\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.expect_err("an unresolvable snippet reference must fail, not silently omit content");
    let message = err.to_string();
    assert!(
        message.contains("python"),
        "error must name the language, got: {message}"
    );
    assert!(
        message.contains("missing.py"),
        "error must name the path, got: {message}"
    );
    assert!(
        message.contains("docs/snippets"),
        "error must name the root(s) that were searched, got: {message}"
    );
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "already-rendered API reference pages must survive a later llms-render failure, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

/// A `SKILL.md.jinja` whose author forgot the blank line after `# Title` folds the description
/// paragraph onto the same heading line -- exactly the shape `poly lint` flags as `MD022`
/// (heading needs a blank line after it) and `MD026` (trailing punctuation in a heading), because
/// the fused line ends mid-sentence. Before `validate_leading_heading` existed this rendered
/// successfully and shipped the fused heading verbatim; it must now fail loudly instead. ~keep
#[test]
fn should_error_with_actionable_message_when_skill_heading_fuses_title_and_description() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates/skills/api")).unwrap();
    fs::write(
        root.join("templates/skills/api/SKILL.md.jinja"),
        "# {{ krate.name }} API Reference Use this skill when working with {{ krate.name }} \
         generated language APIs, data types, configuration, or error surfaces. Generated \
         reference pages:\n{% for ref in api_references %}- {{ ref.path }}\n{% endfor %}",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.skills]
outputs = [".codex/skills"]

[workspace.docs.skills.templates.api]
template = "templates/skills/api/SKILL.md.jinja"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (_files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.expect_err(
        "a heading that fuses the title and description onto one line must fail, not silently \
         ship malformed markdown",
    );
    let message = err.to_string();
    assert!(
        message.contains("trailing punctuation"),
        "error must explain the MD026 violation, got: {message}"
    );
    assert!(
        message.contains("templates/skills/api/SKILL.md.jinja"),
        "error must name the offending template, got: {message}"
    );
}

/// The corrected shape of the fixture above: the title is its own heading line, a blank line
/// separates it from the description paragraph, and the heading itself carries no trailing
/// punctuation. This must render cleanly, and the emitted heading line must not have absorbed
/// any of the description text. ~keep
#[test]
fn skill_heading_with_title_on_its_own_line_renders_with_blank_line_before_description() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates/skills/api")).unwrap();
    fs::write(
        root.join("templates/skills/api/SKILL.md.jinja"),
        "# {{ krate.name }} API Reference\n\nUse this skill when working with {{ krate.name }} \
         generated language APIs, data types, configuration, or error surfaces.\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.skills]
outputs = [".codex/skills"]

[workspace.docs.skills.templates.api]
template = "templates/skills/api/SKILL.md.jinja"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    result.unwrap();
    let skill = files
        .iter()
        .find(|file| file.path == Path::new(".codex/skills/api/SKILL.md"))
        .expect("rendered skill file must be emitted");
    let content_lines: Vec<&str> = skill.content.lines().collect();
    let heading_index = content_lines
        .iter()
        .position(|line| line.starts_with("# "))
        .expect("rendered skill must contain a top-level heading");
    let heading_line = content_lines[heading_index];
    assert!(
        !heading_line.contains("Use this skill"),
        "the heading line must not absorb the description paragraph, got: {heading_line}"
    );
    let next_line = content_lines.get(heading_index + 1).copied().unwrap_or("");
    assert!(
        next_line.trim().is_empty(),
        "a blank line must separate the heading from the description paragraph, got: {next_line:?}"
    );
}

fn reference_output_config() -> ResolvedCrateConfig {
    config_from_toml(
        r#"
[workspace]
languages = ["python", "ffi"]

[workspace.docs]
reference_output = "docs-site/src/content/docs/reference"

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "literllm"
"#,
    )
}

/// A previously published reference page, carrying the `<!-- alef:hash: -->` line the docs
/// stage stamps into every page it emits, must be committed to disk. ~keep
fn stale_reference_page(version: &str) -> String {
    format!(
        "---\ntitle: \"C API Reference\"\n---\n\n\
         <!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
         <!-- alef:hash:0000000000000000000000000000000000000000000000000000000000000000 -->\n\n\
         ## C API Reference <span class=\"version-badge\">v{version}</span>\n"
    )
}

/// A full generation must land every API-reference page on disk, in the configured
/// `[docs] reference_output`.
///
/// The regression: `alef all --clean` regenerated the bindings and the 2821 committed
/// snippets but left all 15 `api-*.md` pages at the previous release's content, so the
/// published reference contradicted the published snippets on the same site. This drives the
/// real writer rather than only inspecting the returned `GeneratedFile`s, because the pages
/// went missing between emission and disk, not during rendering. ~keep
#[test]
fn full_generation_writes_api_reference_pages_into_configured_output() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let reference_output = root.join("docs-site/src/content/docs/reference");

    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python, Language::Ffi], None, root);
    result.unwrap();
    let written = crate::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();
    assert!(written > 0, "the docs stage must write files, wrote {written}");

    for page in ["api-python.md", "api-c.md", "configuration.md", "types.md", "errors.md"] {
        let path = reference_output.join(page);
        let content =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("{} was not written: {err}", path.display()));
        assert!(
            content.contains("auto-generated by alef"),
            "{page} must carry the generated-file header:\n{content}"
        );
    }
}

/// Regenerating over the previously published page must replace it, not preserve it.
///
/// `.md` sits outside `marker_comment_style`'s extension allowlist, so a reference page can
/// only ever be proved alef-owned by the `<!-- alef:hash: -->` line `finalize_hashes` stamps
/// into its *content* -- which every published page carries. An ownership guard that ignores
/// that in-content evidence for unstampable extensions and consults only the machine-local
/// ownership record (empty on a fresh clone and in cache-less CI) refuses every reference page
/// forever, which is exactly the staleness this suite exists to prevent. ~keep
#[test]
fn stale_api_reference_page_is_regenerated_not_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let reference_output = root.join("docs-site/src/content/docs/reference");
    fs::create_dir_all(&reference_output).unwrap();
    fs::write(reference_output.join("api-c.md"), stale_reference_page("1.16.0")).unwrap();

    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Ffi], None, root);
    result.unwrap();
    crate::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();

    let page = fs::read_to_string(reference_output.join("api-c.md")).unwrap();
    assert!(
        page.contains("<span class=\"version-badge\">v1.17.1</span>"),
        "the page must be regenerated at the current version:\n{page}"
    );
    assert!(
        !page.contains("v1.16.0"),
        "the previous release's content must not survive a full regeneration:\n{page}"
    );
}

/// Every emitted page must badge the resolved workspace version that produced it.
#[test]
fn generated_api_reference_badges_the_resolved_version() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python, Language::Ffi], None, root);
    result.unwrap();

    for slug in ["api-python.md", "api-c.md"] {
        let page = files
            .iter()
            .find(|file| file.path.file_name().is_some_and(|name| name.to_string_lossy() == slug))
            .unwrap_or_else(|| panic!("missing {slug}"));
        assert!(
            page.content.contains("<span class=\"version-badge\">v1.17.1</span>"),
            "{slug} must badge the resolved version:\n{}",
            page.content
        );
    }
}

/// The C page must spell symbols exactly as the generated header declares them.
///
/// `[ffi] prefix = "literllm"` makes the FFI backend set cbindgen's `[export] prefix` to
/// `LITERLLM`, so `liter_llm.h` declares `LITERLLMDefaultClient`. The reference page used to
/// publish `LiterllmDefaultClient`, a spelling that occurs zero times in the emitted header. ~keep
#[test]
fn generated_c_reference_uses_the_header_symbol_spelling() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = reference_output_config();
    let export_prefix = config.ffi_prefix().to_uppercase();

    let mut api = make_minimal_api("1.17.1");
    api.types = vec![empty_type("DefaultClient")];
    api.functions = vec![make_function(
        "create_client",
        vec![],
        TypeRef::Named("DefaultClient".to_string()),
        false,
        None,
    )];

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Ffi], None, root);
    result.unwrap();
    let page = files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == "api-c.md")
        })
        .expect("missing api-c.md");

    assert!(
        page.content.contains(&format!("{export_prefix}DefaultClient")),
        "C page must use the cbindgen export prefix `{export_prefix}`:\n{}",
        page.content
    );
    assert!(
        !page.content.contains("LiterllmDefaultClient"),
        "the PascalCase prefix appears nowhere in the generated header:\n{}",
        page.content
    );
}

#[test]
fn generate_docs_stage_rejects_unmanaged_llms_output_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("templates/llms.txt.jinja"), "Generated {{ krate.name }}\n").unwrap();
    fs::write(root.join("docs/llms.txt"), "human authored\n").unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not Alef-managed"));
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "an unmanaged llms.txt must not discard the already-rendered API reference pages, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

/// THE STRUCTURAL PROOF for `bin_cli::helpers::collect_managed_surface`'s ownership-only docs
/// call (`generate_docs_stage_without_snippet_compile_validation`): it must never reach
/// `validate_snippets`'s compile/type-check/run block at all, not merely run it against an
/// empty corpus or run it faster.
///
/// A wall-clock assertion would be worthless here -- it would pass by accident on a machine
/// with every referenced toolchain missing, which is the exact false-green class this guards
/// against (see `alef adopt`'s 90-minute regression on one `Cargo.toml`, entirely spent
/// spawning `zig`/`go`/`javac`/`dotnet`/... subprocesses whose result nothing downstream ever
/// read). So this fixture deliberately uses JSON: `JsonValidator::validate` never spawns a
/// process, it parses in-process with `serde_json`, identically on every machine regardless of
/// installed toolchains. The snippet's JSON is syntactically invalid, so `run_validation`
/// reports it `Fail` and `enforce_snippet_summary` bails *unconditionally* on any failure --
/// strict mode is not even required for the assertion to hold.
///
/// `generate_docs_stage` (validation on) proves the fixture genuinely reaches the validator by
/// failing on it. `generate_docs_stage_without_snippet_compile_validation` (validation off)
/// must then succeed on the byte-identical fixture -- the only variable between the two calls
/// is whether the compile-validation block executed, so success there can only mean it was
/// skipped, never that it ran and passed. ~keep
#[test]
fn generate_docs_stage_without_snippet_compile_validation_never_runs_the_validator() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/json")).unwrap();
    fs::write(
        root.join("docs/snippets/json/example.md"),
        "```json\n{ this is not valid json\n```\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.snippets]
dirs = ["docs/snippets"]
validation_level = "syntax"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (validated_files, validated_result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = validated_result.expect_err(
        "the invalid JSON snippet must fail validation when the compile-validation block actually \
         runs -- if this passes, the fixture is not exercising the validator at all and proves \
         nothing below",
    );
    assert!(
        err.to_string().contains("snippet validation failed"),
        "expected a snippet-validation failure naming the invalid JSON, got: {err:#}"
    );

    let (skipped_files, skipped_result) =
        generate_docs_stage_without_snippet_compile_validation(&api, &config, &[Language::Python], None, root);
    skipped_result.expect(
        "the ownership-only variant must never invoke the same invalid-JSON snippet's validator -- \
         its failure above proves this fixture reaches the validator when it runs, so success here \
         can only mean the compile-validation block was skipped entirely",
    );

    assert_eq!(
        emitted_paths(&validated_files),
        emitted_paths(&skipped_files),
        "skipping snippet compile validation must not change which files the docs stage owns"
    );
}
