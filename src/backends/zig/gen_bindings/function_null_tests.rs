//! Compile and run the Zig that the free-function emitter writes for a function that cannot
//! fail and returns a list, string, character, path, JSON value or map.
//!
//! A stub C layer returns a null result, as the real C function does for an argument that is
//! not valid UTF-8. The Zig tests check that the wrapper returns an error with the native
//! message instead of slicing the null pointer. Each return kind has its own function, so a
//! kind that drops out of the null check turns its own test red.

use super::errors::emit_error_set;
use super::functions::emit_function;
use super::helpers::emit_helpers;
use crate::core::ir::{ErrorDef, ErrorVariant, FunctionDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// The functions that cannot fail, one for each return kind the null check covers.
fn infallible_functions() -> Vec<(&'static str, TypeRef)> {
    vec![
        ("list_books", TypeRef::Vec(Box::new(TypeRef::Named("Book".to_string())))),
        ("book_title", TypeRef::String),
        ("book_initial", TypeRef::Char),
        ("book_cover", TypeRef::Path),
        ("book_meta", TypeRef::Json),
        (
            "book_counts",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        ),
    ]
}

/// A C layer in Zig. Every call into the library clears the error state first, as
/// `catch_ffi_panic` does in the real FFI crate, so a wrapper that reads the error after a
/// second call sees nothing.
fn stub_c_layer() -> String {
    let mut out = String::from(
        r#"const std = @import("std");

const c = struct {
    pub var code: i32 = 0;
    pub var ctx: [*c]const u8 = null;
    pub var list_result: [*c]u8 = null;
    pub var fallible_result: [*c]u8 = null;
    fn clear() void {
        code = 0;
        ctx = null;
    }
    pub fn sample_last_error_code() i32 {
        return code;
    }
    pub fn sample_last_error_context() [*c]const u8 {
        return ctx;
    }
    pub fn sample_free_string(p: [*c]u8) void {
        _ = p;
    }
    pub fn sample_all_books() [*c]u8 {
        return fallible_result;
    }
    pub fn sample_all_books_len() usize {
        return if (fallible_result == null) 0 else std.mem.len(fallible_result);
    }
"#,
    );
    for (name, _) in infallible_functions() {
        out.push_str(&format!(
            "    pub fn sample_{name}() [*c]u8 {{\n        return list_result;\n    }}\n    pub fn sample_{name}_len() usize {{\n        clear();\n        return if (list_result == null) 0 else std.mem.len(list_result);\n    }}\n"
        ));
    }
    out.push_str("};\n\n");
    out
}

const ZIG_TESTS: &str = r#"
fn reset() void {
    c.code = 0;
    c.ctx = null;
    c.list_result = null;
    c.fallible_result = null;
    if (_captured_error) |_old| std.heap.c_allocator.free(_old);
    _captured_error = null;
}

test "a list function that cannot fail returns an error for a null result and keeps its message" {
    reset();
    c.code = 1;
    c.ctx = "Invalid UTF-8 in parameter 'author'";
    try std.testing.expectError(error.UnknownFfiError, list_books());
    try std.testing.expectEqualStrings("Invalid UTF-8 in parameter 'author'", _last_error().?);
}

test "a list function that cannot fail returns Cancelled for the cancelled code" {
    reset();
    c.code = 5;
    c.ctx = "cancelled by the caller";
    try std.testing.expectError(error.Cancelled, list_books());
}

test "a list function that cannot fail returns its list" {
    reset();
    c.list_result = @ptrCast(@constCast("[1]"));
    const books = try list_books();
    defer std.heap.c_allocator.free(books);
    try std.testing.expectEqualStrings("[1]", books);
}

test "a list function that can fail returns the typed error for a null result" {
    reset();
    c.code = 100;
    c.ctx = "shelf is closed";
    try std.testing.expectError(error.Closed, all_books());
    try std.testing.expectEqualStrings("shelf is closed", _last_error().?);
}
"#;

/// The Zig tests in `ZIG_TESTS`, before the one test for each return kind.
const FIXED_ZIG_TEST_COUNT: usize = 4;

/// One Zig test for each return kind: a null result returns the error and keeps its message.
fn null_result_tests() -> String {
    infallible_functions()
        .iter()
        .map(|(name, _)| {
            format!(
                "\ntest \"{name} returns an error for a null result and keeps its message\" {{\n    reset();\n    c.code = 1;\n    c.ctx = \"Invalid UTF-8 in parameter 'author'\";\n    try std.testing.expectError(error.UnknownFfiError, {name}());\n    try std.testing.expectEqualStrings(\"Invalid UTF-8 in parameter 'author'\", _last_error().?);\n}}\n"
            )
        })
        .collect()
}

fn shelf_error() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "ShelfError".to_string(),
        rust_path: "sample::ShelfError".to_string(),
        variants: vec![ErrorVariant {
            error_code: Some(100),
            name: "Closed".to_string(),
            is_unit: true,
            ..Default::default()
        }],
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn function_returning(name: &str, return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        error_type: error_type.map(str::to_string),
        ..FunctionDef::default()
    }
}

/// The Zig source of every function that cannot fail and one list function that can, written by
/// the real emitter.
fn generated_source() -> String {
    let error = shelf_error();
    let declared_errors = vec!["ShelfError".to_string()];
    let mut out = stub_c_layer();
    emit_helpers("sample", std::slice::from_ref(&error), &mut out);
    emit_error_set(&error, &mut out);
    let mut functions: Vec<FunctionDef> = infallible_functions()
        .into_iter()
        .map(|(name, return_type)| function_returning(name, return_type, None))
        .collect();
    functions.push(function_returning(
        "all_books",
        TypeRef::Vec(Box::new(TypeRef::Named("Book".to_string()))),
        Some("ShelfError"),
    ));
    for f in &functions {
        emit_function(
            f,
            "sample",
            &declared_errors,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &mut out,
        );
    }
    out.push_str(ZIG_TESTS);
    out.push_str(&null_result_tests());
    out
}

#[test]
fn every_function_that_cannot_fail_checks_the_native_result_for_null() {
    let source = generated_source();
    for (name, _) in infallible_functions() {
        let body = source
            .split(&format!("pub fn {name}("))
            .nth(1)
            .unwrap_or_else(|| panic!("{name} missing:\n{source}"));
        let body = body.split("\n}\n").next().expect("body");
        assert!(
            body.contains("if (_result == null) {\n        return _error_with_message(error{OutOfMemory,UnknownFfiError,Cancelled});"),
            "{name} must check the result for null:\n{body}"
        );
        let check = body.find("_result == null").expect("null check");
        let len = body.find("_result_len").expect("length call");
        assert!(
            check < len,
            "the null check must come before the length call in {name}:\n{body}"
        );
    }
}

#[test]
fn generated_functions_that_cannot_fail_return_the_native_error_at_runtime() {
    if std::process::Command::new("zig").arg("version").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("functions.zig");
    std::fs::write(&source_path, generated_source()).expect("write Zig source");
    // `-lc` mirrors the `.link_libc = true` that every generated project sets: the wrappers
    // allocate through `std.heap.c_allocator`. ~keep
    let output = std::process::Command::new("zig")
        .args(["test", "-lc"])
        .arg(&source_path)
        .current_dir(directory.path())
        .output()
        .expect("run Zig compiler");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the generated functions failed:\n{report}");
    assert!(
        report.contains(&format!(
            "All {} tests passed",
            FIXED_ZIG_TEST_COUNT + infallible_functions().len()
        )),
        "the run must execute every Zig test:\n{report}"
    );
}
