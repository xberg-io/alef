pub fn gen_magnus_error_methods_struct(error: &ErrorDef, core_import: &str) -> String {
    if error.methods.is_empty() {
        return String::new();
    }

    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let struct_name = format!("{}Info", error.name);

    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut ctor_assignments = Vec::new();

    for method in &error.methods {
        match method.name.as_str() {
            "status_code" => {
                fields.push("    status_code: u16,".to_string());
                methods.push(
                    concat!(
                        "    /// HTTP status code for this error (0 means no associated status).\n",
                        "    pub fn status_code(&self) -> u16 {\n",
                        "        self.status_code\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        status_code: e.status_code(),".to_string());
            }
            "is_transient" => {
                fields.push("    is_transient: bool,".to_string());
                methods.push(
                    concat!(
                        "    /// Returns `true` if the error is transient and a retry may succeed.\n",
                        "    pub fn transient(&self) -> bool {\n",
                        "        self.is_transient\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        is_transient: e.is_transient(),".to_string());
            }
            "error_type" => {
                fields.push("    error_type: String,".to_string());
                methods.push(
                    concat!(
                        "    /// Machine-readable error category string for matching and logging.\n",
                        "    pub fn error_type(&self) -> String {\n",
                        "        self.error_type.clone()\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        error_type: e.error_type().to_string(),".to_string());
            }
            other => {
                methods.push(format!("    // Not emitted: method `{other}` on `{struct_name}`"));
            }
        }
    }

    let struct_def = format!(
        "#[magnus::wrap(class = \"{struct_name}\", free_immediately, size)]\npub struct {struct_name} {{\n{}\n}}",
        fields.join("\n")
    );

    let from_fn = format!(
        "#[allow(dead_code)]\nfn {snake_name}_info(e: &{rust_path}) -> {struct_name} {{\n    {struct_name} {{\n{}\n    }}\n}}",
        ctor_assignments.join("\n"),
        snake_name = to_snake_case(&error.name),
    );

    let impl_block = format!("impl {struct_name} {{\n{}\n}}", methods.join("\n\n"));

    format!("{struct_def}\n\n{from_fn}\n\n{impl_block}")
}

/// Returns the `define_class` + `define_method` registration lines for the error info struct.
pub fn magnus_error_methods_registrations(error: &ErrorDef) -> Vec<String> {
    if error.methods.is_empty() {
        return Vec::new();
    }
    let struct_name = format!("{}Info", error.name);
    let snake = to_snake_case(&error.name);
    let class_var = format!("{snake}_info_class");
    let mut lines = Vec::new();
    lines.push(format!(
        "    let {class_var} = module.define_class(\"{struct_name}\", ruby.class_object())?;"
    ));
    for method in &error.methods {
        let (ruby_name, rust_fn) = if method.name == "is_transient" {
            ("transient?".to_string(), "transient".to_string())
        } else {
            (method.name.clone(), method.name.clone())
        };
        lines.push(format!(
            "    {class_var}.define_method(\"{ruby_name}\", magnus::method!({struct_name}::{rust_fn}, 0))?;"
        ));
    }
    lines
}

/// Generate a converter function that maps a core error to `PhpException`.
pub fn gen_php_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_php_err", to_snake_case(&error.name));

    let mut variants = Vec::new();
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        variants.push((pattern, variant.name.clone()));
    }

    crate::codegen::template_env::render(
        "error_gen/php_error_converter.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
            variants => variants,
        },
    )
}

/// Return the PHP converter function name for a given error type.
pub fn php_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_php_err", to_snake_case(&error.name))
}

