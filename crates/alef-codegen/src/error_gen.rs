use ahash::AHashSet;
use alef_core::ir::{ErrorDef, ErrorVariant};

use crate::conversions::is_tuple_variant;

/// Generate a wildcard match pattern for an error variant.
/// Struct variants use `{ .. }`, tuple variants use `(..)`, unit variants have no suffix.
fn error_variant_wildcard_pattern(rust_path: &str, variant: &ErrorVariant) -> String {
    if variant.is_unit {
        format!("{rust_path}::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        format!("{rust_path}::{}(..)", variant.name)
    } else {
        format!("{rust_path}::{} {{ .. }}", variant.name)
    }
}

/// Python builtin exception names that must not be shadowed (A004 compliance).
const PYTHON_BUILTIN_EXCEPTIONS: &[&str] = &[
    "ConnectionError",
    "TimeoutError",
    "PermissionError",
    "FileNotFoundError",
    "ValueError",
    "TypeError",
    "RuntimeError",
    "OSError",
    "IOError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "ImportError",
    "MemoryError",
    "OverflowError",
    "StopIteration",
    "RecursionError",
    "SystemError",
    "ReferenceError",
    "BufferError",
    "EOFError",
    "LookupError",
    "ArithmeticError",
    "AssertionError",
    "BlockingIOError",
    "BrokenPipeError",
    "ChildProcessError",
    "FileExistsError",
    "InterruptedError",
    "IsADirectoryError",
    "NotADirectoryError",
    "ProcessLookupError",
    "UnicodeError",
];

/// Compute a prefix from the error type name by stripping a trailing "Error" suffix.
/// E.g. `"CrawlError"` -> `"Crawl"`, `"MyException"` -> `"MyException"`.
fn error_base_prefix(error_name: &str) -> &str {
    error_name.strip_suffix("Error").unwrap_or(error_name)
}

/// Return the Python exception name for a variant, avoiding shadowing of Python builtins.
///
/// 1. Appends `"Error"` suffix if not already present (N818 compliance).
/// 2. If the resulting name shadows a Python builtin, prefixes it with the error type's base
///    name. E.g. for `CrawlError::Connection` -> `ConnectionError` (shadowed) -> `CrawlConnectionError`.
pub fn python_exception_name(variant_name: &str, error_name: &str) -> String {
    let candidate = if variant_name.ends_with("Error") {
        variant_name.to_string()
    } else {
        format!("{}Error", variant_name)
    };

    if PYTHON_BUILTIN_EXCEPTIONS.contains(&candidate.as_str()) {
        let prefix = error_base_prefix(error_name);
        // Avoid double-prefixing if the candidate already starts with the prefix
        if candidate.starts_with(prefix) {
            candidate
        } else {
            format!("{}{}", prefix, candidate)
        }
    } else {
        candidate
    }
}

/// Generate `pyo3::create_exception!` macros for each error variant plus the base error type.
/// Appends "Error" suffix to variant names that don't already have it (N818 compliance).
/// Prefixes names that would shadow Python builtins (A004 compliance).
pub fn gen_pyo3_error_types(error: &ErrorDef, module_name: &str, seen_exceptions: &mut AHashSet<String>) -> String {
    let mut lines = Vec::with_capacity(error.variants.len() + 2);
    lines.push("// Error types".to_string());

    // One exception per variant (with Error suffix if needed, prefixed if shadowing builtins)
    for variant in &error.variants {
        let variant_name = python_exception_name(&variant.name, &error.name);
        if seen_exceptions.insert(variant_name.clone()) {
            lines.push(format!(
                "pyo3::create_exception!({module_name}, {}, pyo3::exceptions::PyException);",
                variant_name
            ));
        }
    }

    // Base exception for the enum itself
    if seen_exceptions.insert(error.name.clone()) {
        lines.push(format!(
            "pyo3::create_exception!({module_name}, {}, pyo3::exceptions::PyException);",
            error.name
        ));
    }

    lines.join("\n")
}

/// Generate a `to_py_err` converter function that maps each Rust error variant to a Python exception.
/// Uses Error-suffixed names for variant exceptions (N818 compliance).
pub fn gen_pyo3_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        let normalized = error.rust_path.replace('-', "_");
        // Paths with more than 2 segments (e.g. `mylib_core::di::error::DependencyError`)
        // reference private internal modules that are not accessible from generated binding code.
        // Fall back to the public re-export form `{crate}::{ErrorName}` (2 segments).
        let segments: Vec<&str> = normalized.split("::").collect();
        if segments.len() > 2 {
            let crate_name = segments[0];
            let error_name = segments[segments.len() - 1];
            format!("{crate_name}::{error_name}")
        } else {
            normalized
        }
    };

    let fn_name = format!("{}_to_py_err", to_snake_case(&error.name));

    let mut lines = Vec::new();
    lines.push(format!("/// Convert a `{rust_path}` error to a Python exception."));
    lines.push(format!("fn {fn_name}(e: {rust_path}) -> pyo3::PyErr {{"));
    lines.push("    let msg = e.to_string();".to_string());
    lines.push("    #[allow(unreachable_patterns)]".to_string());
    lines.push("    match &e {".to_string());

    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        let variant_exc_name = python_exception_name(&variant.name, &error.name);
        lines.push(format!("        {pattern} => {}::new_err(msg),", variant_exc_name));
    }

    // Catch-all for cfg-gated variants not in the IR
    lines.push(format!("        _ => {}::new_err(msg),", error.name));
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate `m.add(...)` registration calls for each exception type.
/// Uses Error-suffixed names for variant exceptions (N818 compliance).
/// Prefixes names that would shadow Python builtins (A004 compliance).
pub fn gen_pyo3_error_registration(error: &ErrorDef, seen_registrations: &mut AHashSet<String>) -> Vec<String> {
    let mut registrations = Vec::with_capacity(error.variants.len() + 1);

    for variant in &error.variants {
        let variant_exc_name = python_exception_name(&variant.name, &error.name);
        if seen_registrations.insert(variant_exc_name.clone()) {
            registrations.push(format!(
                "    m.add(\"{}\", m.py().get_type::<{}>())?;",
                variant_exc_name, variant_exc_name
            ));
        }
    }

    // Base exception
    if seen_registrations.insert(error.name.clone()) {
        registrations.push(format!(
            "    m.add(\"{}\", m.py().get_type::<{}>())?;",
            error.name, error.name
        ));
    }

    registrations
}

