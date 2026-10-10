use crate::codegen::error_gen::{LastErrorField, LastErrorFieldKind, last_error_field_kind, last_error_fields};
use crate::core::ir::{ApiSurface, ErrorDef, ErrorVariant, MethodDef, PrimitiveType, TypeRef};

#[derive(serde::Serialize)]
struct FfiErrorVariantCode {
    pattern: String,
    name: String,
    code_expression: String,
}

#[derive(serde::Serialize)]
struct FfiErrorFieldCapture {
    upper: String,
    expression: String,
}

#[derive(serde::Serialize)]
struct FfiErrorCodeImpl {
    error_path: String,
    variants: Vec<FfiErrorVariantCode>,
    fields: Vec<FfiErrorFieldCapture>,
}

#[derive(serde::Serialize)]
struct FfiLastErrorField {
    name: String,
    upper: String,
    accessor: String,
    kind: &'static str,
    storage_type: String,
    c_type: String,
    reset_expression: String,
    fallback_expression: String,
}

fn ffi_last_error_field(prefix: &str, field: &LastErrorField) -> FfiLastErrorField {
    let accessor = crate::codegen::c_consumer::last_error_field_symbol(prefix, &field.name);
    let (kind, storage_type, c_type, reset_expression, fallback_expression) = match &field.kind {
        LastErrorFieldKind::Scalar(primitive) => {
            let ty = primitive.rust_source_display();
            let zero = match primitive {
                PrimitiveType::Bool => "false",
                PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
                _ => "0",
            };
            ("scalar", ty, ty, zero, zero)
        }
        LastErrorFieldKind::Text => ("text", "Option<CString>", "*const c_char", "None", "std::ptr::null()"),
        LastErrorFieldKind::DurationMillis => ("duration", "i64", "i64", "-1", "-1"),
    };
    FfiLastErrorField {
        name: field.name.clone(),
        upper: field.name.to_uppercase(),
        accessor,
        kind,
        storage_type: storage_type.to_string(),
        c_type: c_type.to_string(),
        reset_expression: reset_expression.to_string(),
        fallback_expression: fallback_expression.to_string(),
    }
}

fn ffi_error_field_capture(method: &MethodDef, kind: &LastErrorFieldKind) -> FfiErrorFieldCapture {
    let call = format!("error.{}()", method.name);
    let expression = match kind {
        LastErrorFieldKind::Scalar(_) => call,
        LastErrorFieldKind::Text => format!("CString::new({call}.to_string()).ok()"),
        LastErrorFieldKind::DurationMillis if matches!(method.return_type, TypeRef::Optional(_)) => {
            format!("{call}.map_or(-1, alef_ffi_duration_millis)")
        }
        LastErrorFieldKind::DurationMillis => format!("alef_ffi_duration_millis({call})"),
    };
    FfiErrorFieldCapture {
        upper: method.name.to_uppercase(),
        expression,
    }
}

fn ffi_error_variant_code(error: &ErrorDef, error_path: &str, variant: &ErrorVariant) -> FfiErrorVariantCode {
    let suffix = if variant.is_unit {
        String::new()
    } else if variant.is_tuple {
        "(..)".to_string()
    } else {
        " { .. }".to_string()
    };
    FfiErrorVariantCode {
        pattern: format!("{error_path}::{}{suffix}", variant.name),
        name: variant.name.clone(),
        code_expression: variant.error_code.map_or_else(
            || "ALEF_FFI_UNKNOWN_ERROR".to_string(),
            |_| {
                let variant_name = crate::codegen::naming::ffi_error_code_variant_name(&error.rust_path, &variant.name);
                format!("AlefFfiErrorCode::{variant_name} as i32")
            },
        ),
    }
}

fn ffi_error_code_impl(error: &ErrorDef, core_import: &str, last_error_fields: &[LastErrorField]) -> FfiErrorCodeImpl {
    let error_path = if error.rust_path.contains("::") {
        error.rust_path.replace('-', "_")
    } else {
        format!("{core_import}::{}", error.name)
    };
    let variants = error
        .variants
        .iter()
        .map(|variant| ffi_error_variant_code(error, &error_path, variant))
        .collect();
    let fields = error
        .methods
        .iter()
        .filter_map(|method| {
            let kind = last_error_field_kind(method)?;
            let declared = last_error_fields.iter().find(|field| field.name == method.name)?;
            (declared.kind == kind).then(|| ffi_error_field_capture(method, &kind))
        })
        .collect();
    FfiErrorCodeImpl {
        error_path,
        variants,
        fields,
    }
}

pub(super) fn gen_last_error(api: &ApiSurface, prefix: &str, core_import: &str) -> String {
    let taxonomy = api.error_taxonomy();
    let last_error_fields = last_error_fields(&api.errors);
    let error_code_impls: Vec<_> = api
        .errors
        .iter()
        .map(|error| ffi_error_code_impl(error, core_import, &last_error_fields))
        .collect();
    let has_error_code_impls = !error_code_impls.is_empty();
    let has_duration_fields = last_error_fields
        .iter()
        .any(|field| field.kind == LastErrorFieldKind::DurationMillis);
    let error_fields: Vec<FfiLastErrorField> = last_error_fields
        .iter()
        .map(|field| ffi_last_error_field(prefix, field))
        .collect();
    crate::backends::ffi::template_env::render(
        "last_error.jinja",
        crate::alef_context! {
            prefix => prefix,
            builtin_prefix => crate::codegen::naming::ffi_builtin_error_code_prefix(prefix),
            error_code_impls => error_code_impls,
            has_error_code_impls => has_error_code_impls,
            error_fields => error_fields,
            has_duration_fields => has_duration_fields,
            taxonomy => taxonomy.iter().map(|entry| crate::alef_context! {
                code => entry.code,
                enum_variant => crate::codegen::naming::ffi_error_code_variant_name(&entry.error_type, &entry.variant),
            }).collect::<Vec<_>>(),
            no_error_code => ApiSurface::FFI_ERROR_CODE_NONE,
            conversion_error_code => ApiSurface::FFI_ERROR_CODE_CONVERSION,
            unknown_error_code => ApiSurface::FFI_ERROR_CODE_UNKNOWN,
            panic_error_code => ApiSurface::FFI_ERROR_CODE_PANIC,
            invalid_handle_error_code => ApiSurface::FFI_ERROR_CODE_INVALID_HANDLE,
            cancelled_error_code => ApiSurface::FFI_ERROR_CODE_CANCELLED,
        },
    )
}
