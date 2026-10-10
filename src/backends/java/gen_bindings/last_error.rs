//! `NativeLib.lastErrorException`: the single place a native error code becomes a typed Java
//! exception carrying the introspection values the FFI layer captured with it.
//!
//! The FFI layer exports one `<prefix>_last_error_<method>` getter per capturable error method
//! (`codegen::error_gen::last_error_fields`). Both the getters declared here and the exception
//! constructors read the same field set, so a method the FFI layer captures cannot go unread and
//! a getter Java declares always exists in the library.

use crate::codegen::error_gen::{
    LastErrorField, LastErrorFieldKind, java_default_value, java_field_name, last_error_fields, typeref_to_java_type,
};
use crate::core::ir::{ApiSurface, ErrorDef, MethodDef, PrimitiveType, TypeRef};

use super::marshal::INFRASTRUCTURE_ERROR_CLASSES;

fn handle_name(prefix: &str, field: &LastErrorField) -> String {
    format!("{}_LAST_ERROR_{}", prefix.to_uppercase(), field.name.to_uppercase())
}

fn symbol_name(prefix: &str, field: &LastErrorField) -> String {
    crate::codegen::c_consumer::last_error_field_symbol(prefix, &field.name)
}

fn layout(kind: &LastErrorFieldKind) -> &'static str {
    match kind {
        LastErrorFieldKind::Scalar(primitive) => match primitive {
            PrimitiveType::Bool => "ValueLayout.JAVA_BOOLEAN",
            PrimitiveType::U8 | PrimitiveType::I8 => "ValueLayout.JAVA_BYTE",
            PrimitiveType::U16 | PrimitiveType::I16 => "ValueLayout.JAVA_SHORT",
            PrimitiveType::U32 | PrimitiveType::I32 => "ValueLayout.JAVA_INT",
            PrimitiveType::F32 => "ValueLayout.JAVA_FLOAT",
            PrimitiveType::F64 => "ValueLayout.JAVA_DOUBLE",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                "ValueLayout.JAVA_LONG"
            }
        },
        LastErrorFieldKind::Text => "ValueLayout.ADDRESS",
        LastErrorFieldKind::DurationMillis => "ValueLayout.JAVA_LONG",
    }
}

pub(crate) fn field_symbols(api: &ApiSurface, prefix: &str) -> Vec<String> {
    last_error_fields(&api.errors)
        .iter()
        .map(|field| symbol_name(prefix, field))
        .collect()
}

pub(crate) fn field_handles(api: &ApiSurface, prefix: &str) -> Vec<String> {
    last_error_fields(&api.errors)
        .iter()
        .map(|field| {
            format!(
                "    static final MethodHandle {} = LINKER.downcallHandle(\n        LIB.find(\"{}\").orElse(null),\n        FunctionDescriptor.of({})\n    );\n",
                handle_name(prefix, field),
                symbol_name(prefix, field),
                layout(&field.kind),
            )
        })
        .collect()
}

/// The first declaration of `name` among the errors' capturable methods; the FFI layer keeps the
/// first declaration too, so this is the type its getter returns.
fn declared_method<'a>(errors: &'a [ErrorDef], name: &str) -> Option<&'a MethodDef> {
    errors
        .iter()
        .flat_map(|error| &error.methods)
        .find(|method| method.name == name)
}

fn read_statements(field: &LastErrorField, handle: &str, local: &str, return_type: &TypeRef) -> Vec<String> {
    match &field.kind {
        LastErrorFieldKind::Scalar(primitive) => {
            let expression = match primitive {
                PrimitiveType::Bool => format!("(boolean) NativeLib.{handle}.invoke()"),
                PrimitiveType::U8 => format!("Byte.toUnsignedInt((byte) NativeLib.{handle}.invoke())"),
                PrimitiveType::I8 => format!("(byte) NativeLib.{handle}.invoke()"),
                PrimitiveType::U16 => format!("Short.toUnsignedInt((short) NativeLib.{handle}.invoke())"),
                PrimitiveType::I16 => format!("(short) NativeLib.{handle}.invoke()"),
                PrimitiveType::U32 | PrimitiveType::I32 => format!("(int) NativeLib.{handle}.invoke()"),
                PrimitiveType::F32 => format!("(float) NativeLib.{handle}.invoke()"),
                PrimitiveType::F64 => format!("(double) NativeLib.{handle}.invoke()"),
                PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                    format!("(long) NativeLib.{handle}.invoke()")
                }
            };
            vec![format!("{local} = {expression};")]
        }
        LastErrorFieldKind::Text => vec![
            format!("MemorySegment {local}Ptr = (MemorySegment) NativeLib.{handle}.invoke();"),
            format!(
                "{local} = {local}Ptr.equals(MemorySegment.NULL) ? \"\" : {local}Ptr.reinterpret(Long.MAX_VALUE).getString(0);"
            ),
        ],
        LastErrorFieldKind::DurationMillis => {
            let optional = matches!(return_type, TypeRef::Optional(_));
            let value = if optional {
                format!("{local}Millis < 0 ? null : java.time.Duration.ofMillis({local}Millis)")
            } else {
                format!("java.time.Duration.ofMillis(Math.max({local}Millis, 0L))")
            };
            vec![
                format!("long {local}Millis = (long) NativeLib.{handle}.invoke();"),
                format!("{local} = {value};"),
            ]
        }
    }
}

#[derive(serde::Serialize)]
struct FieldContext {
    local: String,
    java_type: String,
    default_value: String,
    read: Vec<String>,
}

#[derive(serde::Serialize)]
struct CaseContext {
    code: u32,
    class_name: String,
    args: String,
}

fn local_name(field_name: &str) -> String {
    format!("{}Native", java_field_name(field_name))
}