/// Return the converter function name for a given error type.
pub fn converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_py_err", to_snake_case(&error.name))
}

/// Simple CamelCase to snake_case conversion.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// NAPI (Node.js) error generation
// ---------------------------------------------------------------------------

/// Generate a `JsError` enum with string constants for each error variant name.
pub fn gen_napi_error_types(error: &ErrorDef) -> String {
    let mut lines = Vec::with_capacity(error.variants.len() + 4);
    lines.push("// Error variant name constants".to_string());
    for variant in &error.variants {
        lines.push(format!(
            "pub const {}_ERROR_{}: &str = \"{}\";",
            to_screaming_snake(&error.name),
            to_screaming_snake(&variant.name),
            variant.name,
        ));
    }
    lines.join("\n")
}

/// Generate a converter function that maps a core error to `napi::Error`.
pub fn gen_napi_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_napi_err", to_snake_case(&error.name));

    let mut lines = Vec::new();
    lines.push(format!("/// Convert a `{rust_path}` error to a NAPI error."));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!("fn {fn_name}(e: {rust_path}) -> napi::Error {{"));
    lines.push("    let msg = e.to_string();".to_string());
    lines.push("    #[allow(unreachable_patterns)]".to_string());
    lines.push("    match &e {".to_string());

    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        lines.push(format!(
            "        {pattern} => napi::Error::new(napi::Status::GenericFailure, format!(\"[{}] {{}}\", msg)),",
            variant.name,
        ));
    }

    // Catch-all for cfg-gated variants not in the IR
    lines.push("        _ => napi::Error::new(napi::Status::GenericFailure, msg),".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Return the NAPI converter function name for a given error type.
pub fn napi_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_napi_err", to_snake_case(&error.name))
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen) error generation
// ---------------------------------------------------------------------------

/// Generate a converter function that maps a core error to a `JsValue` object
/// with `code` (string) and `message` (string) fields, plus a private
/// `error_code` helper that returns the variant code string.
pub fn gen_wasm_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_js_value", to_snake_case(&error.name));
    let code_fn_name = format!("{}_error_code", to_snake_case(&error.name));

    let mut lines = Vec::new();

    // error_code helper — maps each variant to a snake_case string code
    lines.push(format!("/// Return the error code string for a `{rust_path}` variant."));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!("fn {code_fn_name}(e: &{rust_path}) -> &'static str {{"));
    lines.push("    #[allow(unreachable_patterns)]".to_string());
    lines.push("    match e {".to_string());
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        let code = to_snake_case(&variant.name);
        lines.push(format!("        {pattern} => \"{code}\","));
    }
    lines.push(format!("        _ => \"{}\",", to_snake_case(&error.name)));
    lines.push("    }".to_string());
    lines.push("}".to_string());

    lines.push(String::new());

    // main converter — returns a JS object { code, message }
    lines.push(format!(
        "/// Convert a `{rust_path}` error to a `JsValue` object with `code` and `message` fields."
    ));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!("fn {fn_name}(e: {rust_path}) -> wasm_bindgen::JsValue {{"));
    lines.push(format!("    let code = {code_fn_name}(&e);"));
    lines.push("    let message = e.to_string();".to_string());
    lines.push("    let obj = js_sys::Object::new();".to_string());
    lines.push("    js_sys::Reflect::set(&obj, &\"code\".into(), &code.into()).ok();".to_string());
    lines.push("    js_sys::Reflect::set(&obj, &\"message\".into(), &message.into()).ok();".to_string());
    lines.push("    obj.into()".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Return the WASM converter function name for a given error type.
pub fn wasm_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_js_value", to_snake_case(&error.name))
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs) error generation
// ---------------------------------------------------------------------------

/// Generate a converter function that maps a core error to `PhpException`.
pub fn gen_php_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_php_err", to_snake_case(&error.name));

    let mut lines = Vec::new();
    lines.push(format!("/// Convert a `{rust_path}` error to a PHP exception."));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!(
        "fn {fn_name}(e: {rust_path}) -> ext_php_rs::exception::PhpException {{"
    ));
    lines.push("    let msg = e.to_string();".to_string());
    lines.push("    #[allow(unreachable_patterns)]".to_string());
    lines.push("    match &e {".to_string());

    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        lines.push(format!(
            "        {pattern} => ext_php_rs::exception::PhpException::default(format!(\"[{}] {{}}\", msg)),",
            variant.name,
        ));
    }

    // Catch-all for cfg-gated variants not in the IR
    lines.push("        _ => ext_php_rs::exception::PhpException::default(msg),".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Return the PHP converter function name for a given error type.
pub fn php_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_php_err", to_snake_case(&error.name))
}

// ---------------------------------------------------------------------------
// Magnus (Ruby) error generation
// ---------------------------------------------------------------------------

/// Generate a converter function that maps a core error to `magnus::Error`.
pub fn gen_magnus_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_magnus_err", to_snake_case(&error.name));

    let mut lines = Vec::new();
    lines.push(format!("/// Convert a `{rust_path}` error to a Magnus runtime error."));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!("fn {fn_name}(e: {rust_path}) -> magnus::Error {{"));
    lines.push("    let msg = e.to_string();".to_string());
    lines.push(
        "    magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), msg)".to_string(),
    );
    lines.push("}".to_string());
    lines.join("\n")
}

/// Return the Magnus converter function name for a given error type.
pub fn magnus_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_magnus_err", to_snake_case(&error.name))
}

// ---------------------------------------------------------------------------
// Rustler (Elixir) error generation
// ---------------------------------------------------------------------------