/// Generate a `#[php_class]` + `#[php_impl]` block for the error type, storing
/// the whitelisted introspection method return values as Rust fields exposed via
/// `#[php_method]`.
///
/// Returns an empty string when `error.methods` is empty.
pub fn gen_php_error_methods_impl(error: &ErrorDef, core_import: &str) -> String {
    if error.methods.is_empty() {
        return String::new();
    }

    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let struct_name = format!("{}Info", error.name);

    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut ctor_assignments = Vec::new();

    for method in &error.methods {
        match method.name.as_str() {
            "status_code" => {
                fields.push("    pub status_code: u16,".to_string());
                methods.push(
                    concat!(
                        "    /// HTTP status code for this error (0 means no associated status).\n",
                        "    pub fn status_code(&self) -> u16 {\n",
                        "        self.status_code\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        status_code: e.status_code(),".to_string());
            }
            "is_transient" => {
                fields.push("    pub is_transient: bool,".to_string());
                methods.push(
                    concat!(
                        "    /// Returns `true` if the error is transient and a retry may succeed.\n",
                        "    pub fn is_transient(&self) -> bool {\n",
                        "        self.is_transient\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        is_transient: e.is_transient(),".to_string());
            }
            "error_type" => {
                fields.push("    pub error_type: String,".to_string());
                methods.push(
                    concat!(
                        "    /// Machine-readable error category string for matching and logging.\n",
                        "    pub fn error_type(&self) -> String {\n",
                        "        self.error_type.clone()\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        error_type: e.error_type().to_string(),".to_string());
            }
            other => {
                methods.push(format!("    // Not emitted: method for `{other}` on `{struct_name}`"));
            }
        }
    }

    let struct_def = format!("#[php_class]\npub struct {struct_name} {{\n{}\n}}", fields.join("\n"));

    let from_fn = format!(
        "#[allow(dead_code)]\nfn {snake_name}_info(e: &{rust_path}) -> {struct_name} {{\n    {struct_name} {{\n{}\n    }}\n}}",
        ctor_assignments.join("\n"),
        snake_name = to_snake_case(&error.name),
    );

    let impl_block = format!("#[php_impl]\nimpl {struct_name} {{\n{}\n}}", methods.join("\n\n"));

    format!("{struct_def}\n\n{from_fn}\n\n{impl_block}")
}

/// Generate a converter function that maps a core error to `magnus::Error`.
pub fn gen_magnus_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_magnus_err", to_snake_case(&error.name));

    crate::codegen::template_env::render(
        "error_gen/magnus_error_converter.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
        },
    )
}

/// Return the Magnus converter function name for a given error type.
pub fn magnus_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_magnus_err", to_snake_case(&error.name))
}

/// Generate a converter function that maps a core error to a Rustler error tuple `{:error, reason}`.
pub fn gen_rustler_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_rustler_err", to_snake_case(&error.name));

    crate::codegen::template_env::render(
        "error_gen/rustler_error_converter.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
        },
    )
}

/// Return the Rustler converter function name for a given error type.
pub fn rustler_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_rustler_err", to_snake_case(&error.name))
}

/// Generate a C enum of error codes plus an error-message function declaration.
///
/// Produces a `typedef enum` with `PREFIX_ERROR_NONE = 0` followed by one entry
/// per variant, plus a function that returns the default message for a given code.
pub fn gen_ffi_error_codes(error: &ErrorDef) -> String {
    let prefix = to_screaming_snake(&error.name);
    let prefix_lower = to_snake_case(&error.name);

    let mut variant_variants = Vec::new();
    for (i, variant) in error.variants.iter().enumerate() {
        let variant_screaming = to_screaming_snake(&variant.name);
        variant_variants.push((variant_screaming, (i + 1).to_string()));
    }

    crate::codegen::template_env::render(
        "error_gen/ffi_error_codes.jinja",
        minijinja::context! {
            error_name => error.name.as_str(),
            prefix => prefix.as_str(),
            prefix_lower => prefix_lower.as_str(),
            variant_variants => variant_variants,
        },
    )
}

