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
        crate::alef_context! {
            variants => variants,
        },
    )
}

/// Marker that starts the message of the `cause` error that carries the structured properties
/// of a thrown error. The generated `binding.cjs` strips the cause and assigns the properties
/// to the thrown error. `napi::Error` holds only a status string, a message and a cause, so the
/// cause is the one slot that crosses from an async Rust function to the JS error object.
pub const NAPI_ERROR_INFO_MARKER: &str = "alef:error-info:";

/// One own property of a thrown Node error: the same name and type in the Rust payload, the
/// `binding.cjs` shim and the `.d.ts`.
pub struct NapiErrorProperty {
    pub js_name: String,
    pub ts_type: String,
    /// Display expression for a JSON fragment, reading converter locals `e`, `kind` and `error_code`. ~keep
    pub json_expr: String,
}

/// The properties attached to every error thrown for `error`, in a fixed order.
///
/// `kind` and `errorCode` are always present. `statusCode`, `isTransient`, `errorType` and the
/// duration getters (`retryAfter`, in whole milliseconds) follow the introspection methods
/// the core error type has, as `gen_napi_error_class` does for the `*Info` class.
pub fn napi_error_properties(error: &ErrorDef) -> Vec<NapiErrorProperty> {
    let kind_ts = if error.variants.is_empty() {
        "string".to_string()
    } else {
        let mut names: Vec<String> = error.variants.iter().map(|v| format!("\"{}\"", v.name)).collect();
        names.push("\"Unknown\"".to_string());
        names.join(" | ")
    };
    let mut props = vec![
        NapiErrorProperty {
            js_name: "kind".to_string(),
            ts_type: kind_ts,
            json_expr: "serde_json::Value::String(kind.to_string())".to_string(),
        },
        NapiErrorProperty {
            js_name: "errorCode".to_string(),
            ts_type: "number".to_string(),
            json_expr: "error_code".to_string(),
        },
    ];
    props.extend(
        error
            .methods
            .iter()
            .filter_map(|method| napi_method_error_property(error, method)),
    );
    props
}

fn napi_method_error_property(error: &ErrorDef, method: &crate::core::ir::MethodDef) -> Option<NapiErrorProperty> {
    let (js_name, ts_type, json_expr) = match method.name.as_str() {
        "status_code" => ("statusCode", "number", "e.status_code()"),
        "is_transient" => ("isTransient", "boolean", "e.is_transient()"),
        "error_type" => (
            "errorType",
            "string",
            "serde_json::Value::String(e.error_type().to_string())",
        ),
        _ => return napi_duration_error_property(error, method),
    };
    Some(NapiErrorProperty {
        js_name: js_name.to_string(),
        ts_type: ts_type.to_string(),
        json_expr: json_expr.to_string(),
    })
}

fn napi_duration_error_property(error: &ErrorDef, method: &crate::core::ir::MethodDef) -> Option<NapiErrorProperty> {
    let (_, optional) = duration_methods(error)
        .into_iter()
        .find(|(item, _)| item.name == method.name)?;
    let call = millis_expr(&format!("e.{}()", method.name), optional, "i64");
    let (ts_type, json_expr) = if optional {
        ("number | null", format!("serde_json::json!({call})"))
    } else {
        ("number", call)
    };
    Some(NapiErrorProperty {
        js_name: crate::codegen::naming::public_host_identifier(
            crate::core::config::Language::Node,
            crate::codegen::naming::PublicIdentifierKind::Method,
            &method.name,
        ),
        ts_type: ts_type.to_string(),
        json_expr,
    })
}

/// Generate a converter function that maps a core error to `napi::Error`.
///
/// The returned error keeps the display text as its message and carries the structured
/// properties (see [`napi_error_properties`]) as a JSON payload in its `cause`.
pub fn gen_napi_error_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    let fn_name = format!("{}_to_napi_err", to_snake_case(&error.name));

    let unknown_code = crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN;
    let mut arms = Vec::with_capacity(error.variants.len());
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        let code = variant
            .taxonomy(&error.rust_path)
            .map_or(unknown_code, |taxonomy| taxonomy.code);
        arms.push((pattern, variant.name.clone(), code));
    }
    let props = napi_error_properties(error);
    let json_template = format!(
        "{{{{{}}}}}",
        props
            .iter()
            .map(|p| format!("\\\"{}\\\":{{}}", p.js_name))
            .collect::<Vec<_>>()
            .join(",")
    );
    let json_args: Vec<_> = props.iter().map(|p| p.json_expr.as_str()).collect();

    crate::codegen::template_env::render(
        "error_gen/napi_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path.as_str(),
            fn_name => fn_name.as_str(),
            arms => arms,
            unknown_code => unknown_code,
            json_template => json_template,
            json_args => json_args,
            marker => NAPI_ERROR_INFO_MARKER,
        },
    )
}

