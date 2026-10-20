//! Tests for the Zig that the constructor and static-function emitters write.
//!
//! The runtime test compiles and runs the output. A stub C layer plays a library whose constructors fail. The Zig tests check the error that
//! each failed call returns and the message the wrapper keeps for `_last_error`.

use super::{OpaqueHandleContext, emit_opaque_constructor, emit_opaque_handle};
use crate::backends::zig::gen_bindings::errors::emit_error_set;
use crate::backends::zig::gen_bindings::helpers::emit_helpers;
use crate::core::config::workspace::{ClientConstructorConfig, ConstructorParam};
use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// A C layer in Zig. Every `*_free` call clears the error state, as `catch_ffi_panic` does in
/// the real FFI crate, so a wrapper that reads the error late sees nothing.
const STUB_C_LAYER: &str = r#"const std = @import("std");

const c = struct {
    pub var code: i32 = 0;
    pub var ctx: [*c]const u8 = null;
    pub var new_result: u64 = 0;
    pub var with_result: u64 = 0;
    pub var infallible_result: u64 = 0;
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
    pub fn sample_greeter_new(g: [*c]const u8) u64 {
        _ = g;
        return new_result;
    }
    pub fn sample_greeter_with_greeting(g: [*c]const u8) u64 {
        _ = g;
        return with_result;
    }
    pub fn sample_greeter_plain() u64 {
        return infallible_result;
    }
    pub fn sample_greeter_free(h: u64) void {
        _ = h;
        clear();
    }
};

"#;

const ZIG_TESTS: &str = r#"
fn reset() void {
    c.code = 0;
    c.ctx = null;
    c.new_result = 0;
    c.with_result = 0;
    c.infallible_result = 0;
    if (_captured_error) |_old| std.heap.c_allocator.free(_old);
    _captured_error = null;
}

test "a failed constructor returns the native error and keeps its message" {
    reset();
    c.code = 100;
    c.ctx = "empty greeting";
    try std.testing.expectError(error.EmptyGreeting, create_greeter(std.testing.allocator, ""));
    try std.testing.expectEqualStrings("empty greeting", _last_error().?);
}

test "a failed static function returns the native error and keeps its message" {
    reset();
    c.code = 100;
    c.ctx = "empty greeting";
    try std.testing.expectError(error.EmptyGreeting, with_greeting_greeter(""));
    try std.testing.expectEqualStrings("empty greeting", _last_error().?);
}

test "a failed constructor with an unknown native code returns UnknownFfiError" {
    reset();
    c.code = 77;
    c.ctx = "unmapped";
    try std.testing.expectError(error.UnknownFfiError, create_greeter(std.testing.allocator, ""));
    try std.testing.expectEqualStrings("unmapped", _last_error().?);
}

test "a failed static function with an unknown native code returns UnknownFfiError" {
    reset();
    c.code = 77;
    c.ctx = "unmapped";
    try std.testing.expectError(error.UnknownFfiError, with_greeting_greeter(""));
    try std.testing.expectEqualStrings("unmapped", _last_error().?);
}

test "a cancelled constructor returns Cancelled" {
    reset();
    c.code = 5;
    c.ctx = "cancelled by the caller";
    try std.testing.expectError(error.Cancelled, create_greeter(std.testing.allocator, ""));
}

test "a constructor and a static function that succeed return a handle" {
    reset();
    c.new_result = 11;
    c.with_result = 12;
    const made = try create_greeter(std.testing.allocator, "hello");
    try std.testing.expectEqual(@as(u64, 11), made._handle);
    const other = try with_greeting_greeter("hello");
    try std.testing.expectEqual(@as(u64, 12), other._handle);
}

test "a failed static function that declares no error type returns UnknownFfiError" {
    reset();
    c.code = 100;
    c.ctx = "ignored";
    try std.testing.expectError(error.UnknownFfiError, plain_greeter());
}
"#;

const ZIG_TEST_COUNT: usize = 7;

fn greet_error() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "GreetError".to_string(),
        rust_path: "sample::GreetError".to_string(),
        variants: vec![ErrorVariant {
            error_code: Some(100),
            name: "EmptyGreeting".to_string(),
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

fn static_method(name: &str, params: Vec<ParamDef>, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        is_static: true,
        params,
        return_type: TypeRef::Named("Greeter".to_string()),
        error_type: error_type.map(str::to_string),
        ..MethodDef::default()
    }
}

/// The Zig source of a handle type with a constructor and two static functions, written by the
/// real emitters.
fn generated_source() -> String {
    let error = greet_error();
    let greeting = ParamDef {
        name: "greeting".to_string(),
        ty: TypeRef::String,
        ..ParamDef::default()
    };
    let ty = TypeDef {
        name: "Greeter".to_string(),
        is_opaque: true,
        methods: vec![
            static_method("with_greeting", vec![greeting], Some("GreetError")),
            static_method("plain", Vec::new(), None),
        ],
        ..TypeDef::default()
    };
    let ctor = ClientConstructorConfig {
        params: vec![ConstructorParam {
            name: "greeting".to_string(),
            ty: "*const c_char".to_string(),
        }],
        body: String::new(),
        error_type: Some("GreetError".to_string()),
    };
    let declared_errors = vec!["GreetError".to_string()];

    let mut out = String::from(STUB_C_LAYER);
    emit_helpers("sample", std::slice::from_ref(&error), &mut out);
    emit_error_set(&error, &mut out);
    emit_opaque_handle(
        &ty,
        "sample",
        &OpaqueHandleContext {
            declared_errors: &declared_errors,
            struct_names: &HashSet::new(),
            streaming_item_types: &HashMap::new(),
            enum_names: &HashSet::new(),
        },
        &mut out,
    );
    emit_opaque_constructor(&ty, "sample", &ctor, &HashSet::new(), &declared_errors, &mut out);
    out.push_str(ZIG_TESTS);
    out
}

#[test]
fn generated_constructor_and_static_function_return_the_native_error_at_runtime() {
    if std::process::Command::new("zig").arg("version").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("creation.zig");
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
    assert!(
        output.status.success(),
        "the generated constructor and static function failed:\n{report}"
    );
    assert!(
        report.contains(&format!("All {ZIG_TEST_COUNT} tests passed")),
        "the run must execute every Zig test:\n{report}"
    );
}

#[test]
fn failed_constructor_and_static_function_read_the_native_error() {
    let source = generated_source();
    assert!(
        source.contains("if (_handle == 0) return _error_with_message(GreetError);"),
        "a null handle must return the typed native error:\n{source}"
    );
    assert_eq!(
        source.matches("return _error_with_message(GreetError);").count(),
        2,
        "the constructor and the fallible static function each read the native error:\n{source}"
    );
    assert!(
        source.contains("pub fn with_greeting_greeter(greeting: []const u8) (GreetError||error{OutOfMemory})!Greeter {"),
        "the declared error set must admit the typed error:\n{source}"
    );
}