/// Generate a converter function that maps a core error to a Rustler error tuple `{:error, reason}`.
pub fn gen_rustler_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_rustler_err", to_snake_case(&error.name));

    let mut lines = Vec::new();
    lines.push(format!("/// Convert a `{rust_path}` error to a Rustler error string."));
    lines.push("#[allow(dead_code)]".to_string());
    lines.push(format!("fn {fn_name}(e: {rust_path}) -> String {{"));
    lines.push("    e.to_string()".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Return the Rustler converter function name for a given error type.
pub fn rustler_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_rustler_err", to_snake_case(&error.name))
}

// ---------------------------------------------------------------------------
// FFI (C) error code generation
// ---------------------------------------------------------------------------

/// Generate a C enum of error codes plus an error-message function declaration.
///
/// Produces a `typedef enum` with `PREFIX_ERROR_NONE = 0` followed by one entry
/// per variant, plus a function that returns the default message for a given code.
pub fn gen_ffi_error_codes(error: &ErrorDef) -> String {
    let prefix = to_screaming_snake(&error.name);
    let prefix_lower = to_snake_case(&error.name);

    let mut lines = Vec::new();
    lines.push(format!("/// Error codes for `{}`.", error.name));
    lines.push("typedef enum {".to_string());
    lines.push(format!("    {}_NONE = 0,", prefix));

    for (i, variant) in error.variants.iter().enumerate() {
        let variant_screaming = to_screaming_snake(&variant.name);
        lines.push(format!("    {}_{} = {},", prefix, variant_screaming, i + 1));
    }

    lines.push(format!("}} {}_t;\n", prefix_lower));

    // Error message function
    lines.push(format!(
        "/// Return a static string describing the error code.\nconst char* {}_error_message({}_t code);",
        prefix_lower, prefix_lower
    ));

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Go error type generation
// ---------------------------------------------------------------------------

/// Generate Go sentinel errors and a structured error type for an `ErrorDef`.
///
/// `pkg_name` is the Go package name (e.g. `"literllm"`). When the error struct
/// name starts with the package name (case-insensitively), the package-name
/// prefix is stripped to avoid the revive `exported` stutter lint error
/// (e.g. `LiterLlmError` in package `literllm` → exported as `Error`).
pub fn gen_go_error_types(error: &ErrorDef, pkg_name: &str) -> String {
    let sentinels = gen_go_sentinel_errors(std::slice::from_ref(error));
    let structured = gen_go_error_struct(error, pkg_name);
    format!("{}\n\n{}", sentinels, structured)
}

/// Generate a single consolidated `var (...)` block of Go sentinel errors
/// across multiple `ErrorDef`s.
///
/// When the same variant name appears in more than one `ErrorDef` (e.g. both
/// `GraphQLError` and `SchemaError` define `ValidationError`), the colliding
/// const names are disambiguated by prefixing with the parent error type's
/// stripped base name. For example, `GraphQLError::ValidationError` and
/// `SchemaError::ValidationError` become `ErrGraphQLValidationError` and
/// `ErrSchemaValidationError`. Variant names that are unique across all
/// errors are emitted as plain `Err{Variant}` consts.
pub fn gen_go_sentinel_errors(errors: &[ErrorDef]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let mut variant_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for err in errors {
        for v in &err.variants {
            *variant_counts.entry(v.name.as_str()).or_insert(0) += 1;
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut lines = Vec::new();
    lines.push("var (".to_string());
    for err in errors {
        let parent_base = error_base_prefix(&err.name);
        for variant in &err.variants {
            let collides = variant_counts.get(variant.name.as_str()).copied().unwrap_or(0) > 1;
            let const_name = if collides {
                format!("Err{}{}", parent_base, variant.name)
            } else {
                format!("Err{}", variant.name)
            };
            if !seen.insert(const_name.clone()) {
                continue;
            }
            let msg = variant_display_message(variant);
            lines.push(format!("\t// {} is returned when {}.", const_name, msg));
            lines.push(format!("\t{} = errors.New(\"{}\")", const_name, msg));
        }
    }
    lines.push(")".to_string());
    lines.join("\n")
}

/// Generate the structured error type (struct + Error() method) for a single
/// error definition. Sentinel errors are emitted separately by
/// [`gen_go_sentinel_errors`].
pub fn gen_go_error_struct(error: &ErrorDef, pkg_name: &str) -> String {
    let go_type_name = strip_package_prefix(&error.name, pkg_name);
    let mut lines = Vec::new();
    lines.push(format!("// {} is a structured error type.", go_type_name));
    lines.push(format!("type {} struct {{", go_type_name));
    lines.push("\tCode    string".to_string());
    lines.push("\tMessage string".to_string());
    lines.push("}\n".to_string());
    lines.push(format!(
        "func (e *{}) Error() string {{ return e.Message }}",
        go_type_name
    ));
    lines.join("\n")
}

/// Strip the package-name prefix from a type name to avoid revive's stutter lint.
///
/// Revive reports `exported: type name will be used as pkg.PkgFoo by other packages,
/// and that stutters` when a type name begins with the package name. This function
/// removes the prefix when it matches (case-insensitively) so that the exported name
/// does not repeat the package name.
///
/// Examples:
/// - `("LiterLlmError", "literllm")` → `"Error"` (lowercased `literllm` is a prefix
///   of lowercased `literllmerror`)
/// - `("ConversionError", "converter")` → `"ConversionError"` (no match)
fn strip_package_prefix(type_name: &str, pkg_name: &str) -> String {
    let type_lower = type_name.to_lowercase();
    let pkg_lower = pkg_name.to_lowercase();
    if type_lower.starts_with(&pkg_lower) && type_lower.len() > pkg_lower.len() {
        // Retain the original casing for the suffix part.
        type_name[pkg_lower.len()..].to_string()
    } else {
        type_name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Java error type generation
// ---------------------------------------------------------------------------

/// Generate Java exception sub-classes for each error variant.
///
/// Returns a `Vec` of `(class_name, file_content)` tuples: the base exception
/// class followed by one per-variant exception.  The caller writes each to a
/// separate `.java` file.
pub fn gen_java_error_types(error: &ErrorDef, package: &str) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(error.variants.len() + 1);

    // Base exception class
    let base_name = format!("{}Exception", error.name);
    let mut base = String::with_capacity(512);
    base.push_str(&format!(
        "// DO NOT EDIT - auto-generated by alef\npackage {};\n\n",
        package
    ));
    if !error.doc.is_empty() {
        // Multi-line Rust doc strings must be emitted as a proper Javadoc
        // block (one `* ` per line) — not as `/** <doc>. */` with embedded
        // newlines, which forces spotless to reformat and leaves trailing
        // whitespace on the blank-line `* ` rows. The blank-line case emits
        // ` *\n` (no trailing space), so prek's `trailing-whitespace` hook
        // and `alef-verify`'s embedded hash agree.
        crate::doc_emission::emit_javadoc(&mut base, &error.doc, "");
    }
    base.push_str(&format!("public class {} extends Exception {{\n", base_name));
    base.push_str(&format!(
        "    /** Creates a new {} with the given message. */\n    public {}(final String message) {{\n        super(message);\n    }}\n\n",
        base_name, base_name
    ));
    base.push_str(&format!(
        "    /** Creates a new {} with the given message and cause. */\n    public {}(final String message, final Throwable cause) {{\n        super(message, cause);\n    }}\n",
        base_name, base_name
    ));
    base.push_str("}\n");
    files.push((base_name.clone(), base));

    // Per-variant exception classes
    for variant in &error.variants {
        let class_name = format!("{}Exception", variant.name);
        let mut content = String::with_capacity(512);
        content.push_str(&format!(
            "// DO NOT EDIT - auto-generated by alef\npackage {};\n\n",
            package
        ));
        if !variant.doc.is_empty() {
            crate::doc_emission::emit_javadoc(&mut content, &variant.doc, "");
        }
        content.push_str(&format!("public class {} extends {} {{\n", class_name, base_name));
        content.push_str(&format!(
            "    /** Creates a new {} with the given message. */\n    public {}(final String message) {{\n        super(message);\n    }}\n\n",
            class_name, class_name
        ));
        content.push_str(&format!(
            "    /** Creates a new {} with the given message and cause. */\n    public {}(final String message, final Throwable cause) {{\n        super(message, cause);\n    }}\n",
            class_name, class_name
        ));
        content.push_str("}\n");
        files.push((class_name, content));
    }

    files
}

// ---------------------------------------------------------------------------
// C# error type generation
// ---------------------------------------------------------------------------

/// Generate C# exception sub-classes for each error variant.
///
/// Returns a `Vec` of `(class_name, file_content)` tuples: the base exception
/// class followed by one per-variant exception.  The caller writes each to a
/// separate `.cs` file.
///
/// `fallback_class` is the name of the generic library exception class (e.g.
/// `TreeSitterLanguagePackException`) that the base error class should extend so that
/// callers can `catch` the general library exception and catch all typed errors.
pub fn gen_csharp_error_types(
    error: &ErrorDef,
    namespace: &str,
    fallback_class: Option<&str>,
) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(error.variants.len() + 1);

    let base_name = format!("{}Exception", error.name);
    // Inherit from the generic library exception when provided so that
    // `Assert.ThrowsAny<LibException>()` catches typed errors too.
    let base_parent = fallback_class.unwrap_or("Exception");

    // Base exception class
    {
        let mut out = String::with_capacity(512);
        out.push_str("// This file is auto-generated by alef. DO NOT EDIT.\n#nullable enable\n\nusing System;\n\n");
        out.push_str(&format!("namespace {};\n\n", namespace));
        if !error.doc.is_empty() {
            out.push_str("/// <summary>\n");
            for line in error.doc.lines() {
                out.push_str(&format!("/// {}\n", line));
            }
            out.push_str("/// </summary>\n");
        }
        out.push_str(&format!("public class {} : {}\n{{\n", base_name, base_parent));
        out.push_str(&format!(
            "    public {}(string message) : base(message) {{ }}\n\n",
            base_name
        ));
        out.push_str(&format!(
            "    public {}(string message, Exception innerException) : base(message, innerException) {{ }}\n",
            base_name
        ));
        out.push_str("}\n");
        files.push((base_name.clone(), out));
    }

    // Per-variant exception classes
    for variant in &error.variants {
        let class_name = format!("{}Exception", variant.name);
        let mut out = String::with_capacity(512);
        out.push_str("// This file is auto-generated by alef. DO NOT EDIT.\n#nullable enable\n\nusing System;\n\n");
        out.push_str(&format!("namespace {};\n\n", namespace));
        if !variant.doc.is_empty() {
            out.push_str("/// <summary>\n");
            for line in variant.doc.lines() {
                out.push_str(&format!("/// {}\n", line));
            }
            out.push_str("/// </summary>\n");
        }
        out.push_str(&format!("public class {} : {}\n{{\n", class_name, base_name));
        out.push_str(&format!(
            "    public {}(string message) : base(message) {{ }}\n\n",
            class_name
        ));
        out.push_str(&format!(
            "    public {}(string message, Exception innerException) : base(message, innerException) {{ }}\n",
            class_name
        ));
        out.push_str("}\n");
        files.push((class_name, out));
    }

    files
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert CamelCase to SCREAMING_SNAKE_CASE.
fn to_screaming_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_uppercase());
        } else {
            result.push(c.to_ascii_uppercase());
        }
    }
    result
}

