use crate::type_map::is_void_return;
use ahash::AHashSet;
use alef_core::ir::TypeRef;

/// Render an expression that produces a Copy-typed value, avoiding clippy::clone_on_copy.
///
/// `expr` is either a place expression (e.g., `obj.field`, `(*obj.field)`) or a binding
/// to a reference (e.g., `val`). For places, auto-copy applies. For refs, we deref.
fn copy_expr(expr: &str) -> String {
    if expr.starts_with("obj.") || expr.starts_with("(*") {
        expr.to_string()
    } else {
        format!("*{expr}")
    }
}

/// Generate code to convert a Rust value reference to a C return value.
///
/// `expr` is the Rust expression to read from (a borrowed place or ref binding).
/// `enum_names` is the set of IR enum type names — Copy in our codegen, so we use
/// the copy path instead of `.clone()` (avoids `clippy::clone_on_copy`).
/// `clone_names` is the set of IR named-type names that implement `Clone`.
/// For `Named` types **not** in `clone_names` (non-Clone opaques), a raw pointer
/// cast is emitted so the accessor compiles without requiring `Clone`.
pub(super) fn gen_value_to_c(
    expr: &str,
    ty: &TypeRef,
    indent: &str,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            // Bool needs cast to i32 for C ABI; other primitives may need deref if from Option
            let type_class = if matches!(p, alef_core::ir::PrimitiveType::Bool) {
                "primitive_bool"
            } else {
                "primitive_other"
            };
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => type_class,
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::String | TypeRef::Char => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "path",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Named(name) => {
            if enum_names.contains(name.as_str()) {
                // Copy-typed enums: clippy::clone_on_copy fires on .clone(). Use auto-copy/deref.
                let copy = copy_expr(expr);
                crate::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_enum",
                        expr => expr,
                        copy_expr => &copy,
                        indent => indent,
                    },
                )
            } else if clone_names.contains(name.as_str()) {
                // Clone-capable struct: clone the borrowed reference into an owned box.
                crate::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_clone",
                        expr => expr,
                        indent => indent,
                    },
                )
            } else {
                // Non-Clone opaque type: the caller holds a borrow from the parent struct.
                // Return a raw pointer alias — the C caller must not outlive the parent handle.
                crate::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_non_clone",
                        expr => expr,
                        indent => indent,
                    },
                )
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Serialize as JSON
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "json_or_vec_or_map",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Bytes => {
            // Return pointer; caller must also get length. Cast to *mut u8 to match FFI signature.
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "bytes",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Duration => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "duration",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Unit => String::new(),
        TypeRef::Optional(inner) => {
            let inner_conversion = gen_value_to_c("val", inner, &format!("{indent}        "), enum_names, clone_names);
            let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "optional",
                    expr => expr,
                    indent => indent,
                    inner_conversion => &inner_conversion,
                    null_value => null_value,
                },
            )
        }
    }
}

/// Generate a type-appropriate unimplemented body for FFI (no todo!()).
/// Uses set_last_error + null/zero return instead of panicking.
pub(super) fn gen_ffi_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error && is_void_return(return_type) {
        // Fallible + void: return error code
        format!("    set_last_error(99, \"{err_msg}\");\n    -1")
    } else if is_void_return(return_type) {
        // Infallible void: just set error context and return
        format!("    set_last_error(99, \"{err_msg}\");")
    } else {
        // Non-void: set error and return null/zero
        let ret = null_return_value(return_type);
        format!("    set_last_error(99, \"{err_msg}\");\n    {ret}")
    }
}

