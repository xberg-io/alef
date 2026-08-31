//! Real `tsc` proof that `NODE_AMBIENT_DECLARATION_FILE_NAME` fixes the `TS2591` family reported
//! against alef-generated TypeScript docs snippets targeting packages with no reachable
//! `@types/node` (a WASM/browser binding has no reason to depend on Node's types at all).
//!
//! `typescript/docs_file_expression.jinja` and `typescript/docs_file_assignment.jinja` emit
//! `await import("node:fs/promises")` into every TypeScript docs snippet that reads a fixture
//! file from disk, regardless of target. When the checked package has no `@types/node`
//! resolvable, `tsc` does not report the usual "cannot find module" for the unresolved
//! `node:`-prefixed specifier -- it degrades the dynamic `import(...)` to a bare identifier
//! lookup and reports `TS2591: Cannot find name 'node:fs/promises'`. These tests reproduce the
//! exact construct the templates emit and prove it now typechecks without any `@types/node`
//! anywhere on the toolchain's resolution path -- these run with `session: None`, the isolated
//! scratch path that (unlike a consumer's own `node_modules`) can never see a real `@types/node`
//! by accident.
//!
//! The `buffer_from_base64_*` tests below cover the sibling gap: `ts_bytes_value_expression`'s
//! base64 branch (`src/e2e/codegen/typescript/test_file/bytes.rs`) emits the bare global
//! `Buffer.from(value, "base64")`, which `tsc` rejects the same way for the same reason --
//! `TS2591: Cannot find name 'Buffer'. Do you need to install type definitions for node?` --
//! `Buffer` being one of the handful of Node globals TypeScript special-cases that hint for.

use super::TypeScriptValidator;
use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 30;

fn snippet(code: &str) -> Snippet {
    Snippet {
        id: None,
        path: "snippet.ts".into(),
        language: Language::TypeScript,
        title: None,
        code: code.into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "snippet.ts".into(),
            line: 1,
            block_index: 0,
        },
    }
}

fn check(code: &str) -> (SnippetStatus, Option<String>) {
    TypeScriptValidator::validate_with_context(
        &snippet(code),
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("validation runs")
}

/// The exact expression form `docs_file_expression.jinja` emits: a dynamic `import()` used
/// inline as a value, immediately awaited and called.
#[test]
fn node_fs_promises_expression_form_typechecks_without_a_types_node_dependency() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "async function main() {
  const bytes = await (await import(\"node:fs/promises\")).readFile(\"fixture.bin\");
  console.log(bytes.length);
}

void main();
",
    );
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// The exact assignment form `docs_file_assignment.jinja` emits: the same dynamic `import()`
/// assigned onto a builder's field, the shape used across every `Wasm*` builder IIFE.
#[test]
fn node_fs_promises_assignment_form_typechecks_without_a_types_node_dependency() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "async function main() {
  const target: { bytes: Uint8Array } = { bytes: new Uint8Array() };
  target.bytes = await (await import(\"node:fs/promises\")).readFile(\"fixture.bin\");
  console.log(target.bytes.length);
}

void main();
",
    );
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// The same construct, through the batch path real docs-snippet checks actually use (`alef e2e
/// generate` validates in batches, not one `tsc` invocation per snippet -- see
/// `TypeScriptValidator::validate_batch_in_session`). Proves the ambient declaration reaches the
/// batch's own `tsconfig.json` `files` list, not just the single-snippet overlay.
#[test]
fn node_fs_promises_typechecks_in_a_batch_without_a_types_node_dependency() {
    if !super::tsc_is_runnable() {
        return;
    }
    let uses_node_import = snippet(
        "async function main() {
  const bytes = await (await import(\"node:fs/promises\")).readFile(\"fixture.bin\");
  console.log(bytes.length);
}

void main();
",
    );
    let results = TypeScriptValidator::validate_batch_with_context(
        &[&uses_node_import],
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results, vec![(SnippetStatus::Pass, None)], "{results:?}");
}

/// The exact construct `ts_bytes_value_expression`'s base64 branch emits
/// (`src/e2e/codegen/typescript/test_file/bytes.rs`): a bare `Buffer.from(value, "base64")` call,
/// assigned into a `Uint8Array`-typed local the way a generated docs snippet assigns it into a
/// builder field.
#[test]
fn buffer_from_base64_typechecks_without_a_types_node_dependency() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "const bytes: Uint8Array = Buffer.from(\"aGVsbG8=\", \"base64\");
console.log(bytes.length);
",
    );
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// The same construct through the batch path real docs-snippet checks actually use -- proves the
/// `Buffer` ambient declaration reaches the batch's own `tsconfig.json` `files` list, not just
/// the single-snippet overlay, mirroring `node_fs_promises_typechecks_in_a_batch_without_a_types_node_dependency`.
#[test]
fn buffer_from_base64_typechecks_in_a_batch_without_a_types_node_dependency() {
    if !super::tsc_is_runnable() {
        return;
    }
    let uses_buffer = snippet(
        "const bytes: Uint8Array = Buffer.from(\"aGVsbG8=\", \"base64\");
console.log(bytes.length);
",
    );
    let results = TypeScriptValidator::validate_batch_with_context(
        &[&uses_buffer],
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("batch validation runs");

    assert_eq!(results, vec![(SnippetStatus::Pass, None)], "{results:?}");
}

/// Negative control: a snippet that never references a Node builtin must still typecheck
/// cleanly -- the ambient declaration alef now always writes is a no-op for code that never
/// mentions `"node:fs/promises"`, so it must never turn an unrelated failure into a false pass
/// or introduce one of its own.
#[test]
fn a_snippet_without_a_node_import_is_unaffected_by_the_ambient_declaration() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "function add(a: number, b: number): number {
  return a + b;
}

console.log(add(1, 2));
",
    );
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// Negative control, other direction: an unrelated genuine type error in a snippet that also has
/// no node import must still fail -- the fix must not paper over real defects.
#[test]
fn an_unrelated_type_error_still_fails_with_the_ambient_declaration_present() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check("const value: number = \"not a number\";\nconsole.log(value);\n");
    assert_eq!(status, SnippetStatus::Fail);
    let diagnostics = diagnostics.expect("a failed check carries diagnostics");
    assert!(
        diagnostics.contains("TS2322"),
        "must fail on the real type error, not something related to the ambient declaration: {diagnostics}"
    );
}