/// Generate `#[no_mangle] extern "C"` helper functions for the whitelisted
/// introspection methods (`status_code`, `is_transient`, `error_type`) declared
/// in `error.methods`.
///
/// Each function follows the opaque-pointer convention: accepts a
/// `*const {rust_path}` (null-checked before dereference) and returns the
/// method's value. For `error_type` an additional `*_error_type_free` companion
/// is emitted so callers can release the `CString`-allocated memory.
///
/// Returns an empty string when `error.methods` is empty.
pub fn gen_ffi_error_methods(error: &ErrorDef, core_import: &str, api_prefix: &str) -> String {
    if error.methods.is_empty() {
        return String::new();
    }

    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let error_snake = to_snake_case(&error.name);
    let mut items: Vec<String> = Vec::new();

    for method in &error.methods {
        match method.name.as_str() {
            "status_code" => {
                let fn_name = format!("{api_prefix}_{error_snake}_status_code");
                items.push(format!(
                    "/// Return the HTTP status code for the error pointed to by `err`.\n\
                     /// Returns `0` if `err` is null.\n\
                     #[no_mangle]\n\
                     pub unsafe extern \"C\" fn {fn_name}(err: *const {rust_path}) -> u16 {{\n\
                         // SAFETY: caller guarantees `err` points to a live `{rust_path}` value\n\
                         // allocated by this library, or is null.\n\
                         if err.is_null() {{\n\
                             return 0;\n\
                         }}\n\
                         (*err).status_code()\n\
                     }}"
                ));
            }
            "is_transient" => {
                let fn_name = format!("{api_prefix}_{error_snake}_is_transient");
                items.push(format!(
                    "/// Return whether the error pointed to by `err` is transient.\n\
                     /// Returns `false` if `err` is null.\n\
                     #[no_mangle]\n\
                     pub unsafe extern \"C\" fn {fn_name}(err: *const {rust_path}) -> bool {{\n\
                         // SAFETY: caller guarantees `err` points to a live `{rust_path}` value\n\
                         // allocated by this library, or is null.\n\
                         if err.is_null() {{\n\
                             return false;\n\
                         }}\n\
                         (*err).is_transient()\n\
                     }}"
                ));
            }
            "error_type" => {
                let fn_name = format!("{api_prefix}_{error_snake}_error_type");
                let free_fn_name = format!("{fn_name}_free");
                items.push(format!(
                    "/// Return the machine-readable error category string for the error pointed\n\
                     /// to by `err` as a heap-allocated, NUL-terminated C string.\n\
                     /// The caller must free the returned pointer with `{free_fn_name}`.\n\
                     /// Returns a null pointer if `err` is null.\n\
                     #[no_mangle]\n\
                     pub unsafe extern \"C\" fn {fn_name}(err: *const {rust_path}) -> *mut std::ffi::c_char {{\n\
                         // SAFETY: caller guarantees `err` points to a live `{rust_path}` value\n\
                         // allocated by this library, or is null.\n\
                         if err.is_null() {{\n\
                             return std::ptr::null_mut();\n\
                         }}\n\
                         let s = (*err).error_type();\n\
                         // SAFETY: `error_type()` returns a `'static str` containing no NUL bytes.\n\
                         std::ffi::CString::new(s)\n\
                             .map(|c| c.into_raw())\n\
                             .unwrap_or(std::ptr::null_mut())\n\
                     }}\n\n\
                     /// Free a string previously returned by `{fn_name}`.\n\
                     /// Passing a null pointer is a no-op.\n\
                     #[no_mangle]\n\
                     pub unsafe extern \"C\" fn {free_fn_name}(ptr: *mut std::ffi::c_char) {{\n\
                         // SAFETY: `ptr` was allocated by `CString::into_raw` inside\n\
                         // `{fn_name}` and is now being reclaimed by the matching\n\
                         // `CString::from_raw`.  Passing null is explicitly allowed.\n\
                         if !ptr.is_null() {{\n\
                             drop(std::ffi::CString::from_raw(ptr));\n\
                         }}\n\
                     }}"
                ));
            }
            other => {
                items.push(format!(
                    "// Not emitted: FFI helper for method `{other}` on `{rust_path}`"
                ));
            }
        }
    }

    items.join("\n\n")
}

/// Generate Go sentinel errors and a structured error type for an `ErrorDef`.
///
/// `pkg_name` is the Go package name (e.g. `"samplellm"`). When the error struct
/// name starts with the package name (case-insensitively), the package-name
/// prefix is stripped to avoid the revive `exported` stutter lint error
/// (e.g. `SampleLlmError` in package `samplellm` → exported as `Error`).
use crate::core::ir::ErrorDef;

use super::shared::{error_variant_wildcard_pattern, to_screaming_snake, to_snake_case};
