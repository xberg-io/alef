//! Compile and run the Zig that the free-function emitter writes for a list function that
//! cannot fail.
//!
//! A stub C layer returns a null list, as the real C function does for an argument that is not
//! valid UTF-8. The Zig tests check that the wrapper returns an error with the native message
//! instead of slicing the null pointer.

use super::errors::emit_error_set;
use super::functions::emit_function;
use super::helpers::emit_helpers;
use crate::core::ir::{ErrorDef, ErrorVariant, FunctionDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// A C layer in Zig. Every call into the library clears the error state first, as
/// `catch_ffi_panic` does in the real FFI crate, so a wrapper that reads the error after a
/// second call sees nothing.
const STUB_C_LAYER: &str = r#"const std = @import("std");

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
    pub fn sample_list_books() [*c]u8 {
        return list_result;
    }
    pub fn sample_list_books_len() usize {
        clear();
        return if (list_result == null) 0 else std.mem.len(list_result);
    }
    pub fn sample_all_books() [*c]u8 {
        return fallible_result;
    }
    pub fn sample_all_books_len() usize {
        return if (fallible_result == null) 0 else std.mem.len(fallible_result);
    }
};

"#;

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

const ZIG_TEST_COUNT: usize = 4;

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

fn list_function(name: &str, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("Book".to_string()))),
        error_type: error_type.map(str::to_string),
        ..FunctionDef::default()
    }
}

/// The Zig source of one list function that cannot fail and one that can, written by the real
/// emitter.
fn generated_source() -> String {
    let error = shelf_error();
    let declared_errors = vec!["ShelfError".to_string()];
    let mut out = String::from(STUB_C_LAYER);
    emit_helpers("sample", std::slice::from_ref(&error), &mut out);
    emit_error_set(&error, &mut out);
    for f in [list_function("list_books", None), list_function("all_books", Some("ShelfError"))] {
        emit_function(
            &f,
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
    out
}

#[test]
fn list_function_that_cannot_fail_checks_the_native_result_for_null() {
    let source = generated_source();
    assert!(
        source.contains("if (_result == null) {\n        return _error_with_message(error{OutOfMemory,UnknownFfiError,Cancelled});"),
        "the result must be checked for null before the length call and the slice:\n{source}"
    );
    let infallible = source.split("pub fn list_books").nth(1).expect("list_books");
    let infallible = infallible.split("pub fn all_books").next().expect("body");
    let check = infallible.find("_result == null").expect("null check");
    let len = infallible.find("_result_len").expect("length call");
    assert!(check < len, "the null check must come before the length call:\n{infallible}");
}

#[test]
fn generated_list_function_returns_the_native_error_at_runtime() {
    if std::process::Command::new("zig").arg("version").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("list.zig");
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
    assert!(output.status.success(), "the generated list function failed:\n{report}");
    assert!(
        report.contains(&format!("All {ZIG_TEST_COUNT} tests passed")),
        "the run must execute every Zig test:\n{report}"
    );
}
