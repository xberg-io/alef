//! Real `tsc` proof for `node_project_root::resolve_isolated_scratch`.
//!
//! `node:path` (unlike `node:fs/promises` and the `Buffer` global) is not one of the finite
//! constructs alef's own templates emit, so `NODE_AMBIENT_DECLARATION_CONTENT` deliberately does
//! not declare it -- that would be re-inventing `@types/node` one construct at a time for
//! arbitrary hand-written snippets, the exact shape `node_project_root`'s module docs argue
//! against. These tests instead prove the "ask the real project" path: a snippet whose own file
//! sits inside a real (fake, for the test) project that has `@types/node` installed now
//! typechecks a builtin alef declares nothing about, while the same import still fails loudly --
//! with `tsc`'s own diagnostic naming what is missing -- when no such project is resolvable. Both
//! halves matter: a fix that only proved the positive case could not rule out a silent downgrade.

use super::TypeScriptValidator;
use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};
use std::path::{Path, PathBuf};

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 30;

/// A hand-written-style snippet whose `path` is a real, on-disk markdown file -- unlike every
/// other test in this module family, which uses the synthetic `"snippet.ts"` `node_project_root`
/// deliberately refuses to treat as real. `code` uses `node:path`'s `join`, a builtin neither
/// `NODE_AMBIENT_DECLARATION_CONTENT` nor any alef template declares anything about.
fn hand_written_node_path_snippet(markdown_path: PathBuf) -> Snippet {
    Snippet {
        id: None,
        path: markdown_path.clone(),
        language: Language::TypeScript,
        title: None,
        code: "import { join } from \"node:path\";\n\
               const result: string = join(\"a\", \"b\");\n\
               console.log(result);\n"
            .to_string(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: markdown_path,
            line: 1,
            block_index: 0,
        },
    }
}

/// Writes a minimal fake `@types/node` package under `root/node_modules/@types/node` -- enough
/// for TypeScript's automatic type-acquisition to pick it up and resolve `node:path`, without
/// depending on a real `@types/node` being installed anywhere on the machine running this test.
/// Mirrors the existing convention elsewhere in this validator (see
/// `project_manifest_resolves_declared_local_package_and_replaces_stale_source` in
/// `typescript.rs`), which writes a fake `node_modules` package for the same reason: the fact
/// being tested is resolution behavior, not any particular package's real contents.
fn write_fake_types_node(root: &Path) {
    let package = root.join("node_modules/@types/node");
    std::fs::create_dir_all(&package).expect("fake @types/node directory");
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@types/node","types":"index.d.ts"}"#,
    )
    .expect("fake @types/node package.json");
    std::fs::write(
        package.join("index.d.ts"),
        "declare module \"node:path\" {\n  function join(...paths: string[]): string;\n}\n",
    )
    .expect("fake @types/node index.d.ts");
}

/// Writes a real markdown file at `root/docs/example.md` and returns its path -- the anchor
/// `node_project_root::real_snippet_directory` needs to resolve anything at all.
fn write_real_markdown_snippet_file(root: &Path) -> PathBuf {
    let markdown = root.join("docs/example.md");
    std::fs::create_dir_all(markdown.parent().unwrap()).expect("docs directory");
    std::fs::write(&markdown, "example doc source").expect("markdown source file");
    markdown
}

/// The negative control this fix must not regress: with no ancestor `@types/node` to resolve,
/// a builtin alef declares nothing about must still fail, loudly, with `tsc`'s own diagnostic --
/// not silently pass and not silently downgrade to a skipped check.
#[test]
fn an_uncovered_node_builtin_fails_loudly_with_no_resolvable_project() {
    if !super::tsc_is_runnable() {
        return;
    }
    let project = tempfile::tempdir().expect("project root");
    let markdown = write_real_markdown_snippet_file(project.path());
    let snippet = hand_written_node_path_snippet(markdown);

    let (status, diagnostics) = TypeScriptValidator::validate_with_context(
        &snippet,
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("validation runs");

    assert_eq!(status, SnippetStatus::Fail);
    let diagnostics = diagnostics.expect("a failed check carries diagnostics");
    assert!(
        !diagnostics.trim().is_empty(),
        "a failure must name what tsc actually reported, not an empty message"
    );
}

/// The positive proof: the same import now typechecks once its own real file resolves to a
/// project with `@types/node` installed -- without alef declaring anything about `node:path`
/// itself, unlike the `Buffer`/`node:fs/promises` ambient declaration cases.
#[test]
fn an_uncovered_node_builtin_typechecks_when_the_real_project_has_types_node_installed() {
    if !super::tsc_is_runnable() {
        return;
    }
    let project = tempfile::tempdir().expect("project root");
    write_fake_types_node(project.path());
    let markdown = write_real_markdown_snippet_file(project.path());
    let snippet = hand_written_node_path_snippet(markdown);

    let (status, diagnostics) = TypeScriptValidator::validate_with_context(
        &snippet,
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("validation runs");

    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// The same proof through the batch path real docs-snippet checks actually use, mirroring
/// `node_fs_promises_typechecks_in_a_batch_without_a_types_node_dependency` in
/// `node_ambient_declaration_tsc_tests.rs`.
#[test]
fn an_uncovered_node_builtin_typechecks_in_a_batch_when_the_real_project_has_types_node_installed() {
    if !super::tsc_is_runnable() {
        return;
    }
    let project = tempfile::tempdir().expect("project root");
    write_fake_types_node(project.path());
    let markdown = write_real_markdown_snippet_file(project.path());
    let snippet = hand_written_node_path_snippet(markdown);

    let results = TypeScriptValidator::validate_batch_with_context(
        &[&snippet],
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results, vec![(SnippetStatus::Pass, None)], "{results:?}");
}