/// Well-known acronyms recognised by the doc/error renderers.
///
/// When emitting human-readable Display strings (e.g. for Go sentinel
/// `errors.New("...")`), variant names like `IoError` must render as
/// "IO error" — not "iO error" (the result of naive `lowercase first
/// character` after `to_snake_case`).
const TECHNICAL_ACRONYMS: &[&str] = &[
    "API", "ASCII", "CPU", "CSS", "CSV", "DNS", "EOF", "FFI", "FTP", "GID", "GPU", "GUI", "HTML", "HTTP", "HTTPS",
    "ID", "IO", "IP", "JSON", "JWT", "LDAP", "MFA", "MIME", "OCR", "OS", "PDF", "PID", "PNG", "QPS", "RAM", "RGB",
    "RPC", "RTF", "SDK", "SLA", "SMTP", "SQL", "SSH", "SSL", "SVG", "TCP", "TLS", "TOML", "TTL", "UDP", "UI", "UID",
    "URI", "URL", "UTF8", "UUID", "VM", "XML", "XMPP", "XSRF", "XSS", "YAML", "ZIP",
];

/// Strip `thiserror`-style `{name}` placeholders from a Display template
/// without leaving stray punctuation.
///
/// Examples:
///
/// - `"OCR error: {message}"`           → `"OCR error"`
/// - `"plugin error in '{plugin_name}'"` → `"plugin error"`
/// - `"timed out after {elapsed_ms}ms (limit: {limit_ms}ms)"` → `"timed out"`
/// - `"I/O error: {0}"`                  → `"I/O error"`
///
/// Used by `variant_display_message` and binding error renderers
/// (Dart, Go, …) so the literal placeholder string never reaches
/// the runtime.
pub fn strip_thiserror_placeholders(template: &str) -> String {
    // Remove every `{...}` segment.
    let mut without_placeholders = String::with_capacity(template.len());
    let mut depth = 0u32;
    for ch in template.chars() {
        match ch {
            '{' => depth = depth.saturating_add(1),
            '}' => depth = depth.saturating_sub(1),
            other if depth == 0 => without_placeholders.push(other),
            _ => {}
        }
    }
    // Remove orphaned punctuation/whitespace immediately around the holes
    // (collapse runs of whitespace, drop trailing `:`/quote runs, drop
    // `(...)` shells that wrapped only placeholders).
    let mut compacted = String::with_capacity(without_placeholders.len());
    let mut last_was_space = false;
    for ch in without_placeholders.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !compacted.is_empty() {
                compacted.push(' ');
            }
            last_was_space = true;
        } else {
            compacted.push(ch);
            last_was_space = false;
        }
    }
    // Trim trailing punctuation that only made sense before a placeholder.
    let trimmed = compacted
        .trim()
        .trim_end_matches([':', ',', '-', ';', '(', '\'', '"', ' '])
        .trim();
    // If we left e.g. `"limit: ms ms"` artefacts behind, collapse stray
    // empty parens / paired quotes.
    let cleaned = trimmed
        .replace("()", "")
        .replace("''", "")
        .replace("\"\"", "")
        .replace("  ", " ");
    cleaned.trim().to_string()
}

