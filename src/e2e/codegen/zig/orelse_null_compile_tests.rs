//! Regression test: a wire-optional accessor chain must render Zig that actually **compiles**,
//! not just text a snapshot expects.
//!
//! 0.62.7 taught `json_get` (`zig/assertions.rs`) to guard a wire-optional JSON key with
//! `orelse .null` instead of force-unwrapping it with `.?`. That guard compiles fine as the
//! right-hand side of a `const` whose type Zig can infer from the optional's own payload, but
//! breaks the moment the guarded expression is immediately chained into more field access
//! (`(x orelse .null).object.get(...)`) with no intervening declaration to give the `orelse` a
//! result type: Zig's peer-type resolution then has nothing to resolve the bare `.null` enum
//! literal against, and fails with "incompatible types: '*const json.dynamic.Value' and '*const
//! @EnumLiteral()'". That shape is exactly what a wire-optional key produces when it sits in the
//! *middle* of a field path rather than at the leaf (`a.b.c` where `b` is wire-optional), and it
//! shipped uncaught in 0.62.7 because snapshot tests only assert on emitted text, never that the
//! text compiles.
//!
//! This test renders that exact shape and feeds it through the same `zig` toolchain
//! `ZigValidator` (`src/snippets/validators/zig.rs`) uses to compile doc snippets, so a
//! regression here fails a real `zig build-exe`, not a string comparison.
//!
//! Split into its own file rather than added to `zig/assertions.rs`: that file is already over
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test coverage
//! goes into a fresh module instead of growing it. ~keep

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};
use crate::snippets::validators::SnippetValidator;
use crate::snippets::validators::zig::ZigValidator;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

/// Whether `zig` runs, not merely resolves: a version-manager shim spawns fine then exits
/// non-zero, so a PATH-only check would leave the skip below unreachable and fire the assert
/// everywhere Zig is absent. ~keep
fn zig_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("zig")
            .arg("version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Wraps a generated assertion body in a standalone `pub fn main` compiled the same way
/// `ZigValidator` compiles a doc snippet (`zig build-exe -fno-emit-bin`) -- proof the emitted
/// expression is code `zig` accepts, not just text a snapshot expects.
fn wrap_in_main(json_literal: &str, assertion_body: &str) -> String {
    format!(
        "const std = @import(\"std\");\nconst testing = std.testing;\n\npub fn main() !void {{\n    \
         var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);\n    defer arena.deinit();\n    \
         const alloc = arena.allocator();\n    \
         const parsed = try std.json.parseFromSlice(std.json.Value, alloc, \"{json_literal}\", .{{}});\n    \
         const result = parsed.value;\n{assertion_body}}}\n"
    )
}

fn zig_snippet(code: String) -> Snippet {
    Snippet {
        id: None,
        path: PathBuf::from("snippet.zig"),
        language: Language::Zig,
        title: None,
        code,
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: PathBuf::from("snippet.zig"),
            line: 1,
            block_index: 0,
        },
    }
}

/// A wire-optional key (`children`, standing in for a `#[serde(skip_serializing_if = "...")]`
/// field) sitting in the *middle* of a field path (`data.children.count`), asserted `equals`.
/// `json_path_expr_with` chains `.object.get("count").?` directly onto `json_get`'s guarded
/// `children` expression with no intervening `const`, which is exactly the field-access
/// position that broke under a bare `.null` fallback.
#[test]
fn nested_wire_optional_key_assertion_compiles_under_real_zig() {
    if !zig_is_runnable() {
        return;
    }
    let wire_optional: HashSet<String> = ["children".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_wire_optional_fields(wire_optional);
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("data.children.count".to_string()),
        value: Some(serde_json::Value::from(3)),
        ..Assertion::default()
    };
    let mut body = String::new();
    render_json_assertion(&mut body, &assertion, "result", &resolver, false);
    // A loose substring check on purpose: the decisive proof below is a real `zig` compile,
    // not a string comparison -- a snapshot-only assertion here is exactly the kind of check
    // that let 0.62.7 ship this bug uncaught.
    assert!(
        body.contains("orelse"),
        "the fixture must actually exercise the wire-optional guard. Rendered:\n{body}"
    );

    let code = wrap_in_main(r#"{\"data\":{\"children\":{\"count\":3}}}"#, &body);
    let snippet = zig_snippet(code.clone());

    let (status, output) = ZigValidator
        .validate(&snippet, ValidationLevel::Compile, TOOLCHAIN_TEST_TIMEOUT_SECS)
        .expect("validation runs");

    assert_eq!(
        status,
        SnippetStatus::Pass,
        "the generated wire-optional accessor chain must compile under real zig; generated \
         code:\n{code}\ncompiler output:\n{output:?}"
    );
}
