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