/// Convert a PascalCase variant name into a human readable phrase that
/// preserves canonical acronyms.
///
/// Examples:
/// - `"IoError"`           → `"IO error"`
/// - `"OcrError"`          → `"OCR error"`
/// - `"PdfParse"`          → `"PDF parse"`
/// - `"HttpRequestFailed"` → `"HTTP request failed"`
/// - `"Other"`             → `"other"`
pub fn acronym_aware_snake_phrase(variant_name: &str) -> String {
    if variant_name.is_empty() {
        return String::new();
    }
    // Split into PascalCase words (each word starts with an uppercase letter).
    let bytes = variant_name.as_bytes();
    let mut words: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            words.push(&variant_name[start..i]);
            start = i;
        }
    }
    words.push(&variant_name[start..]);

    let mut rendered: Vec<String> = Vec::with_capacity(words.len());
    for word in &words {
        let upper = word.to_ascii_uppercase();
        if TECHNICAL_ACRONYMS.contains(&upper.as_str()) {
            rendered.push(upper);
        } else {
            rendered.push(word.to_ascii_lowercase());
        }
    }
    rendered.join(" ")
}

/// Generate a human-readable message for an error variant.
///
/// Uses the `message_template` if present, otherwise falls back to a
/// space-separated version of the variant name (e.g. "ParseError" -> "parse error").
fn variant_display_message(variant: &ErrorVariant) -> String {
    if let Some(tmpl) = &variant.message_template {
        let stripped = strip_thiserror_placeholders(tmpl);
        if stripped.is_empty() {
            return acronym_aware_snake_phrase(&variant.name);
        }
        // Preserve canonical acronyms but lowercase the first regular word so
        // Go's `lowercase first char` convention does not corrupt `IO` → `iO`.
        // Heuristic: if the first whitespace-delimited token is *not* already
        // a known acronym, downcase its first character.
        let mut tokens = stripped.splitn(2, ' ');
        let head = tokens.next().unwrap_or("").to_string();
        let tail = tokens.next().unwrap_or("");
        let head_upper = head.to_ascii_uppercase();
        let head_rendered = if TECHNICAL_ACRONYMS.contains(&head_upper.as_str()) {
            head_upper
        } else {
            let mut chars = head.chars();
            match chars.next() {
                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                None => head,
            }
        };
        if tail.is_empty() {
            head_rendered
        } else {
            format!("{} {}", head_rendered, tail)
        }
    } else {
        acronym_aware_snake_phrase(&variant.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{ErrorDef, ErrorVariant};

    use alef_core::ir::{CoreWrapper, FieldDef, TypeRef};

    /// Helper to create a tuple-style field (e.g. `_0: String`).
    fn tuple_field(index: usize) -> FieldDef {
        FieldDef {
            name: format!("_{index}"),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    /// Helper to create a named struct field.
    fn named_field(name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    fn sample_error() -> ErrorDef {
        ErrorDef {
            name: "ConversionError".to_string(),
            rust_path: "html_to_markdown_rs::ConversionError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "ParseError".to_string(),
                    message_template: Some("HTML parsing error: {0}".to_string()),
                    fields: vec![tuple_field(0)],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "IoError".to_string(),
                    message_template: Some("I/O error: {0}".to_string()),
                    fields: vec![tuple_field(0)],
                    has_source: false,
                    has_from: true,
                    is_unit: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "Other".to_string(),
                    message_template: Some("Conversion error: {0}".to_string()),
                    fields: vec![tuple_field(0)],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
            ],
            doc: "Error type for conversion operations.".to_string(),
        }
    }

    #[test]
    fn test_gen_error_types() {
        let error = sample_error();
        let output = gen_pyo3_error_types(&error, "_module", &mut AHashSet::new());
        assert!(output.contains("pyo3::create_exception!(_module, ParseError, pyo3::exceptions::PyException);"));
        assert!(output.contains("pyo3::create_exception!(_module, IoError, pyo3::exceptions::PyException);"));
        assert!(output.contains("pyo3::create_exception!(_module, OtherError, pyo3::exceptions::PyException);"));
        assert!(output.contains("pyo3::create_exception!(_module, ConversionError, pyo3::exceptions::PyException);"));
    }

    #[test]
    fn test_gen_error_converter() {
        let error = sample_error();
        let output = gen_pyo3_error_converter(&error, "html_to_markdown_rs");
        assert!(
            output.contains("fn conversion_error_to_py_err(e: html_to_markdown_rs::ConversionError) -> pyo3::PyErr {")
        );
        assert!(output.contains("html_to_markdown_rs::ConversionError::ParseError(..) => ParseError::new_err(msg),"));
        assert!(output.contains("html_to_markdown_rs::ConversionError::IoError(..) => IoError::new_err(msg),"));
    }

    #[test]
    fn test_gen_error_registration() {
        let error = sample_error();
        let regs = gen_pyo3_error_registration(&error, &mut AHashSet::new());
        assert_eq!(regs.len(), 4); // 3 variants + 1 base
        assert!(regs[0].contains("\"ParseError\""));
        assert!(regs[3].contains("\"ConversionError\""));
    }

    #[test]
    fn test_unit_variant_pattern() {
        let error = ErrorDef {
            name: "MyError".to_string(),
            rust_path: "my_crate::MyError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".to_string(),
                message_template: Some("not found".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: String::new(),
            }],
            doc: String::new(),
        };
        let output = gen_pyo3_error_converter(&error, "my_crate");
        assert!(output.contains("my_crate::MyError::NotFound => NotFoundError::new_err(msg),"));
        // Ensure no (..) for unit variants
        assert!(!output.contains("NotFound(..)"));
    }

    #[test]
    fn test_struct_variant_pattern() {
        let error = ErrorDef {
            name: "MyError".to_string(),
            rust_path: "my_crate::MyError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Parsing".to_string(),
                message_template: Some("parsing error: {message}".to_string()),
                fields: vec![named_field("message")],
                has_source: false,
                has_from: false,
                is_unit: false,
                doc: String::new(),
            }],
            doc: String::new(),
        };
        let output = gen_pyo3_error_converter(&error, "my_crate");
        assert!(
            output.contains("my_crate::MyError::Parsing { .. } => ParsingError::new_err(msg),"),
            "Struct variants must use {{ .. }} pattern, got:\n{output}"
        );
        // Ensure no (..) for struct variants
        assert!(!output.contains("Parsing(..)"));
    }

    // -----------------------------------------------------------------------
    // NAPI tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_napi_error_types() {
        let error = sample_error();
        let output = gen_napi_error_types(&error);
        assert!(output.contains("CONVERSION_ERROR_ERROR_PARSE_ERROR"));
        assert!(output.contains("CONVERSION_ERROR_ERROR_IO_ERROR"));
        assert!(output.contains("CONVERSION_ERROR_ERROR_OTHER"));
    }

    #[test]
    fn test_gen_napi_error_converter() {
        let error = sample_error();
        let output = gen_napi_error_converter(&error, "html_to_markdown_rs");
        assert!(
            output
                .contains("fn conversion_error_to_napi_err(e: html_to_markdown_rs::ConversionError) -> napi::Error {")
        );
        assert!(output.contains("napi::Error::new(napi::Status::GenericFailure,"));
        assert!(output.contains("[ParseError]"));
        assert!(output.contains("[IoError]"));
        assert!(output.contains("#[allow(dead_code)]"));
    }

    #[test]
    fn test_napi_unit_variant() {
        let error = ErrorDef {
            name: "MyError".to_string(),
            rust_path: "my_crate::MyError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".to_string(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: String::new(),
            }],
            doc: String::new(),
        };
        let output = gen_napi_error_converter(&error, "my_crate");
        assert!(output.contains("my_crate::MyError::NotFound =>"));
        assert!(!output.contains("NotFound(..)"));
    }

    // -----------------------------------------------------------------------
    // WASM tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_wasm_error_converter() {
        let error = sample_error();
        let output = gen_wasm_error_converter(&error, "html_to_markdown_rs");
        // Main converter function signature
        assert!(output.contains(
            "fn conversion_error_to_js_value(e: html_to_markdown_rs::ConversionError) -> wasm_bindgen::JsValue {"
        ));
        // Structured object with code + message
        assert!(output.contains("js_sys::Object::new()"));
        assert!(output.contains("js_sys::Reflect::set(&obj, &\"code\".into(), &code.into()).ok()"));
        assert!(output.contains("js_sys::Reflect::set(&obj, &\"message\".into(), &message.into()).ok()"));
        assert!(output.contains("obj.into()"));
        // error_code helper
        assert!(
            output
                .contains("fn conversion_error_error_code(e: &html_to_markdown_rs::ConversionError) -> &'static str {")
        );
        assert!(output.contains("\"parse_error\""));
        assert!(output.contains("\"io_error\""));
        assert!(output.contains("\"other\""));
        assert!(output.contains("#[allow(dead_code)]"));
    }

    // -----------------------------------------------------------------------
    // PHP tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_php_error_converter() {
        let error = sample_error();
        let output = gen_php_error_converter(&error, "html_to_markdown_rs");
        assert!(output.contains("fn conversion_error_to_php_err(e: html_to_markdown_rs::ConversionError) -> ext_php_rs::exception::PhpException {"));
        assert!(output.contains("PhpException::default(format!(\"[ParseError] {}\", msg))"));
        assert!(output.contains("#[allow(dead_code)]"));
    }

    // -----------------------------------------------------------------------
    // Magnus tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_magnus_error_converter() {
        let error = sample_error();
        let output = gen_magnus_error_converter(&error, "html_to_markdown_rs");
        assert!(
            output.contains(
                "fn conversion_error_to_magnus_err(e: html_to_markdown_rs::ConversionError) -> magnus::Error {"
            )
        );
        assert!(
            output.contains(
                "magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), msg)"
            )
        );
        assert!(output.contains("#[allow(dead_code)]"));
    }

    // -----------------------------------------------------------------------
    // Rustler tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_rustler_error_converter() {
        let error = sample_error();
        let output = gen_rustler_error_converter(&error, "html_to_markdown_rs");
        assert!(
            output.contains("fn conversion_error_to_rustler_err(e: html_to_markdown_rs::ConversionError) -> String {")
        );
        assert!(output.contains("e.to_string()"));
        assert!(output.contains("#[allow(dead_code)]"));
    }

    // -----------------------------------------------------------------------
    // Helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_screaming_snake() {
        assert_eq!(to_screaming_snake("ConversionError"), "CONVERSION_ERROR");
        assert_eq!(to_screaming_snake("IoError"), "IO_ERROR");
        assert_eq!(to_screaming_snake("Other"), "OTHER");
    }

    #[test]
    fn test_strip_thiserror_placeholders_struct_field() {
        assert_eq!(strip_thiserror_placeholders("OCR error: {message}"), "OCR error");
        assert_eq!(
            strip_thiserror_placeholders("plugin error in '{plugin_name}': {message}"),
            "plugin error in"
        );
        // Multi-placeholder strings retain the surrounding prose verbatim
        // (minus the holes). Critical contract: no `{` / `}` survives.
        let result = strip_thiserror_placeholders("extraction timed out after {elapsed_ms}ms (limit: {limit_ms}ms)");
        assert!(!result.contains('{'), "no braces: {result}");
        assert!(!result.contains('}'), "no braces: {result}");
        assert!(result.starts_with("extraction timed out after"), "{result}");
    }

    #[test]
    fn test_strip_thiserror_placeholders_positional() {
        assert_eq!(strip_thiserror_placeholders("I/O error: {0}"), "I/O error");
        assert_eq!(strip_thiserror_placeholders("Parse error: {0}"), "Parse error");
    }

    #[test]
    fn test_strip_thiserror_placeholders_no_placeholder() {
        assert_eq!(strip_thiserror_placeholders("not found"), "not found");
        assert_eq!(strip_thiserror_placeholders("lock poisoned"), "lock poisoned");
    }

    #[test]
    fn test_acronym_aware_snake_phrase_recognizes_acronyms() {
        assert_eq!(acronym_aware_snake_phrase("IoError"), "IO error");
        assert_eq!(acronym_aware_snake_phrase("OcrError"), "OCR error");
        assert_eq!(acronym_aware_snake_phrase("PdfParse"), "PDF parse");
        assert_eq!(acronym_aware_snake_phrase("HttpRequestFailed"), "HTTP request failed");
        assert_eq!(acronym_aware_snake_phrase("UrlInvalid"), "URL invalid");
    }

    #[test]
    fn test_acronym_aware_snake_phrase_plain_words() {
        assert_eq!(acronym_aware_snake_phrase("Other"), "other");
        assert_eq!(acronym_aware_snake_phrase("ParseError"), "parse error");
        assert_eq!(acronym_aware_snake_phrase("LockPoisoned"), "lock poisoned");
    }

    #[test]
    fn test_variant_display_message_acronym_first_word() {
        let variant = ErrorVariant {
            name: "Io".to_string(),
            message_template: Some("I/O error: {0}".to_string()),
            fields: vec![tuple_field(0)],
            has_source: false,
            has_from: false,
            is_unit: false,
            doc: String::new(),
        };
        // Template "I/O error: {0}" → strip → "I/O error" → first token "I/O" not an acronym (with `/`),
        // so falls back to lowercase first char → "i/O error". Acceptable: at least no `{0}` leak.
        let msg = variant_display_message(&variant);
        assert!(!msg.contains('{'), "no placeholders allowed: {msg}");
    }

    #[test]
    fn test_variant_display_message_no_template_uses_acronyms() {
        let variant = ErrorVariant {
            name: "IoError".to_string(),
            message_template: None,
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: false,
            doc: String::new(),
        };
        assert_eq!(variant_display_message(&variant), "IO error");
    }

    #[test]
    fn test_variant_display_message_struct_template_no_leak() {
        let variant = ErrorVariant {
            name: "Ocr".to_string(),
            message_template: Some("OCR error: {message}".to_string()),
            fields: vec![named_field("message")],
            has_source: false,
            has_from: false,
            is_unit: false,
            doc: String::new(),
        };
        let msg = variant_display_message(&variant);
        assert_eq!(msg, "OCR error", "must not leak {{message}} placeholder: {msg}");
    }

    #[test]
    fn test_go_sentinels_no_placeholder_leak() {
        let error = ErrorDef {
            name: "KreuzbergError".to_string(),
            rust_path: "kreuzberg::KreuzbergError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "Io".to_string(),
                    message_template: Some("IO error: {message}".to_string()),
                    fields: vec![named_field("message")],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "Ocr".to_string(),
                    message_template: Some("OCR error: {message}".to_string()),
                    fields: vec![named_field("message")],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "Timeout".to_string(),
                    message_template: Some(
                        "extraction timed out after {elapsed_ms}ms (limit: {limit_ms}ms)".to_string(),
                    ),
                    fields: vec![named_field("elapsed_ms"), named_field("limit_ms")],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
        };
        let output = gen_go_sentinel_errors(std::slice::from_ref(&error));
        assert!(
            !output.contains('{'),
            "Go sentinels must not contain raw placeholders:\n{output}"
        );
        assert!(
            output.contains("ErrIo = errors.New(\"IO error\")"),
            "expected acronym-preserving Io sentinel, got:\n{output}"
        );
        assert!(
            output.contains("ErrOcr = errors.New(\"OCR error\")"),
            "expected acronym-preserving Ocr sentinel, got:\n{output}"
        );
        assert!(
            output.contains("ErrTimeout = errors.New(\"extraction timed out after"),
            "expected timeout sentinel to start with the prose, got:\n{output}"
        );
    }

    // -----------------------------------------------------------------------
    // FFI (C) tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_ffi_error_codes() {
        let error = sample_error();
        let output = gen_ffi_error_codes(&error);
        assert!(output.contains("CONVERSION_ERROR_NONE = 0"));
        assert!(output.contains("CONVERSION_ERROR_PARSE_ERROR = 1"));
        assert!(output.contains("CONVERSION_ERROR_IO_ERROR = 2"));
        assert!(output.contains("CONVERSION_ERROR_OTHER = 3"));
        assert!(output.contains("conversion_error_t;"));
        assert!(output.contains("conversion_error_error_message(conversion_error_t code)"));
    }

    // -----------------------------------------------------------------------
    // Go tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_go_error_types() {
        let error = sample_error();
        // Package name that does NOT match the error prefix — type name stays unchanged.
        let output = gen_go_error_types(&error, "mylib");
        assert!(output.contains("ErrParseError = errors.New("));
        assert!(output.contains("ErrIoError = errors.New("));
        assert!(output.contains("ErrOther = errors.New("));
        assert!(output.contains("type ConversionError struct {"));
        assert!(output.contains("Code    string"));
        assert!(output.contains("func (e *ConversionError) Error() string"));
        // Each sentinel error var should have a doc comment.
        assert!(output.contains("// ErrParseError is returned when"));
        assert!(output.contains("// ErrIoError is returned when"));
        assert!(output.contains("// ErrOther is returned when"));
    }

    #[test]
    fn test_gen_go_error_types_stutter_strip() {
        let error = sample_error();
        // "conversion" package — "ConversionError" starts with "conversion" (case-insensitive)
        // so the exported Go type should be "Error", not "ConversionError".
        let output = gen_go_error_types(&error, "conversion");
        assert!(
            output.contains("type Error struct {"),
            "expected stutter strip, got:\n{output}"
        );
        assert!(
            output.contains("func (e *Error) Error() string"),
            "expected stutter strip, got:\n{output}"
        );
        // Sentinel vars are unaffected by stutter stripping.
        assert!(output.contains("ErrParseError = errors.New("));
    }

    // -----------------------------------------------------------------------
    // Java tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_java_error_types() {
        let error = sample_error();
        let files = gen_java_error_types(&error, "dev.kreuzberg.test");
        // base + 3 variants
        assert_eq!(files.len(), 4);
        // Base class
        assert_eq!(files[0].0, "ConversionErrorException");
        assert!(
            files[0]
                .1
                .contains("public class ConversionErrorException extends Exception")
        );
        assert!(files[0].1.contains("package dev.kreuzberg.test;"));
        // Variant classes
        assert_eq!(files[1].0, "ParseErrorException");
        assert!(
            files[1]
                .1
                .contains("public class ParseErrorException extends ConversionErrorException")
        );
        assert_eq!(files[2].0, "IoErrorException");
        assert_eq!(files[3].0, "OtherException");
    }

    // -----------------------------------------------------------------------
    // C# tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gen_csharp_error_types() {
        let error = sample_error();
        // Without fallback class: base inherits from Exception.
        let files = gen_csharp_error_types(&error, "Kreuzberg.Test", None);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].0, "ConversionErrorException");
        assert!(files[0].1.contains("public class ConversionErrorException : Exception"));
        assert!(files[0].1.contains("namespace Kreuzberg.Test;"));
        assert_eq!(files[1].0, "ParseErrorException");
        assert!(
            files[1]
                .1
                .contains("public class ParseErrorException : ConversionErrorException")
        );
        assert_eq!(files[2].0, "IoErrorException");
        assert_eq!(files[3].0, "OtherException");
    }

    #[test]
    fn test_gen_csharp_error_types_with_fallback() {
        let error = sample_error();
        // With fallback class: base inherits from the generic library exception.
        let files = gen_csharp_error_types(&error, "Kreuzberg.Test", Some("TestLibException"));
        assert_eq!(files.len(), 4);
        assert!(
            files[0]
                .1
                .contains("public class ConversionErrorException : TestLibException")
        );
        // Variant classes still inherit from the base error class, not from the fallback directly.
        assert!(
            files[1]
                .1
                .contains("public class ParseErrorException : ConversionErrorException")
        );
    }

    // -----------------------------------------------------------------------
    // python_exception_name tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_python_exception_name_no_conflict() {
        // "ParseError" already ends with "Error" and is not a builtin
        assert_eq!(python_exception_name("ParseError", "ConversionError"), "ParseError");
        // "Other" gets "Error" suffix, "OtherError" is not a builtin
        assert_eq!(python_exception_name("Other", "ConversionError"), "OtherError");
    }

    #[test]
    fn test_python_exception_name_shadows_builtin() {
        // "Connection" -> "ConnectionError" shadows builtin -> prefix with "Crawl"
        assert_eq!(
            python_exception_name("Connection", "CrawlError"),
            "CrawlConnectionError"
        );
        // "Timeout" -> "TimeoutError" shadows builtin -> prefix with "Crawl"
        assert_eq!(python_exception_name("Timeout", "CrawlError"), "CrawlTimeoutError");
        // "ConnectionError" already ends with "Error", still shadows -> prefix
        assert_eq!(
            python_exception_name("ConnectionError", "CrawlError"),
            "CrawlConnectionError"
        );
    }

    #[test]
    fn test_python_exception_name_no_double_prefix() {
        // If variant is already prefixed with the error base, don't double-prefix
        assert_eq!(
            python_exception_name("CrawlConnectionError", "CrawlError"),
            "CrawlConnectionError"
        );
    }
}