/// Generate `alef_napi_err`, the one conversion every generated Node call site uses.
///
/// A core error that has a typed converter (see [`gen_napi_error_converter`]) keeps its kind,
/// status and retry delay; any other `Display` error becomes a plain `GenericFailure` error. The
/// downcast lets a call site convert without naming the error type it came from.
pub fn gen_napi_error_dispatch(errors: &[ErrorDef], core_import: &str) -> String {
    gen_napi_error_dispatch_with_bridge_source(errors, core_import, None)
}

pub(crate) fn gen_napi_error_dispatch_with_bridge_source(
    errors: &[ErrorDef],
    core_import: &str,
    bridge_error_type: Option<&str>,
) -> String {
    let mut converters = Vec::new();
    for error in errors {
        let rust_path = if error.rust_path.is_empty() {
            format!("{core_import}::{}", error.name)
        } else {
            error.rust_path.replace('-', "_")
        };
        converters.push((rust_path, napi_converter_fn_name(error)));
    }
    let source_errors = bridge_error_type
        .map(|error_type| napi_source_error_paths(error_type, core_import, &converters))
        .unwrap_or_default();
    crate::codegen::template_env::render(
        "error_gen/napi_error_dispatch.jinja",
        crate::alef_context! { converters => converters, source_errors => source_errors },
    )
}

fn napi_source_error_paths(
    bridge_error: &str,
    core_import: &str,
    converters: &[(String, String)],
) -> Vec<(String, bool)> {
    let bridge_path = if bridge_error.contains("::") || bridge_error.contains('<') {
        bridge_error.to_owned()
    } else {
        format!("{core_import}::{bridge_error}")
    };
    let boxed = bridge_error.split_once('<').is_some_and(|(outer, _)| {
        matches!(
            outer.trim().trim_start_matches("::"),
            "Box" | "std::boxed::Box" | "alloc::boxed::Box"
        )
    });
    let mut paths = converters
        .iter()
        .filter(|(path, _)| *path != bridge_path)
        .map(|(path, _)| (path.clone(), false))
        .collect::<Vec<_>>();
    paths.insert(0, (bridge_path, boxed));
    paths
}

/// Return the NAPI converter function name for a given error type.
pub fn napi_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_napi_err", to_snake_case(&error.name))
}

/// The Rust path of `error` in the generated WASM crate, with the source-crate remaps applied.
fn wasm_error_rust_path(error: &ErrorDef, core_import: &str, source_remaps: &[(&str, &str)]) -> String {
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
    rust_path
}

/// The own properties set on a thrown WASM error, as `(name, JsValue expression)` pairs.
///
/// The names match the Node binding's thrown error (`kind`, `errorCode`, `statusCode`,
/// `isTransient`, `errorType`, and the duration getters such as `retryAfter` in whole
/// milliseconds, `null` when absent), so a JS caller reads one shape from either package. The
/// expressions read the converter locals `e`, `kind` and `error_code`.
fn wasm_error_properties(error: &ErrorDef) -> Vec<(String, String)> {
    let mut props = vec![
        ("kind".to_string(), "wasm_bindgen::JsValue::from_str(kind)".to_string()),
        (
            "errorCode".to_string(),
            "wasm_bindgen::JsValue::from_f64(f64::from(error_code))".to_string(),
        ),
    ];
    for method in &error.methods {
        match method.name.as_str() {
            "status_code" => props.push((
                "statusCode".to_string(),
                "wasm_bindgen::JsValue::from_f64(f64::from(e.status_code()))".to_string(),
            )),
            "is_transient" => props.push((
                "isTransient".to_string(),
                "wasm_bindgen::JsValue::from_bool(e.is_transient())".to_string(),
            )),
            "error_type" => props.push((
                "errorType".to_string(),
                match &method.return_type {
                    crate::core::ir::TypeRef::String if method.returns_ref => {
                        "wasm_bindgen::JsValue::from_str(e.error_type())"
                    }
                    crate::core::ir::TypeRef::String => "wasm_bindgen::JsValue::from_str(&e.error_type())",
                    _ => "wasm_bindgen::JsValue::from_str(&e.error_type().to_string())",
                }
                .to_string(),
            )),
            other => {
                let Some((_, optional)) = duration_methods(error).into_iter().find(|(m, _)| m.name == other) else {
                    continue;
                };
                let call = millis_expr(&format!("e.{other}()"), optional, "i64");
                let value = if optional {
                    format!(
                        "match {call} {{ Some(v) => wasm_bindgen::JsValue::from_f64(v as f64), None => wasm_bindgen::JsValue::NULL }}"
                    )
                } else {
                    format!("wasm_bindgen::JsValue::from_f64(({call}) as f64)")
                };
                props.push((heck::ToLowerCamelCase::to_lower_camel_case(other), value));
            }
        }
    }
    props
}

