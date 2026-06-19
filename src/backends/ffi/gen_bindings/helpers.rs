use crate::backends::ffi::type_map::is_void_return;
use crate::codegen::doc_emission::emit_c_doxygen;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

/// Render a `/** ... */` Doxygen block above a `typedef` line. `doc` is the
/// raw rustdoc lifted from the upstream type's `///` comments; an empty `doc`
/// yields the empty string so the caller can place a bare `typedef` directly.
///
/// The block is built by reusing the shared `emit_c_doxygen` emitter (which
/// produces `///`-prefixed lines) and converting the result into `/** * */`
/// form, because `forward_decls` is C-text passthrough — there is no source
/// line for cbindgen to lift `///` comments from. Indentation is forced to
/// zero so the inserted block aligns with the typedef.
fn render_doxygen_typedef_block(doc: &str) -> String {
    if doc.trim().is_empty() {
        return String::new();
    }
    let mut raw = String::new();
    emit_c_doxygen(&mut raw, doc, "");
    let mut out = String::with_capacity(raw.len() + 16);
    out.push_str("/**\n");
    for line in raw.lines() {
        // `emit_c_doxygen` prefixes each line with `/// ` — swap that for the
        // Doxygen `*` continuation marker used inside `/** */` blocks.
        let body = line.strip_prefix("/// ").unwrap_or(line.trim_start_matches("///"));
        if body.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(" * ");
            out.push_str(body);
            out.push('\n');
        }
    }
    out.push_str(" */\n");
    out
}

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
            let type_class = if matches!(p, crate::core::ir::PrimitiveType::Bool) {
                "primitive_bool"
            } else {
                "primitive_other"
            };
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => type_class,
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::String | TypeRef::Char => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "path",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::backends::ffi::template_env::render(
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
                crate::backends::ffi::template_env::render(
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
                crate::backends::ffi::template_env::render(
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
                crate::backends::ffi::template_env::render(
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
            crate::backends::ffi::template_env::render(
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
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "bytes",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Duration => crate::backends::ffi::template_env::render(
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
            crate::backends::ffi::template_env::render(
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

/// Generate a type-appropriate unsupported body for FFI.
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
    use crate::core::ir::PrimitiveType;
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
            crate::core::ir::PrimitiveType::Bool => crate::backends::ffi::template_env::render(
                "owned_value_to_c_bool.jinja",
                minijinja::context! {
                    expr => expr,
                    indent => indent,
                },
            ),
            _ => crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "primitive_other",
                    expr => expr,
                    indent => indent,
                },
            ),
        },
        TypeRef::String | TypeRef::Char => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::backends::ffi::template_env::render(
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
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "named_owned",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Bytes => {
            // Return pointer; assume out-param for length. Cast to *mut u8 to match FFI signature.
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "bytes",
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::Optional(inner) => {
            // For non-bool primitives the inner conversion is just the value itself (passthrough),
            // so the manual-match pattern `match expr { Some(val) => val, None => default }`
            // is equivalent to `expr.unwrap_or(default)`.  Emit the latter to satisfy
            // clippy::manual_unwrap_or.  Bool needs `as i32` and stays with the match form.
            if let TypeRef::Primitive(prim) = inner.as_ref() {
                if !matches!(prim, crate::core::ir::PrimitiveType::Bool) {
                    let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
                    return format!("{indent}{expr}.unwrap_or({null_value})");
                }
            }
            let inner_conversion = gen_owned_value_to_c("val", inner, &format!("{indent}        "), _enum_names);
            let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
            // Owned-context: consume the Option so val is owned T (not &T).
            crate::backends::ffi::template_env::render(
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
        TypeRef::Duration => crate::backends::ffi::template_env::render(
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

pub(super) fn gen_cbindgen_toml(
    prefix: &str,
    api: &crate::core::ir::ApiSurface,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) -> String {
    let prefix_upper = prefix.to_uppercase();

    // Collect (c_name, doc) pairs for every opaque handle that needs a forward
    // declaration in the generated C header. cbindgen renames Rust types using
    // the export prefix, producing e.g. `HTMMetadataConfig` for prefix `HTM`.
    //
    // Capsule (Language-passthrough) types are skipped here: their passthrough function
    // returns the host-native pointee (forward-declared unprefixed below). The exception
    // is a capsule type still returned as an opaque handle by a method (e.g.
    // `LanguageRegistry.get_language`) — that prefixed handle (`{PREFIX}Language`) must be
    // forward-declared too, or the method's C declaration references an undefined type.
    let capsule_used_as_opaque: std::collections::HashSet<&str> = api
        .types
        .iter()
        .flat_map(|t| t.methods.iter())
        .filter_map(|m| match &m.return_type {
            crate::core::ir::TypeRef::Named(name) if capsule_types.contains_key(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let mut entries: Vec<(String, String)> = api
        .types
        .iter()
        .filter(|t| !capsule_types.contains_key(t.name.as_str()) || capsule_used_as_opaque.contains(t.name.as_str()))
        // Use the IR type name verbatim (it already comes from Rust source as
        // PascalCase). `to_pascal_case` mangles names containing all-caps
        // abbreviations: e.g. `GraphQLError` becomes `GraphQlError`, which
        // disagrees with cbindgen's emit (e.g. `MYLIBGraphQLError` for prefix
        // `MYLIB`) and breaks the C consumer build.
        .map(|t| (format!("{prefix_upper}{}", t.name), t.doc.clone()))
        .collect();

    // Forward-declare each capsule pointee type UNPREFIXED (e.g. `TSLanguage`), so the
    // `const TSLanguage *` return type cbindgen emits from `*const tree_sitter::ffi::TSLanguage`
    // resolves in the generated header. Sorted+deduped via the shared `entries` vec below.
    {
        let mut c_names: Vec<&str> = capsule_types.values().map(|c| c.c_return_type.as_str()).collect();
        c_names.sort_unstable();
        c_names.dedup();
        for c_name in c_names {
            if !entries.iter().any(|(n, _)| n == c_name) {
                entries.push((c_name.to_string(), String::new()));
            }
        }
    }

    // Include enum types as well — they may appear as opaque handles in
    // function signatures when used across module boundaries.
    for e in &api.enums {
        let c_name = format!("{prefix_upper}{}", e.name);
        if !entries.iter().any(|(n, _)| n == &c_name) {
            entries.push((c_name, e.doc.clone()));
        }
    }

    // Include error types — every error whose accessor functions are emitted
    // (via gen_ffi_error_methods) references *const ErrorType in the FFI
    // signature. Without a forward typedef cbindgen produces an "unknown type
    // name" error in the generated C header.
    for err in &api.errors {
        if !err.methods.is_empty() {
            let c_name = format!("{prefix_upper}{}", err.name);
            if !entries.iter().any(|(n, _)| n == &c_name) {
                entries.push((c_name, err.doc.clone()));
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let forward_decls: String = entries
        .iter()
        .map(|(name, doc)| {
            // Render a Doxygen `/** ... */` block above each typedef when the
            // upstream rustdoc is non-empty. The block is emitted inline (the
            // declaration is part of cbindgen's `after_includes` literal text)
            // rather than via `///` comments because there is no source line
            // for cbindgen to lift comments from — `forward_decls` is a raw
            // C-text passthrough.
            let doc_block = render_doxygen_typedef_block(doc);
            if doc_block.is_empty() {
                format!("typedef struct {name} {name};")
            } else {
                format!("{doc_block}typedef struct {name} {name};")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let after_includes = if forward_decls.is_empty() {
        String::new()
    } else {
        toml_multiline_basic_string(&format!("/* Opaque type forward declarations */\n{forward_decls}\n"))
    };

    crate::backends::ffi::template_env::render(
        "cbindgen_toml.jinja",
        minijinja::context! {
            prefix_upper => &prefix_upper,
            after_includes => &after_includes,
        },
    )
}

fn toml_multiline_basic_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"")
        .replace('\u{8}', "\\b")
        .replace('\u{c}', "\\f");
    format!("\"\"\"\n{escaped}\"\"\"")
}

// ---------------------------------------------------------------------------
// build.rs generation
// ---------------------------------------------------------------------------

pub(super) fn gen_build_rs(
    header_name: &str,
    lib_name: &str,
    go_output_dir: Option<&str>,
    prefix: &str,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) -> String {
    // cbindgen applies the export prefix to every type it references, including the
    // host-native capsule pointee types (e.g. `tree_sitter::ffi::TSLanguage`), so
    // `const TSLanguage *` is emitted as `const {PREFIX}TSLanguage *` — a name cbindgen
    // never declares. Those pointees are forward-declared UNPREFIXED in the header
    // prelude, so rewrite the prefixed references back to the bare prelude name.
    // Longest-first so a prefixed name that is a substring of another is replaced safely.
    let capsule_header_fixup = {
        let prefix_upper = prefix.to_uppercase();
        let mut pairs: Vec<(String, String)> = capsule_types
            .values()
            .map(|c| (format!("{prefix_upper}{}", c.c_return_type), c.c_return_type.clone()))
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        pairs.sort_by_key(|(prefixed, _)| std::cmp::Reverse(prefixed.len()));
        if pairs.is_empty() {
            String::new()
        } else {
            let arr = pairs
                .iter()
                .map(|(prefixed, bare)| format!("(\"{prefixed}\", \"{bare}\")"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "\n    // Rewrite prefixed host-native capsule pointee types back to the unprefixed\n    \
                 // names forward-declared in the header prelude (cbindgen prefixes all referenced\n    \
                 // types, but these are external types defined by the host tree-sitter runtime).\n    \
                 {{\n        \
                 let header_path = \"include/{header_name}\";\n        \
                 let mut header = std::fs::read_to_string(header_path).expect(\"read generated header\");\n        \
                 for (prefixed, bare) in [{arr}] {{\n            \
                 header = header.replace(prefixed, bare);\n        \
                 }}\n        \
                 std::fs::write(header_path, header).expect(\"write patched header\");\n    \
                 }}\n"
            )
        }
    };
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
    crate::backends::ffi::template_env::render(
        "build_rs.jinja",
        minijinja::context! {
            header_name => header_name,
            lib_name => lib_name,
            go_copy_step => go_copy_step,
            capsule_header_fixup => capsule_header_fixup,
        },
    )
}

// ---------------------------------------------------------------------------
// last_error pattern
// ---------------------------------------------------------------------------

pub(super) fn gen_last_error(prefix: &str) -> String {
    crate::backends::ffi::template_env::render(
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
    crate::backends::ffi::template_env::render(
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
    crate::backends::ffi::template_env::render(
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
    crate::backends::ffi::template_env::render("ffi_tokio_runtime.jinja", minijinja::context! {})
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
    // Error type is erased to a boxed trait object so the handle type is stable across
    // error-type changes in core.  Uses only std — no anyhow dependency required.
    let boxed_err = "Box<dyn std::error::Error + Send + Sync + 'static>";
    let stream_ty = format!("futures_util::stream::BoxStream<'static, Result<{core_item}, {boxed_err}>>");
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

    // Map the stream's concrete error type to Box<dyn Error> to erase it from the handle type.
    let mapped: {stream_ty} = {{
        use futures_util::StreamExt;
        Box::pin(raw_stream.map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)))
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

    use futures_util::StreamExt;
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