/// Return the null/zero value for a given type in return position.
pub(super) fn null_return_value(ty: &TypeRef) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            _ => "0",
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "std::ptr::null_mut()",
        TypeRef::Bytes => "std::ptr::null_mut()",
        TypeRef::Named(_) => "std::ptr::null_mut()",
        TypeRef::Vec(_) | TypeRef::Map(_, _) => "std::ptr::null_mut()",
        TypeRef::Duration => "0",
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
                _ => "0",
            },
            // Option<Option<Primitive>> — both None cases collapse to 0/false.
            TypeRef::Optional(inner2) => match inner2.as_ref() {
                TypeRef::Primitive(p) => match p {
                    PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
                    _ => "0",
                },
                _ => "std::ptr::null_mut()",
            },
            TypeRef::Duration => "0",
            _ => "std::ptr::null_mut()",
        },
        TypeRef::Unit => "()",
    }
}

// ---------------------------------------------------------------------------
// Convert owned Rust value to C return (non-Result path)
// ---------------------------------------------------------------------------

pub(super) fn gen_owned_value_to_c(expr: &str, ty: &TypeRef, indent: &str, _enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            alef_core::ir::PrimitiveType::Bool => crate::template_env::render(
                "owned_value_to_c_bool.jinja",
                minijinja::context! {
                    expr => expr,
                    indent => indent,
                },
            ),
            _ => crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "primitive_other",
                    expr => expr,
                    indent => indent,
                },
            ),
        },
        TypeRef::String | TypeRef::Char => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "path",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Named(_) => {
            // For owned values, .clone() is wasteful AND incorrect when the type is non-Clone
            // (opaque handles like Parser/Registry). Move the value into the Box.
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "named_owned",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Bytes => {
            // Return pointer; assume out-param for length. Cast to *mut u8 to match FFI signature.
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "bytes",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Optional(inner) => {
            let inner_conversion = gen_owned_value_to_c("val", inner, &format!("{indent}        "), _enum_names);
            let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
            // Owned-context: consume the Option so val is owned T (not &T).
            crate::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "optional_owned",
                    expr => expr,
                    indent => indent,
                    inner_conversion => &inner_conversion,
                    null_value => null_value,
                },
            )
        }
        TypeRef::Duration => crate::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "duration",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Unit => String::new(),
    }
}

// ---------------------------------------------------------------------------
// cbindgen.toml generation
// ---------------------------------------------------------------------------

pub(super) fn gen_cbindgen_toml(prefix: &str, api: &alef_core::ir::ApiSurface) -> String {
    let prefix_upper = prefix.to_uppercase();

    // Collect all type names that appear in the API surface and need forward
    // declarations in the generated C header. cbindgen renames Rust types using
    // the export prefix, producing e.g. `HTMMetadataConfig` for prefix `HTM`.
    let mut type_names: Vec<String> = api
        .types
        .iter()
        // Use the IR type name verbatim (it already comes from Rust source as
        // PascalCase). `to_pascal_case` mangles names containing all-caps
        // abbreviations: e.g. `GraphQLError` becomes `GraphQlError`, which
        // disagrees with cbindgen's emit (e.g. `MYLIBGraphQLError` for prefix
        // `MYLIB`) and breaks the C consumer build.
        .map(|t| format!("{prefix_upper}{}", t.name))
        .collect();

    // Include enum types as well — they may appear as opaque handles in
    // function signatures when used across module boundaries.
    for e in &api.enums {
        let c_name = format!("{prefix_upper}{}", e.name);
        if !type_names.contains(&c_name) {
            type_names.push(c_name);
        }
    }

    type_names.sort();

    let forward_decls: String = type_names
        .iter()
        .map(|name| format!("typedef struct {name} {name};"))
        .collect::<Vec<_>>()
        .join("\n");

    crate::template_env::render(
        "cbindgen_toml.jinja",
        minijinja::context! {
            prefix_upper => &prefix_upper,
            forward_decls => &forward_decls,
        },
    )
}

// ---------------------------------------------------------------------------
// build.rs generation
// ---------------------------------------------------------------------------

