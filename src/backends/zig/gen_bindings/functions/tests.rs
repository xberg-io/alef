use super::*;

#[test]
fn optional_opaque_handle_return_wraps_into_struct_instead_of_bare_raw_value() {
    let expr = unwrap_return_expr(
        "_result",
        &TypeRef::Optional(Box::new(TypeRef::Named("NodeHandle".to_string()))),
        "sample",
        &std::collections::HashSet::new(),
        None,
    );

    assert_eq!(expr, "if (_result == 0) null else NodeHandle{ ._handle = _result }");
}

/// Regression test: `Char` crosses the C ABI exactly like `String` (a `*mut c_char` +
/// `_len()` companion, see `returns_c_char` in the FFI backend), so the wrapper must copy
/// and free it the same way instead of falling through to a bare passthrough of a pointer
/// the declared `[]u8` return type cannot represent. ~keep
#[test]
fn char_return_copies_and_frees_like_string_instead_of_bare_passthrough() {
    let expr = unwrap_return_expr(
        "_result",
        &TypeRef::Char,
        "sample",
        &std::collections::HashSet::new(),
        None,
    );

    assert!(expr.contains("_free_string(_result)"), "{expr}");
    assert!(expr.contains("std.heap.c_allocator.dupe(u8, slice)"), "{expr}");
    assert_ne!(expr, "_result");
}

#[test]
fn optional_char_return_degrades_to_null_instead_of_bare_passthrough() {
    let expr = unwrap_return_expr(
        "_result",
        &TypeRef::Optional(Box::new(TypeRef::Char)),
        "sample",
        &std::collections::HashSet::new(),
        None,
    );

    assert!(expr.contains("if (_result == null) break :blk null;"), "{expr}");
    assert!(expr.contains("_free_string(_result)"), "{expr}");
    assert_ne!(expr, "_result");
}

#[test]
fn zig_return_type_char_is_owned_slice_not_field_style_const_slice() {
    assert_eq!(
        zig_return_type(&TypeRef::Char, &std::collections::HashSet::new()),
        "[]u8"
    );
    assert_eq!(
        zig_return_type(
            &TypeRef::Optional(Box::new(TypeRef::Char)),
            &std::collections::HashSet::new()
        ),
        "?[]u8"
    );
}

#[test]
fn return_uses_len_companion_covers_char_like_string() {
    assert!(return_uses_len_companion(&TypeRef::Char));
    assert!(return_uses_len_companion(&TypeRef::Optional(Box::new(TypeRef::Char))));
}

/// `Optional<Bytes>` gets no `_len()` companion (unlike Optional<String/Char/Path/Json/Vec/Map>)
/// and its `None` case never sets the FFI last-error state, so treating its null as an
/// FFI-call error here would misclassify a legitimate `None` as failure. This must stay
/// `false` even though bare `Bytes` (routed entirely through the separate out-param
/// convention) is `true`. ~keep
#[test]
fn return_type_can_be_null_excludes_optional_bytes_but_includes_optional_char() {
    assert!(!return_type_can_be_null(
        &TypeRef::Optional(Box::new(TypeRef::Bytes)),
        &std::collections::HashSet::new()
    ));
    assert!(return_type_can_be_null(
        &TypeRef::Optional(Box::new(TypeRef::Char)),
        &std::collections::HashSet::new()
    ));
}

