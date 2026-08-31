//! Real `tsc` proof for the `wasm` docs-snippet TS18048 family (task #299).
//!
//! ~keep A render-only assertion cannot see `TS18048` -- it can only see that the renderer
//! chose to emit `?.` or not, never whether the choice actually satisfies `strict` TypeScript.
//! That gap is exactly how 31 non-compiling `wasm` snippets shipped past
//! `e2e::codegen::presentation::wasm_optional_leaf_field_tests`-style coverage in the first
//! place: the accessor text can be *checked* without ever being *compiled*. These tests compile
//! the accessor shapes that module's assertions pin, with real `tsc`, so a future change that
//! keeps the string right but the compiler wrong is caught here instead of in a consumer's CI.
//!
//! Reproduces `ProcessResult { data: Option<DataNode> }` / `DataNode { kind, children }`, the
//! shape measured against a released alef in a consumer repo's
//! `data_extraction_*` wasm snippet family. No imported module is declared -- the accessor
//! shape is the only thing under test, so the snippet declares its own ambient types instead of
//! needing a `node_modules` stub package.

use super::TypeScriptValidator;
use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 30;

const AMBIENT_TYPES: &str = "\
interface DataNode {
  kind: string;
  children: DataNode[];
}
interface ProcessResult {
  data?: DataNode;
}
declare function process(source: string, config: unknown): ProcessResult;
";

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

fn check(body: &str) -> (SnippetStatus, Option<String>) {
    let code = format!("{AMBIENT_TYPES}\nfunction main() {{\n{body}\n}}\n\nvoid main();\n");
    TypeScriptValidator::validate_with_context(
        &snippet(&code),
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("validation runs")
}

/// The exact accessor shape `wasm_optional_leaf_field_tests` proves the current codegen emits
/// for an IR-only optional field: the bare field unguarded, and every nested reach through it
/// guarded with `?.`.
#[test]
fn the_guarded_accessor_shape_typechecks() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "  const result = process(\"source\", {});\n\
         \x20 console.log(result.data);\n\
         \x20 console.log(result.data?.kind);\n\
         \x20 console.log(result.data?.children);",
    );
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}

/// The defect as measured: the SAME accessor shape with no `?.` at all, which is precisely what
/// alef 0.67.5 published across the `data_extraction_*` family. Proves the reproduction names a
/// real compiler failure, not just a stylistic difference from the node renderer.
#[test]
fn the_unguarded_accessor_shape_fails_ts18048() {
    if !super::tsc_is_runnable() {
        return;
    }
    let (status, diagnostics) = check(
        "  const result = process(\"source\", {});\n\
         \x20 console.log(result.data);\n\
         \x20 console.log(result.data.kind);\n\
         \x20 console.log(result.data.children);",
    );
    assert_eq!(status, SnippetStatus::Fail);
    let diagnostics = diagnostics.expect("a failed check carries diagnostics");
    assert!(
        diagnostics.contains("TS18048") || diagnostics.contains("TS2532"),
        "must fail on the possibly-undefined access, not some unrelated error: {diagnostics}"
    );
}

/// Negative control: a required (non-optional) field must typecheck WITHOUT `?.`, so a fix that
/// over-applies optional chaining does not merely trade one defect for another.
#[test]
fn a_required_field_typechecks_without_optional_chaining() {
    if !super::tsc_is_runnable() {
        return;
    }
    let code = "\
interface DataNode {
  kind: string;
}
interface ProcessResult {
  data: DataNode;
}
declare function process(source: string, config: unknown): ProcessResult;

function main() {
  const result = process(\"source\", {});
  console.log(result.data.kind);
}

void main();
";
    let (status, diagnostics) = TypeScriptValidator::validate_with_context(
        &snippet(code),
        ValidationLevel::TypeCheck,
        TOOLCHAIN_TEST_TIMEOUT_SECS,
        None,
    )
    .expect("validation runs");
    assert_eq!(status, SnippetStatus::Pass, "diagnostics: {diagnostics:?}");
}
