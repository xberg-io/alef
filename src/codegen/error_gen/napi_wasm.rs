pub fn gen_napi_error_types(error: &ErrorDef) -> String {
    let mut variants = Vec::new();
    let error_screaming = to_screaming_snake(&error.name);
    for variant in &error.variants {
        let variant_const = format!("{}_ERROR_{}", error_screaming, to_screaming_snake(&variant.name));
        variants.push((
            variant_const,
            variant.name.clone(),
            variant
                .taxonomy(&error.rust_path)
                .map_or(crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN, |taxonomy| {
                    taxonomy.code
                }),
        ));
    }

    crate::codegen::template_env::render(
        "error_gen/napi_error_types.jinja",
        minijinja::context! {
            variants => variants,
        },
    )
}

/// Generate a converter function that maps a core error to `napi::Error`.
pub fn gen_napi_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_napi_err", to_snake_case(&error.name));

    crate::codegen::template_env::render(
        "error_gen/napi_error_converter.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
        },
    )
}

/// Return the NAPI converter function name for a given error type.
pub fn napi_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_napi_err", to_snake_case(&error.name))
}

/// Generate a converter function that maps a core error to a `JsValue` object
/// with `code` (string) and `message` (string) fields, plus a private
/// `error_code` helper that returns the variant code string.
pub fn gen_wasm_error_converter(error: &ErrorDef, core_import: &str, source_remaps: &[(&str, &str)]) -> String {
    let mut rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    for (orig_crate, target_crate) in source_remaps {
        if rust_path.starts_with(&format!("{orig_crate}::")) {
            rust_path = rust_path.replacen(&format!("{orig_crate}::"), &format!("{target_crate}::"), 1);
            break;
        }
    }

    let fn_name = format!("{}_to_js_value", to_snake_case(&error.name));
    let code_fn_name = format!("{}_error_code", to_snake_case(&error.name));

    let mut code_variants = Vec::new();
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        let code = to_snake_case(&variant.name);
        code_variants.push((pattern, code));
    }
    let default_code = to_snake_case(&error.name);

    let code_fn = crate::codegen::template_env::render(
        "error_gen/wasm_error_code_fn.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            code_fn_name => code_fn_name.as_str(),
            variants => code_variants,
            default_code => default_code.as_str(),
        },
    );

    let converter_fn = crate::codegen::template_env::render(
        "error_gen/wasm_error_converter.jinja",
        minijinja::context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
            code_fn_name => code_fn_name.as_str(),
        },
    );

    format!("{}\n\n{}", code_fn, converter_fn)
}

/// Return the WASM converter function name for a given error type.
pub fn wasm_converter_fn_name(error: &ErrorDef) -> String {
    wasm_converter_fn_name_for_type(&error.name)
}

pub(crate) fn wasm_converter_fn_name_for_type(error_type: &str) -> String {
    format!("{}_to_js_value", to_snake_case(error_type))
}

/// Generate a `#[wasm_bindgen]` opaque struct for an error type together with an
/// `impl` block that exposes the whitelisted introspection methods
/// (`status_code`, `is_transient`, `error_type`) declared in `error.methods`.
///
/// The struct follows the same `pub(crate) inner: CoreType` convention used by
/// all other opaque WASM handles in the codebase.
///
/// `wasm_prefix` is the full WASM type prefix string (from `config.wasm_type_prefix()`,
/// e.g. `"Wasm"`).  The generated struct name is `{wasm_prefix}{error.name}`
/// (e.g. `WasmSampleLlmError`).
///
/// Returns an empty string when `error.methods` is empty so callers can
/// unconditionally append the result without adding noise to the output file.
pub fn gen_wasm_error_methods(error: &ErrorDef, core_import: &str, wasm_prefix: &str) -> String {
    if error.methods.is_empty() {
        return String::new();
    }

    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let wasm_struct_name = format!("{wasm_prefix}{}", error.name);

    let struct_def = format!(
        "/// Opaque WASM handle for [`{rust_path}`] that exposes introspection methods.\n\
         #[wasm_bindgen]\n\
         pub struct {wasm_struct_name} {{\n\
             pub(crate) inner: {rust_path},\n\
         }}"
    );

    let mut method_bodies = Vec::new();
    for method in &error.methods {
        let method_src = match method.name.as_str() {
            "status_code" => "    /// HTTP status code for this error variant.\n    \
                 #[wasm_bindgen(js_name = \"statusCode\")]\n    \
                 pub fn status_code(&self) -> u16 {\n        \
                 self.inner.status_code()\n    }"
                .to_string(),
            "is_transient" => "    /// Returns `true` if the error is transient and a retry may succeed.\n    \
                 #[wasm_bindgen(js_name = \"isTransient\")]\n    \
                 pub fn is_transient(&self) -> bool {\n        \
                 self.inner.is_transient()\n    }"
                .to_string(),
            "error_type" => "    /// Returns a machine-readable error category string.\n    \
                 #[wasm_bindgen(js_name = \"errorType\")]\n    \
                 pub fn error_type(&self) -> String {\n        \
                 self.inner.error_type().to_string()\n    }"
                .to_string(),
            other => {
                format!(
                    "    // Not emitted: binding for method `{other}` on `{wasm_struct_name}`\n    \
                     #[allow(dead_code)]\n    \
                     pub fn {other}(&self) {{}}"
                )
            }
        };
        method_bodies.push(method_src);
    }

    let impl_block = format!(
        "#[wasm_bindgen]\nimpl {wasm_struct_name} {{\n{}\n}}",
        method_bodies.join("\n\n")
    );

    format!("{struct_def}\n\n{impl_block}")
}