pub(super) fn gen_build_rs(header_name: &str, go_output_dir: Option<&str>) -> String {
    let go_copy_step = match go_output_dir {
        Some(go_dir) => {
            let go_dir = go_dir.trim_end_matches('/');
            let depth = std::path::Path::new(go_dir)
                .components()
                .filter(|c| matches!(c, std::path::Component::Normal(_)))
                .count()
                .max(1);
            let to_root = "../".repeat(depth);
            let dest_dir = format!("{to_root}{go_dir}/include");
            format!(
                "\n    let go_include_dir = std::path::Path::new(\"{dest_dir}\");\n    \
                 std::fs::create_dir_all(go_include_dir).expect(\"Unable to create Go include directory\");\n    \
                 std::fs::copy(\"include/{header_name}\", go_include_dir.join(\"{header_name}\"))\n        \
                 .expect(\"Unable to copy header to Go include directory\");\n"
            )
        }
        None => String::new(),
    };
    crate::template_env::render(
        "build_rs.jinja",
        minijinja::context! {
            header_name => header_name,
            go_copy_step => go_copy_step,
        },
    )
}

// ---------------------------------------------------------------------------
// last_error pattern
// ---------------------------------------------------------------------------

pub(super) fn gen_last_error(prefix: &str) -> String {
    crate::template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    )
}

// ---------------------------------------------------------------------------
// free_string
// ---------------------------------------------------------------------------

pub(super) fn gen_free_string(prefix: &str) -> String {
    crate::template_env::render(
        "free_string.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    )
}

// ---------------------------------------------------------------------------
// version
// ---------------------------------------------------------------------------

pub(super) fn gen_version(prefix: &str) -> String {
    crate::template_env::render(
        "version_fn.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    )
}

// ---------------------------------------------------------------------------
// free_bytes
// ---------------------------------------------------------------------------

/// Generate a `{prefix}_free_bytes` companion that reconstructs and drops a
/// heap-allocated `Vec<u8>` previously returned via the out-param convention
/// (`out_ptr / out_len / out_cap`).
///
/// This is emitted once per FFI module alongside `{prefix}_free_string` so
/// that callers can safely release byte buffers returned by functions whose
/// Rust signature is `Result<Vec<u8>>`.
pub(super) fn gen_free_bytes(prefix: &str) -> String {
    format!(
        r#"/// Free a byte buffer previously returned by this library via out-params.
/// `ptr`, `len`, and `cap` must match the values written by the library function,
/// or the call must pass `ptr = null` (in which case it is a no-op).
/// # Safety
/// Pointer must have been returned by this library (via out_ptr / out_len / out_cap
/// out-params), or be null. The len and cap values must be unchanged since the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_free_bytes(ptr: *mut u8, len: usize, cap: usize) {{
    if !ptr.is_null() {{
        // SAFETY: ptr/len/cap were produced by Vec::into_raw_parts (or equivalent)
        // by this library; caller must not have mutated them.
        unsafe {{ drop(Vec::from_raw_parts(ptr, len, cap)); }}
    }}
}}"#,
        prefix = prefix
    )
}

/// Generate a lazily-initialized tokio runtime helper for blocking on async
/// functions from synchronous FFI entry points.
pub(super) fn gen_ffi_tokio_runtime() -> String {
    crate::template_env::render("ffi_tokio_runtime.jinja", minijinja::context! {})
}

// ---------------------------------------------------------------------------
// Stream handle (iterator-based streaming for FFI consumers)
// ---------------------------------------------------------------------------

