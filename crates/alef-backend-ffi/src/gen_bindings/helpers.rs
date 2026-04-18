use crate::type_map::is_void_return;
use alef_core::ir::TypeRef;
use std::fmt::Write;

/// Generate code to convert a Rust value reference to a C return value.
/// `expr` is the Rust expression to read from (must be borrowable).
pub(super) fn gen_value_to_c(expr: &str, ty: &TypeRef, indent: &str) -> String {
    let mut out = String::with_capacity(2048);
    match ty {
        TypeRef::Primitive(p) => {
            // Bool needs cast to i32 for C ABI; other primitives may need deref if from Option
            if matches!(p, alef_core::ir::PrimitiveType::Bool) {
                writeln!(out, "{indent}{expr} as i32").ok();
            } else {
                writeln!(out, "{indent}{expr}").ok();
            }
        }
        TypeRef::String | TypeRef::Char => {
            writeln!(out, "{indent}match CString::new({expr}.to_string()) {{").ok();
            writeln!(out, "{indent}    Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Path => {
            writeln!(
                out,
                "{indent}match CString::new({expr}.to_string_lossy().to_string()) {{"
            )
            .ok();
            writeln!(out, "{indent}    Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Json => {
            writeln!(out, "{indent}match serde_json::to_string(&{expr}) {{").ok();
            writeln!(out, "{indent}    Ok(s) => match CString::new(s) {{").ok();
            writeln!(out, "{indent}        Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}        Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}    }},").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Named(_) => {
            writeln!(out, "{indent}Box::into_raw(Box::new({expr}.clone()))").ok();
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Serialize as JSON
            writeln!(out, "{indent}match serde_json::to_string(&{expr}) {{").ok();
            writeln!(out, "{indent}    Ok(s) => match CString::new(s) {{").ok();
            writeln!(out, "{indent}        Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}        Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}    }},").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Bytes => {
            // Return pointer; caller must also get length. Cast to *mut u8 to match FFI signature.
            writeln!(out, "{indent}{expr}.as_ptr() as *mut u8").ok();
        }
        TypeRef::Duration => {
            writeln!(out, "{indent}{expr}.as_millis() as u64").ok();
        }
        TypeRef::Unit => {
            // nothing to return
        }
        TypeRef::Optional(inner) => {
            writeln!(out, "{indent}match &{expr} {{").ok();
            writeln!(out, "{indent}    Some(val) => {{").ok();
            write!(out, "{}", gen_value_to_c("val", inner, &format!("{indent}        "))).ok();
            writeln!(out, "{indent}    }}").ok();
            writeln!(
                out,
                "{indent}    None => {},",
                null_return_value(&TypeRef::Optional(inner.clone()))
            )
            .ok();
            writeln!(out, "{indent}}}").ok();
        }
    }
    out
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

pub(super) fn gen_owned_value_to_c(expr: &str, ty: &TypeRef, indent: &str) -> String {
    let mut out = String::with_capacity(2048);
    match ty {
        TypeRef::Primitive(prim) => match prim {
            alef_core::ir::PrimitiveType::Bool => {
                writeln!(out, "{indent}if {expr} {{").ok();
                writeln!(out, "{indent}    1").ok();
                writeln!(out, "{indent}}} else {{").ok();
                writeln!(out, "{indent}    0").ok();
                writeln!(out, "{indent}}}").ok();
            }
            _ => {
                writeln!(out, "{indent}{expr}").ok();
            }
        },
        TypeRef::String | TypeRef::Char => {
            writeln!(out, "{indent}match CString::new({expr}) {{").ok();
            writeln!(out, "{indent}    Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Json => {
            writeln!(out, "{indent}match serde_json::to_string(&{expr}) {{").ok();
            writeln!(out, "{indent}    Ok(s) => match CString::new(s) {{").ok();
            writeln!(out, "{indent}        Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}        Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}    }},").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Path => {
            writeln!(
                out,
                "{indent}match CString::new({expr}.to_string_lossy().to_string()) {{"
            )
            .ok();
            writeln!(out, "{indent}    Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Named(_) => {
            writeln!(out, "{indent}Box::into_raw(Box::new({expr}))").ok();
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            writeln!(out, "{indent}match serde_json::to_string(&{expr}) {{").ok();
            writeln!(out, "{indent}    Ok(s) => match CString::new(s) {{").ok();
            writeln!(out, "{indent}        Ok(cs) => cs.into_raw(),").ok();
            writeln!(out, "{indent}        Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}    }},").ok();
            writeln!(out, "{indent}    Err(_) => std::ptr::null_mut(),").ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Bytes => {
            // Return pointer; assume out-param for length. Cast to *mut u8 to match FFI signature.
            writeln!(out, "{indent}{expr}.as_ptr() as *mut u8").ok();
        }
        TypeRef::Optional(inner) => {
            writeln!(out, "{indent}match {expr} {{").ok();
            writeln!(out, "{indent}    Some(val) => {{").ok();
            write!(
                out,
                "{}",
                gen_owned_value_to_c("val", inner, &format!("{indent}        "))
            )
            .ok();
            writeln!(out, "{indent}    }}").ok();
            writeln!(
                out,
                "{indent}    None => {},",
                null_return_value(&TypeRef::Optional(inner.clone()))
            )
            .ok();
            writeln!(out, "{indent}}}").ok();
        }
        TypeRef::Duration => {
            writeln!(out, "{indent}{expr}.as_millis() as u64").ok();
        }
        TypeRef::Unit => {}
    }
    out
}

// ---------------------------------------------------------------------------
// cbindgen.toml generation
// ---------------------------------------------------------------------------

pub(super) fn gen_cbindgen_toml(prefix: &str, api: &alef_core::ir::ApiSurface) -> String {
    use heck::ToPascalCase;

    let prefix_upper = prefix.to_uppercase();

    // Collect all type names that appear in the API surface and need forward
    // declarations in the generated C header. cbindgen renames Rust types using
    // the export prefix, producing e.g. `HTMMetadataConfig` for prefix `HTM`.
    let mut type_names: Vec<String> = api
        .types
        .iter()
        .map(|t| format!("{prefix_upper}{}", t.name.to_pascal_case()))
        .collect();

    // Include enum types as well — they may appear as opaque handles in
    // function signatures when used across module boundaries.
    for e in &api.enums {
        let c_name = format!("{prefix_upper}{}", e.name.to_pascal_case());
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

    let after_includes = if forward_decls.is_empty() {
        String::new()
    } else {
        format!("\nafter_includes = \"\"\"\n/* Opaque type forward declarations */\n{forward_decls}\n\"\"\"\n")
    };

    format!(
        r#"# This file is auto-generated by alef. DO NOT EDIT.
language = "C"
include_guard = "{prefix_upper}_H"
pragma_once = true
autogen_warning = "/* This file is auto-generated by alef. DO NOT EDIT. */"
{after_includes}
[defines]
"target_os = windows" = "SKIF_WINDOWS"

[export]
prefix = "{prefix_upper}"
include = []
exclude = []

[fn]
args = "vertical"
"#
    )
}

// ---------------------------------------------------------------------------
// build.rs generation
// ---------------------------------------------------------------------------

pub(super) fn gen_build_rs(header_name: &str) -> String {
    format!(
        r#"// This file is auto-generated by alef. DO NOT EDIT.
fn main() {{
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    cbindgen::generate(crate_dir)
        .expect("Unable to generate C bindings")
        .write_to_file("include/{header_name}");
}}
"#
    )
}

// ---------------------------------------------------------------------------
// last_error pattern
// ---------------------------------------------------------------------------

pub(super) fn gen_last_error(prefix: &str) -> String {
    format!(
        r#"thread_local! {{
    static LAST_ERROR_CODE: RefCell<i32> = RefCell::new(0);
    static LAST_ERROR_CONTEXT: RefCell<Option<CString>> = RefCell::new(None);
}}

fn set_last_error(code: i32, message: &str) {{
    LAST_ERROR_CODE.with_borrow_mut(|c| *c = code);
    LAST_ERROR_CONTEXT.with_borrow_mut(|c| *c = CString::new(message).ok());
}}

fn clear_last_error() {{
    LAST_ERROR_CODE.with_borrow_mut(|c| *c = 0);
    LAST_ERROR_CONTEXT.with_borrow_mut(|c| *c = None);
}}

/// Return the last error code (0 means no error).
/// # Safety
/// Caller must ensure all pointer arguments are valid or null.
/// Returned pointers must be freed with the appropriate free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_last_error_code() -> i32 {{
    LAST_ERROR_CODE.with_borrow(|c| *c)
}}

/// Return the last error message. The pointer is valid until the next FFI call on this thread.
/// # Safety
/// Caller must ensure all pointer arguments are valid or null.
/// Returned pointers must be freed with the appropriate free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_last_error_context() -> *const c_char {{
    LAST_ERROR_CONTEXT.with_borrow(|ctx| {{
        ctx.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())
    }})
}}"#,
        prefix = prefix
    )
}

// ---------------------------------------------------------------------------
// free_string
// ---------------------------------------------------------------------------

pub(super) fn gen_free_string(prefix: &str) -> String {
    format!(
        r#"/// Free a string previously returned by this library.
/// # Safety
/// Pointer must have been returned by this library, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_free_string(ptr: *mut c_char) {{
    if !ptr.is_null() {{
        unsafe {{ drop(CString::from_raw(ptr)); }}
    }}
}}"#,
        prefix = prefix
    )
}

// ---------------------------------------------------------------------------
// version
// ---------------------------------------------------------------------------

pub(super) fn gen_version(prefix: &str) -> String {
    format!(
        r#"/// Return the library version string. The pointer is static and must NOT be freed.
/// # Safety
/// Caller must ensure all pointer arguments are valid or null.
/// Returned pointers must be freed with the appropriate free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_version() -> *const c_char {{
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}}"#,
        prefix = prefix
    )
}

/// Generate a lazily-initialized tokio runtime helper for blocking on async
/// functions from synchronous FFI entry points.
pub(super) fn gen_ffi_tokio_runtime() -> String {
    r#"fn get_ffi_runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
    })
}"#
    .to_string()
}