/// Generate a converter function that maps a core error to a JS `Error` object.
///
/// The `Error` keeps the display text as its `message`. It carries `code` (the snake-case variant
/// name), the structured properties of [`wasm_error_properties`], and the payload fields of the
/// variant. Complete safe payload fields are available under `Symbol.for("alef.error.payload")`;
/// `payload` aliases that object only when no existing metadata property uses the name. ~keep
/// A private `error_code` helper returns the variant code string.
pub fn gen_wasm_error_converter(error: &ErrorDef, core_import: &str, source_remaps: &[(&str, &str)]) -> String {
    let rust_path = wasm_error_rust_path(error, core_import, source_remaps);

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
        crate::alef_context! {
            rust_path => rust_path.as_str(),
            code_fn_name => code_fn_name.as_str(),
            variants => code_variants,
            default_code => default_code.as_str(),
        },
    );

    let converter_fn = render_wasm_converter(error, &rust_path, &fn_name, &code_fn_name);

    format!("{}\n\n{}", code_fn, converter_fn)
}

fn render_wasm_converter(error: &ErrorDef, rust_path: &str, fn_name: &str, code_fn_name: &str) -> String {
    let properties = wasm_error_properties(error);
    let payload_variants = wasm_payload::variants(error, rust_path, &properties);
    let attach_payload = !properties.iter().any(|(name, _)| name == "payload");
    let unknown_code = crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN;
    let kind_arms = error
        .variants
        .iter()
        .map(|variant| {
            let pattern = error_variant_wildcard_pattern(rust_path, variant);
            let code = variant
                .taxonomy(&error.rust_path)
                .map_or(unknown_code, |taxonomy| taxonomy.code);
            (pattern, variant.name.clone(), code)
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/wasm_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path,
            fn_name => fn_name,
            code_fn_name => code_fn_name,
            kind_arms => kind_arms,
            unknown_code => unknown_code,
            properties => properties,
            payload_variants => payload_variants,
            attach_payload => attach_payload,
        },
    )
}

/// Generate `alef_wasm_err`, the one conversion every generated WASM call site uses.
///
/// A core error that has a typed converter (see [`gen_wasm_error_converter`]) keeps its kind,
/// status and retry delay; any other `Display` error becomes a string, as before. The downcast
/// lets a call site convert without naming the error type it came from. `errors` must be the
/// errors whose converters the crate emits.
pub fn gen_wasm_error_dispatch(errors: &[&ErrorDef], core_import: &str, source_remaps: &[(&str, &str)]) -> String {
    let arms = errors
        .iter()
        .map(|error| {
            (
                wasm_error_rust_path(error, core_import, source_remaps),
                wasm_converter_fn_name(error),
            )
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/wasm_error_dispatch.jinja",
        crate::alef_context! { arms => arms },
    )
}

fn wasm_payload_type_is_safe(ty: &crate::core::ir::TypeRef) -> bool {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => wasm_payload_type_is_safe(inner),
        TypeRef::Map(key, value) => wasm_payload_type_is_safe(key) && wasm_payload_type_is_safe(value),
        TypeRef::Named(_) | TypeRef::Path | TypeRef::Unit | TypeRef::Duration => false,
    }
}

/// Return the WASM converter function name for a given error type.
pub fn wasm_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_js_value", to_snake_case(&error.name))
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
                let Some((_, optional)) = duration_methods(error).into_iter().find(|(m, _)| m.name == other) else {
                    continue;
                };
                format!(
                    "    /// `{other}` in whole milliseconds{}.\n    \
                     #[wasm_bindgen(js_name = \"{}\")]\n    \
                     pub fn {other}(&self) -> {} {{\n        \
                     {}\n    }}",
                    if optional { " (undefined when absent)" } else { "" },
                    heck::ToLowerCamelCase::to_lower_camel_case(other),
                    millis_type(optional, "u64"),
                    millis_expr(&format!("self.inner.{other}()"), optional, "u64"),
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
                let Some((_, optional)) = duration_methods(error).into_iter().find(|(m, _)| m.name == other) else {
                    continue;
                };
                let ty = millis_type(optional, "i64");
                fields.push(format!("    pub {other}: {ty},"));
                methods.push(format!(
                    "    /// `{other}` in whole milliseconds{}.\n    #[napi(js_name = \"{}\")]\n    pub fn {other}(&self) -> {ty} {{\n        self.{other}\n    }}",
                    if optional { " (null when absent)" } else { "" },
                    heck::ToLowerCamelCase::to_lower_camel_case(other),
                ));
                ctor_assignments.push(format!(
                    "        {other}: {},",
                    millis_expr(&format!("e.{other}()"), optional, "i64")
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

use super::shared::{
    duration_methods, error_variant_wildcard_pattern, millis_expr, millis_type, to_screaming_snake, to_snake_case,
};

#[cfg(test)]
#[path = "tests/wasm_borrowed_error_type.rs"]
mod wasm_borrowed_error_type;

#[path = "wasm_payload.rs"]
mod wasm_payload;