/// Generate a `#[napi]` companion struct for error introspection, exposing
/// the whitelisted methods as `#[napi]` getter methods.
///
/// `napi::Error` (thrown as a generic JS `Error`) has no per-variant subclass to attach
/// structured data to, so we emit a separate `Js{ErrorName}Info` `#[napi]` class instead.
/// It always carries `code` — the stable numeric taxonomy code for the specific variant,
/// resolved via the same per-variant match the converter uses — plus whichever of
/// `status_code` / `is_transient` / `error_type` the error type implements. This is the
/// structured channel `code` belongs in, never the exception message text. (~keep)
///
/// Returns an empty string when `error.methods` is empty.
pub fn gen_napi_error_class(error: &ErrorDef, core_import: &str) -> String {
    if error.methods.is_empty() {
        return String::new();
    }

    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let snake_name = to_snake_case(&error.name);
    let code_fn_name = format!("{snake_name}_error_code");

    let mut fields = vec!["    pub code: u32,".to_string()];
    let mut methods = vec![
        concat!(
            "    /// Stable numeric error code identifying the specific error variant.\n",
            "    #[napi(js_name = \"code\")]\n",
            "    pub fn code(&self) -> u32 {\n",
            "        self.code\n",
            "    }",
        )
        .to_string(),
    ];
    let mut ctor_assignments = vec![format!("        code: {code_fn_name}(e),")];

    let struct_name = format!("Js{}Info", error.name);

    for method in &error.methods {
        match method.name.as_str() {
            "status_code" => {
                fields.push("    pub status_code: u16,".to_string());
                methods.push(
                    concat!(
                        "    /// HTTP status code for this error (0 means no associated status).\n",
                        "    #[napi(js_name = \"statusCode\")]\n",
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
                        "    #[napi(js_name = \"isTransient\")]\n",
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
                        "    #[napi(js_name = \"errorType\")]\n",
                        "    pub fn error_type(&self) -> String {\n",
                        "        self.error_type.clone()\n",
                        "    }",
                    )
                    .to_string(),
                );
                ctor_assignments.push("        error_type: e.error_type().to_string(),".to_string());
            }
            other => {
                methods.push(format!(
                    "    // Not emitted: #[napi] method `{other}` on `{struct_name}`"
                ));
            }
        }
    }

    let struct_def = format!("#[napi]\npub struct {struct_name} {{\n{}\n}}", fields.join("\n"));

    let mut code_match_arms = Vec::with_capacity(error.variants.len());
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        let code = variant
            .taxonomy(&error.rust_path)
            .map_or(crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN, |taxonomy| {
                taxonomy.code
            });
        code_match_arms.push(format!("        {pattern} => {code},"));
    }
    let unknown_code = crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN;
    let code_fn = format!(
        "/// Resolve the stable numeric error code for a `{rust_path}` variant.\n\
         #[allow(dead_code)]\n\
         fn {code_fn_name}(e: &{rust_path}) -> u32 {{\n\
             \x20   #[allow(unreachable_patterns)]\n\
             \x20   match e {{\n\
         {}\n\
             \x20       _ => {unknown_code},\n\
             \x20   }}\n\
         }}",
        code_match_arms.join("\n"),
    );

    let from_fn = format!(
        "#[allow(dead_code)]\nfn {snake_name}_info(e: &{rust_path}) -> {struct_name} {{\n    {struct_name} {{\n{}\n    }}\n}}",
        ctor_assignments.join("\n"),
    );

    let impl_block = format!("#[napi]\nimpl {struct_name} {{\n{}\n}}", methods.join("\n\n"));

    format!("{struct_def}\n\n{code_fn}\n\n{from_fn}\n\n{impl_block}")
}

/// Generate a Magnus-wrapped Rust struct that stores the whitelisted error
/// introspection method return values and exposes them as Ruby instance methods.
///
/// Returns an empty string when `error.methods` is empty.
use crate::core::ir::ErrorDef;

use super::shared::{error_variant_wildcard_pattern, to_screaming_snake, to_snake_case};