fn char_return_fn() -> FunctionDef {
    FunctionDef {
        name: "first_char".to_string(),
        rust_path: "sample::first_char".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Char,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// End-to-end regression: every piece touched by the `Char` return-shape fix must agree —
/// the `_len()` companion call, the `[]u8` declared type, the `OutOfMemory` error set (the
/// generated body's `try std.heap.c_allocator.dupe` requires it even with no declared Rust
/// error type), and the owned-copy-then-free body. Each was a separate missing arm before
/// this fix; asserting them together in one generated function is what proves the whole
/// pipeline — not just one function in isolation — now agrees for `Char`. ~keep
#[test]
fn emit_function_char_return_wires_len_companion_error_set_and_owned_copy_together() {
    let f = char_return_fn();
    let mut out = String::new();
    emit_function(
        &f,
        "sample",
        &[],
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &mut out,
    );

    assert!(
        out.contains("error{OutOfMemory,UnknownFfiError,Cancelled}![]u8"),
        "Char return must declare an owned []u8 in an error set that holds OutOfMemory and the native error. Got:\n{out}"
    );
    assert!(
        out.contains("c.sample_first_char_len("),
        "Char return must call its FFI _len() companion. Got:\n{out}"
    );
    assert!(
        out.contains("_free_string(_result)"),
        "Char return must free the FFI allocation. Got:\n{out}"
    );
    assert!(
        out.contains("std.heap.c_allocator.dupe(u8, slice)"),
        "Char return must copy into an owned, caller-freed slice. Got:\n{out}"
    );
    assert!(
        !out.contains("return _result;"),
        "Char return must not fall through to a bare pointer passthrough. Got:\n{out}"
    );
}

fn bytes_return_fn(return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: "maybe_thumbnail".to_string(),
        rust_path: "sample::maybe_thumbnail".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn emit_bytes_fn(return_type: TypeRef) -> String {
    let f = bytes_return_fn(return_type);
    let mut out = String::new();
    emit_function(
        &f,
        "sample",
        &[],
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &mut out,
    );
    out
}

#[test]
fn zig_return_type_optional_bytes_is_an_owned_optional_slice() {
    assert_eq!(
        zig_return_type(
            &TypeRef::Optional(Box::new(TypeRef::Bytes)),
            &std::collections::HashSet::new()
        ),
        "?[]u8"
    );
}

/// The `Char` fix routed through the `_len()` companion because a NUL-terminated
/// `c_char` has a recoverable length. `Bytes` has no terminator, so `Optional<Bytes>`
/// must take the out-param convention instead — and must specifically NOT acquire a
/// `_len()` companion, which the FFI backend does not emit for it. ~keep
#[test]
fn optional_bytes_takes_out_params_not_the_len_companion_char_uses() {
    assert!(return_uses_bytes_out_params(&TypeRef::Bytes));
    assert!(return_uses_bytes_out_params(&TypeRef::Optional(Box::new(
        TypeRef::Bytes
    ))));
    assert!(!return_uses_bytes_out_params(&TypeRef::Optional(Box::new(
        TypeRef::Char
    ))));
    assert!(!return_uses_len_companion(&TypeRef::Optional(Box::new(TypeRef::Bytes))));
}

/// End-to-end: the declared `?[]u8` and the emitted body must agree. The body reads the
/// length from `_out_len` (there is no `_result_len` to read — `Optional<Bytes>` has no
/// `_len()` companion) and treats a null `_out_ptr` as the `None` encoding. ~keep
#[test]
fn emit_function_optional_bytes_return_reads_out_len_and_maps_null_ptr_to_null() {
    let out = emit_bytes_fn(TypeRef::Optional(Box::new(TypeRef::Bytes)));

    assert!(
        out.contains("error{OutOfMemory}!?[]u8"),
        "Optional<Bytes> must declare an owned ?[]u8. Got:\n{out}"
    );
    assert!(
        out.contains("    var _out_ptr: [*c]u8 = undefined;\n"),
        "Optional<Bytes> must declare the byte-buffer out-param locals. Got:\n{out}"
    );
    assert!(
        out.contains("    _ = c.sample_maybe_thumbnail(&_out_ptr, &_out_len, &_out_cap);\n"),
        "Optional<Bytes> must pass the three out-params and discard the status. Got:\n{out}"
    );
    assert!(
        out.contains("    if (_out_ptr == null) return null;\n"),
        "Optional<Bytes> must map a null out_ptr to Zig null. Got:\n{out}"
    );
    assert!(
        out.contains("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n"),
        "Optional<Bytes> must copy _out_len bytes into an owned slice. Got:\n{out}"
    );
    assert!(
        out.contains("    c.sample_free_bytes(_out_ptr, _out_len, _out_cap);\n"),
        "Optional<Bytes> must release the FFI buffer via free_bytes. Got:\n{out}"
    );
    assert!(
        !out.contains("_result_len"),
        "Optional<Bytes> must not reference the _len() companion's _result_len. Got:\n{out}"
    );
    assert!(
        !out.contains("c.sample_maybe_thumbnail_len("),
        "Optional<Bytes> must not call a _len() companion the FFI side never emits. Got:\n{out}"
    );
    assert!(
        !out.contains("return _result;"),
        "Optional<Bytes> must not fall through to a bare pointer passthrough. Got:\n{out}"
    );
}

/// Present-but-empty (`Some(&[])`) must not be read as absent. The only absence test in
/// the emitted body is the null-pointer check, which runs before the slice is taken; a
/// zero `_out_len` simply yields an empty owned slice. ~keep
#[test]
fn emit_function_optional_bytes_treats_zero_length_as_present_not_absent() {
    let out = emit_bytes_fn(TypeRef::Optional(Box::new(TypeRef::Bytes)));

    assert!(
        !out.contains("_out_len == 0"),
        "a zero length must never be read as absence — only a null pointer is. Got:\n{out}"
    );
    let null_check = out
        .find("if (_out_ptr == null) return null;")
        .expect("null check must be emitted");
    let slice_copy = out
        .find("std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len])")
        .expect("owned copy must be emitted");
    assert!(
        null_check < slice_copy,
        "the null check must precede the slice so an empty present value still copies. Got:\n{out}"
    );
}

/// Positive control: bare `Bytes` has no absent case, so it must NOT gain the null check.
/// Without this, the assertions above would pass for a fix that emitted the check
/// unconditionally for every byte-buffer return. ~keep
#[test]
fn emit_function_bare_bytes_return_keeps_no_absence_check() {
    let out = emit_bytes_fn(TypeRef::Bytes);

    assert!(
        out.contains("error{OutOfMemory}![]u8"),
        "bare Bytes must still declare a non-optional []u8. Got:\n{out}"
    );
    assert!(
        !out.contains("if (_out_ptr == null) return null;"),
        "bare Bytes has no absent case and must not emit the null check. Got:\n{out}"
    );
    assert!(
        out.contains("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n"),
        "bare Bytes must keep its owned copy. Got:\n{out}"
    );
}

/// Regression test: a bare opaque-handle return (`Named` not in `struct_names`, e.g. a
/// tree-sitter `Node`) makes `unwrap_return_expr` unconditionally emit
/// `if (_result == 0) return error.OutOfMemory;` -- so the declared set must carry
/// `OutOfMemory` even with no declared Rust error type and no allocating params. This must
/// fail against the pre-fix emitter, whose `body_needs_try` matched only slice-like and
/// JSON-struct returns and fell through to a bare, error-less return type here. ~keep
#[test]
fn return_conversion_needs_out_of_memory_includes_bare_opaque_handle_return() {
    assert!(return_conversion_needs_out_of_memory(
        &TypeRef::Named("NodeHandle".to_string()),
        &std::collections::HashSet::new()
    ));
}

/// Positive control: `Optional<Named>` for a bare handle degrades straight to Zig `null`
/// (see `unwrap_return_expr`'s `Optional` arm) with no `try` and no `error.OutOfMemory`, so
/// it must NOT be classified as needing `OutOfMemory`. Without this, the assertion above
/// would pass for a fix that marked every `Named` shape (optional or not) as fallible.
#[test]
fn return_conversion_needs_out_of_memory_excludes_optional_bare_handle_return() {
    assert!(!return_conversion_needs_out_of_memory(
        &TypeRef::Optional(Box::new(TypeRef::Named("NodeHandle".to_string()))),
        &std::collections::HashSet::new()
    ));
}

fn handle_return_fn() -> FunctionDef {
    FunctionDef {
        name: "root_node".to_string(),
        rust_path: "sample::root_node".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Named("NodeHandle".to_string()),
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// End-to-end regression: the free-function wrapper for a bare opaque-handle return with no
/// declared Rust error must declare `OutOfMemory`, matching the unconditional
/// `error.OutOfMemory` its body emits on a zero handle. This exercises the same shape as the
/// method-side `root_node`/`node`/`walk` bug report through `emit_function` instead of
/// `emit_opaque_method`, since `wrapper_return_type` had the identical missing arm. ~keep
#[test]
fn emit_function_bare_handle_return_declares_out_of_memory_and_body_agrees() {
    let f = handle_return_fn();
    let mut out = String::new();
    emit_function(
        &f,
        "sample",
        &[],
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &mut out,
    );

    assert!(
        out.contains("error{OutOfMemory}!NodeHandle"),
        "a bare handle return with no declared Rust error must declare OutOfMemory. Got:\n{out}"
    );
    assert!(
        out.contains("if (_result == 0) return error.OutOfMemory;"),
        "Got:\n{out}"
    );
}

#[test]
fn assert_error_set_covers_body_panics_on_declared_set_missing_a_body_error() {
    let result = std::panic::catch_unwind(|| {
        assert_error_set_covers_body(
            "root_node",
            "error{HandleClosed}!NodeHandle",
            "if (_result == 0) return error.OutOfMemory;",
            &[],
        );
    });

    assert!(
        result.is_err(),
        "a declared set missing OutOfMemory must panic when the body returns it"
    );
}

#[test]
fn assert_error_set_covers_body_accepts_a_declared_set_that_covers_the_body() {
    assert_error_set_covers_body(
        "root_node",
        "error{OutOfMemory,HandleClosed}!NodeHandle",
        "if (_result == 0) return error.OutOfMemory;",
        &[],
    );
}

/// A custom Rust-declared error type's declared-set text is just the bare type name (e.g.
/// `MyError!T`), never a literal `error{OutOfMemory,...}` spelling -- `errors::emit_error_set`
/// still guarantees `MyError` carries `OutOfMemory`/`UnknownFfiError` as members, so the
/// invariant check must accept the body's literal via `declared_errors`, not by searching
/// `return_ty`'s text for the token. ~keep
#[test]
fn assert_error_set_covers_body_accepts_out_of_memory_via_a_declared_custom_error_type() {
    assert_error_set_covers_body(
        "parse",
        "MyError!NodeHandle",
        "if (_result == 0) return error.OutOfMemory;",
        &["MyError".to_string()],
    );
}