/// Generate the three iterator-handle functions for a streaming adapter:
///
/// - `{prefix}_{type_snake}_{name}_start` — create handle from client + request
/// - `{prefix}_{type_snake}_{name}_next`  — advance stream, return boxed chunk or null
/// - `{prefix}_{type_snake}_{name}_free`  — drop handle
///
/// Also emits the opaque handle struct that owns the tokio runtime + BoxStream.
///
/// The handle name is derived as `{PascalPrefix}{PascalOwnerType}{PascalName}StreamHandle`.
/// The function prefix is `{prefix}_{owner_type_snake}_{adapter_name}`.
///
/// Error protocol: `_next` returns null on both clean end-of-stream AND error.
/// After null, caller checks `{prefix}_last_error_code()` — 0 is clean end, non-zero is error.
pub(super) fn gen_stream_handle_functions(
    prefix: &str,
    owner_type: &str,
    adapter_name: &str,
    core_path: &str,
    item_type: &str,
    request_type: &str,
    core_import: &str,
) -> String {
    use heck::{ToPascalCase, ToSnakeCase};

    let pascal_prefix = prefix.to_pascal_case();
    let pascal_owner = owner_type.to_pascal_case();
    let pascal_name = adapter_name.to_pascal_case();
    let owner_snake = owner_type.to_snake_case();

    let handle_name = format!("{pascal_prefix}{pascal_owner}{pascal_name}StreamHandle");
    let fn_start = format!("{prefix}_{owner_snake}_{adapter_name}_start");
    let fn_next = format!("{prefix}_{owner_snake}_{adapter_name}_next");
    let fn_free = format!("{prefix}_{owner_snake}_{adapter_name}_free");

    // Full item type path for the BoxStream generic
    let core_item = format!("{core_import}::{item_type}");
    // Error type is the Rust stream item error — we use anyhow::Error as the broadest
    // common type since the core returns BoxStream<Result<Item, impl Error>>.
    // We erase the error type to anyhow::Error to keep the handle type stable.
    let stream_ty = format!(
        "futures::stream::BoxStream<'static, Result<{core_item}, anyhow::Error>>"
    );
    let owner_ty = format!("{core_import}::{owner_type}");

    format!(
        r#"/// Opaque handle owning a tokio runtime and a boxed chat-stream for iterator-style consumption.
///
/// Created by `{fn_start}`, advanced by `{fn_next}`, destroyed by `{fn_free}`.
/// The handle is NOT thread-safe — callers must ensure only one thread calls `_next` at a time.
pub struct {handle_name} {{
    rt: tokio::runtime::Runtime,
    stream: std::sync::Mutex<Option<{stream_ty}>>,
}}

/// Start a streaming chat completion and return an opaque iterator handle.
///
/// Returns null and sets `{prefix}_last_error_code` on failure (null pointers or stream-open error).
/// On success the caller owns the returned pointer and MUST call `{fn_free}` when done.
///
/// # Safety
/// `client` must be a non-null valid pointer to a live `{owner_ty}` produced by this library.
/// `req` must be a non-null valid pointer to a live `{request_type}` produced by this library.
/// Both pointers must remain valid until this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_start}(
    client: *const {owner_ty},
    req: *const {request_type},
) -> *mut {handle_name} {{
    clear_last_error();

    if client.is_null() {{
        set_last_error(99, "{fn_start}: client must not be NULL");
        return std::ptr::null_mut();
    }}
    if req.is_null() {{
        set_last_error(99, "{fn_start}: req must not be NULL");
        return std::ptr::null_mut();
    }}

    // SAFETY: caller guarantees `client` is a non-null, valid, aligned pointer to a live
    // `{owner_ty}` value. The reference does not outlive this function.
    let client_ref = unsafe {{ &*client }};

    // SAFETY: caller guarantees `req` is a non-null, valid, aligned pointer to a live
    // `{request_type}` value. We clone it to obtain an owned request independent of the
    // caller's lifetime.
    let req_owned = unsafe {{ (*req).clone() }};

    let rt = match tokio::runtime::Runtime::new() {{
        Ok(r) => r,
        Err(e) => {{
            set_last_error(99, &format!("{fn_start}: failed to create tokio runtime: {{e}}"));
            return std::ptr::null_mut();
        }}
    }};

    let stream_result = rt.block_on(async {{ client_ref.{core_path}(req_owned).await }});

    let raw_stream = match stream_result {{
        Ok(s) => s,
        Err(e) => {{
            set_last_error(99, &format!("{fn_start}: failed to open stream: {{e}}"));
            return std::ptr::null_mut();
        }}
    }};

    // Map the stream's concrete error type to anyhow::Error to erase it from the handle type.
    let mapped: {stream_ty} = {{
        use futures::StreamExt;
        Box::pin(raw_stream.map(|r| r.map_err(anyhow::Error::from)))
    }};

    let handle = Box::new({handle_name} {{
        rt,
        stream: std::sync::Mutex::new(Some(mapped)),
    }});

    Box::into_raw(handle)
}}

/// Advance the stream and return a heap-allocated chunk, or null.
///
/// Returns null in two cases:
/// - Clean end-of-stream: `{prefix}_last_error_code()` returns 0.
/// - Stream error: `{prefix}_last_error_code()` returns non-zero.
///
/// The returned pointer is heap-allocated and the caller MUST free it by calling
/// `{prefix}_{owner_snake}_{item_type}_free` (or the appropriate type-free function).
///
/// # Safety
/// `handle` must be a non-null valid pointer previously returned by `{fn_start}` and not yet
/// freed. Calling `_next` after `_free` is undefined behaviour. The handle must not be shared
/// across threads without external synchronisation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_next}(
    handle: *mut {handle_name},
) -> *mut {core_item} {{
    clear_last_error();

    if handle.is_null() {{
        set_last_error(99, "{fn_next}: handle must not be NULL");
        return std::ptr::null_mut();
    }}

    // SAFETY: caller guarantees `handle` is a non-null valid pointer produced by `{fn_start}`
    // and not yet freed. We take a shared reference for the duration of this call.
    let h = unsafe {{ &*handle }};

    let mut guard = match h.stream.lock() {{
        Ok(g) => g,
        Err(_) => {{
            set_last_error(99, "{fn_next}: stream mutex is poisoned");
            return std::ptr::null_mut();
        }}
    }};

    let stream = match guard.as_mut() {{
        Some(s) => s,
        None => {{
            // Stream already exhausted or taken.
            return std::ptr::null_mut();
        }}
    }};

    use futures::StreamExt;
    match h.rt.block_on(stream.next()) {{
        Some(Ok(chunk)) => {{
            // SAFETY: We box the chunk and transfer ownership to the caller via raw pointer.
            // The caller must free it via the appropriate type-free function.
            Box::into_raw(Box::new(chunk))
        }}
        Some(Err(e)) => {{
            set_last_error(99, &format!("{fn_next}: stream error: {{e}}"));
            std::ptr::null_mut()
        }}
        None => {{
            // Clean end-of-stream — error code remains 0 (cleared at top of function).
            *guard = None;
            std::ptr::null_mut()
        }}
    }}
}}

/// Free a stream handle created by `{fn_start}`.
///
/// Safe to call with a null pointer (no-op). After this call the handle pointer is invalid.
///
/// # Safety
/// `handle` must either be null or a valid pointer previously returned by `{fn_start}` and
/// not yet freed. Double-free is undefined behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_free}(handle: *mut {handle_name}) {{
    if !handle.is_null() {{
        // SAFETY: `handle` was produced by Box::into_raw in `{fn_start}` and has not been freed.
        // Reconstructing the Box transfers ownership back to Rust, which drops it at end of scope.
        unsafe {{ drop(Box::from_raw(handle)); }}
    }}
}}"#,
        handle_name = handle_name,
        fn_start = fn_start,
        fn_next = fn_next,
        fn_free = fn_free,
        prefix = prefix,
        owner_ty = owner_ty,
        request_type = request_type,
        core_path = core_path,
        stream_ty = stream_ty,
        core_item = core_item,
        owner_snake = owner_snake,
        item_type = item_type,
    )
}
