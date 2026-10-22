/// One core error type the shims can read a kind, an HTTP status and a retry delay from.
#[derive(serde::Serialize)]
struct JniErrorContext {
    snake: String,
    rust_path: String,
    variants: Vec<JniErrorVariantContext>,
    status_expr: String,
    retry_after_expr: String,
}

#[derive(serde::Serialize)]
struct JniErrorVariantContext {
    name: String,
    pattern: String,
}

/// The expression that reads the HTTP status of `e`, or `-1` when the error has no `status_code`
/// method. A status of `0` means the variant has no HTTP status.
fn jni_error_status_expr(error: &crate::core::ir::ErrorDef) -> String {
    let has_status = error.methods.iter().any(|method| {
        method.name == "status_code"
            && !method.is_static
            && !method.is_async
            && method.params.is_empty()
            && matches!(method.return_type, TypeRef::Primitive(PrimitiveType::U16))
    });
    if has_status {
        "match e.status_code() { 0 => -1, status => i32::from(status) }".to_string()
    } else {
        "-1".to_string()
    }
}

/// The expression that reads `retry_after` of `e` in whole milliseconds, or `-1` when the error
/// has no such method or the delay is absent.
fn jni_error_retry_after_expr(error: &crate::core::ir::ErrorDef) -> String {
    let shape = error
        .methods
        .iter()
        .filter(|method| method.name == "retry_after" && !method.is_static && !method.is_async)
        .filter(|method| method.params.is_empty())
        .find_map(|method| crate::codegen::error_gen::duration_shape(&method.return_type));
    match shape {
        Some(crate::codegen::error_gen::DurationShape::Optional) => {
            "e.retry_after().map_or(-1, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))".to_string()
        }
        Some(crate::codegen::error_gen::DurationShape::Bare) => {
            "i64::try_from(e.retry_after().as_millis()).unwrap_or(i64::MAX)".to_string()
        }
        None => "-1".to_string(),
    }
}

fn jni_error_contexts(api: &ApiSurface, core_import: &str) -> Vec<JniErrorContext> {
    api.errors
        .iter()
        .filter(|error| !error.binding_excluded)
        .map(|error| {
            let rust_path = if error.rust_path.is_empty() {
                format!("{core_import}::{}", error.name)
            } else {
                error.rust_path.replace('-', "_")
            };
            JniErrorContext {
                snake: heck::ToSnakeCase::to_snake_case(error.name.as_str()),
                variants: error
                    .variants
                    .iter()
                    .map(|variant| JniErrorVariantContext {
                        name: variant.name.clone(),
                        pattern: crate::codegen::error_gen::error_variant_wildcard_pattern(&rust_path, variant),
                    })
                    .collect(),
                status_expr: jni_error_status_expr(error),
                retry_after_expr: jni_error_retry_after_expr(error),
                rust_path,
            }
        })
        .collect()
}

fn emit_runtime_helpers(out: &mut String, api: &ApiSurface, core_import: &str) {
    out.push_str(&template_env::render(
        "runtime_helpers.rs.jinja",
        crate::alef_context! {
            errors => jni_error_contexts(api, core_import),
        },
    ));
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