/// Constructor arguments (after the message) for a variant of `error`: every introspection
/// method in declaration order, from the captured local when this method's getter exists with the
/// same Java type, otherwise the type's default literal.
fn variant_args(error: &ErrorDef, captured: &[(String, String)]) -> String {
    let mut args = String::new();
    for method in &error.methods {
        let java_type = typeref_to_java_type(&method.return_type);
        let local = local_name(&method.name);
        if captured
            .iter()
            .any(|(name, ty)| *name == method.name && ty == java_type)
        {
            args.push_str(&format!(", {local}"));
        } else {
            args.push_str(&format!(", {}", java_default_value(&method.return_type)));
        }
    }
    args
}

pub(crate) fn gen_last_error_exception(api: &ApiSurface, prefix: &str, main_class: &str) -> String {
    let mut captured: Vec<(String, String)> = Vec::new();
    let mut fields_ctx: Vec<FieldContext> = Vec::new();
    for field in last_error_fields(&api.errors) {
        let Some(method) = declared_method(&api.errors, &field.name) else {
            continue;
        };
        let java_type = typeref_to_java_type(&method.return_type);
        let local = local_name(&field.name);
        captured.push((field.name.clone(), java_type.to_string()));
        fields_ctx.push(FieldContext {
            read: read_statements(&field, &handle_name(prefix, &field), &local, &method.return_type),
            local,
            java_type: java_type.to_string(),
            default_value: java_default_value(&method.return_type).to_string(),
        });
    }

    let infrastructure_names: Vec<&str> = INFRASTRUCTURE_ERROR_CLASSES.iter().map(|(name, _, _)| *name).collect();
    let mut cases: Vec<CaseContext> = INFRASTRUCTURE_ERROR_CLASSES
        .iter()
        .map(|(name, code, _)| CaseContext {
            code: *code,
            class_name: (*name).to_string(),
            args: String::new(),
        })
        .collect();
    for entry in api.error_taxonomy() {
        let class_name = format!("{}Exception", entry.variant);
        let args = if infrastructure_names.contains(&class_name.as_str()) {
            String::new()
        } else {
            api.errors
                .iter()
                .find(|error| error.rust_path == entry.error_type)
                .map(|error| variant_args(error, &captured))
                .unwrap_or_default()
        };
        cases.push(CaseContext {
            code: entry.code,
            class_name,
            args,
        });
    }

    crate::backends::java::template_env::render(
        "helper_last_error_exception.jinja",
        crate::alef_context! {
            exception_class => format!("{main_class}Exception"),
            fields => fields_ctx,
            cases => cases,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ErrorVariant, ReceiverKind};

    fn method(name: &str, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            return_type,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }
    }

    fn api() -> ApiSurface {
        let error = ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample::RequestError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                error_code: Some(100),
                name: "RateLimited".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            doc: String::new(),
            methods: vec![
                method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
                method("is_transient", TypeRef::Primitive(PrimitiveType::Bool)),
                method("error_type", TypeRef::String),
                method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        ApiSurface {
            errors: vec![error],
            ..Default::default()
        }
    }

    #[test]
    fn declares_one_getter_handle_per_capturable_method() {
        let handles = field_handles(&api(), "sample").join("\n");
        assert!(
            handles.contains("static final MethodHandle SAMPLE_LAST_ERROR_STATUS_CODE = LINKER.downcallHandle("),
            "{handles}"
        );
        assert!(
            handles.contains("LIB.find(\"sample_last_error_status_code\")"),
            "{handles}"
        );
        assert!(
            handles.contains("FunctionDescriptor.of(ValueLayout.JAVA_SHORT)"),
            "{handles}"
        );
        assert!(
            handles.contains("FunctionDescriptor.of(ValueLayout.JAVA_BOOLEAN)"),
            "{handles}"
        );
        assert!(
            handles.contains("FunctionDescriptor.of(ValueLayout.ADDRESS)"),
            "{handles}"
        );
        assert_eq!(
            handles.matches("FunctionDescriptor.of(ValueLayout.JAVA_LONG)").count(),
            1,
            "{handles}"
        );
        assert_eq!(field_symbols(&api(), "sample").len(), 4);
    }

    /// Regression: the typed exception used to be constructed with only the message, so every
    /// introspection getter returned its default whatever the native layer captured. ~keep
    #[test]
    fn typed_variants_receive_the_captured_native_fields() {
        let method = gen_last_error_exception(&api(), "sample", "Sample");
        assert!(
            method.contains(
                "case 100 -> new RateLimitedException(msg, statusCodeNative, isTransientFlagNative, errorTypeNative, retryAfterNative);"
            ),
            "{method}"
        );
        assert!(
            method.contains(
                "statusCodeNative = Short.toUnsignedInt((short) NativeLib.SAMPLE_LAST_ERROR_STATUS_CODE.invoke());"
            ),
            "{method}"
        );
        assert!(method.contains("retryAfterNative = retryAfterNativeMillis < 0 ? null : java.time.Duration.ofMillis(retryAfterNativeMillis);"), "{method}");
        assert!(
            method.contains("case 5 -> new OperationCancelledException(msg);"),
            "{method}"
        );
        assert!(
            method.contains("default -> new SampleException(errCode, msg);"),
            "{method}"
        );
    }

    #[test]
    fn without_error_methods_variants_keep_the_message_only_constructor() {
        let mut surface = api();
        surface.errors[0].methods.clear();
        let method = gen_last_error_exception(&surface, "sample", "Sample");
        assert!(
            method.contains("case 100 -> new RateLimitedException(msg);"),
            "{method}"
        );
        assert!(!method.contains("try {"), "no native reads without fields: {method}");
        assert!(field_handles(&surface, "sample").is_empty());
    }
}
