use crate::core::ir::ErrorDef;

use super::shared::{error_base_prefix, variant_display_message};

pub fn gen_go_error_types(error: &ErrorDef, pkg_name: &str) -> String {
    let sentinels = gen_go_sentinel_errors(std::slice::from_ref(error));
    let structured = gen_go_error_struct(error, pkg_name, false);
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
    let mut sentinels = Vec::new();
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
            sentinels.push((const_name, msg));
        }
    }

    crate::codegen::template_env::render(
        "error_gen/go_sentinel_errors.jinja",
        minijinja::context! {
            sentinels => sentinels,
        },
    )
}

pub fn go_error_sentinel_name(errors: &[ErrorDef], error_name: &str, variant_name: &str) -> String {
    let collides = errors
        .iter()
        .flat_map(|error| &error.variants)
        .filter(|variant| variant.name == variant_name)
        .count()
        > 1;
    if collides {
        format!("Err{}{}", error_base_prefix(error_name), variant_name)
    } else {
        format!("Err{variant_name}")
    }
}

/// Generate the structured error type (struct + Error() method) for a single
/// error definition. Sentinel errors are emitted separately by
/// [`gen_go_sentinel_errors`].
///
/// When `error.methods` is non-empty, each whitelisted introspection method
/// produces an exported struct field of the matching Go type.
///
/// `carries_sentinel` adds an unexported sentinel and an `Unwrap` method so
/// `errors.Is(err, ErrX)` matches. Only the error type `lastError()` constructs sets
/// it, so the field is never dead.
pub fn gen_go_error_struct(error: &ErrorDef, pkg_name: &str, carries_sentinel: bool) -> String {
    let go_type_name = crate::codegen::naming::go_error_type_name(&error.name, pkg_name);

    let methods: Vec<serde_json::Value> = go_error_method_fields(error)
        .into_iter()
        .map(|field| {
            serde_json::json!({
                "field_name": field.field_name,
                "go_type": field.go_type,
            })
        })
        .collect();
    let has_methods = !methods.is_empty();

    crate::codegen::template_env::render(
        "error_gen/go_error_struct.jinja",
        minijinja::context! {
            go_type_name => go_type_name.as_str(),
            methods => methods,
            has_methods => has_methods,
            carries_sentinel => carries_sentinel,
        },
    )
}

/// One exported Go struct field backed by a whitelisted error introspection method.
pub struct GoErrorMethodField {
    pub field_name: String,
    pub go_type: &'static str,
    pub method: String,
}

/// The struct fields `gen_go_error_struct` emits for `error.methods`, in declaration order.
///
/// Shared with the Go `lastError()` helper so the fields it populates are exactly the fields
/// the struct declares.
pub fn go_error_method_fields(error: &ErrorDef) -> Vec<GoErrorMethodField> {
    error
        .methods
        .iter()
        .map(|m| GoErrorMethodField {
            field_name: to_pascal_case(&m.name),
            go_type: typeref_to_go_type(&m.return_type),
            method: m.name.clone(),
        })
        .collect()
}

/// Map an IR `TypeRef` to a Go type string for error introspection method returns.
/// Only the primitive subset needed for the whitelisted methods is handled;
/// everything else falls back to `string`.
fn typeref_to_go_type(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "bool",
        TypeRef::Primitive(PrimitiveType::U8) => "uint8",
        TypeRef::Primitive(PrimitiveType::U16) => "uint16",
        TypeRef::Primitive(PrimitiveType::U32) => "uint32",
        TypeRef::Primitive(PrimitiveType::U64) => "uint64",
        TypeRef::Primitive(PrimitiveType::I8) => "int8",
        TypeRef::Primitive(PrimitiveType::I16) => "int16",
        TypeRef::Primitive(PrimitiveType::I32) => "int32",
        TypeRef::Primitive(PrimitiveType::I64) => "int64",
        TypeRef::Primitive(PrimitiveType::F32) => "float32",
        TypeRef::Primitive(PrimitiveType::F64) => "float64",
        TypeRef::String => "string",
        TypeRef::Duration => "DurationMillis",
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration) => "*DurationMillis",
        _ => "string",
    }
}

/// Convert a snake_case or camelCase name to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

/// Generate Java exception sub-classes for each error variant.
///
/// Returns a `Vec` of `(class_name, file_content)` tuples: the base exception
/// class followed by one per-variant exception.  The caller writes each to a
/// separate `.java` file.
///
/// `main_class` is the generated FFI class name (e.g. `Sample` for `SampleException`). The base
/// exception extends `{main_class}Exception` rather than bare `Exception` so that every generated
/// method's trailing `catch ({main_class}Exception e) { throw e; }` guard -- see
/// `ffi_class::error_catch::emit_method_catch_chain` -- actually matches a typed error the enum
/// throws, instead of falling through to the `catch (Throwable e)` clause that replaces it with
/// the placeholder "FFI call failed" message and demotes the real detail to a nested cause. ~keep
///
/// When `error.methods` is non-empty, the base exception class gains private
/// final fields, an extended constructor, and public getter methods for each
/// whitelisted introspection method.  Variant classes delegate via `super(…)`.
pub fn gen_java_error_types(error: &ErrorDef, package: &str, main_class: &str) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(error.variants.len() + 1);

    let base_name = format!("{}Exception", error.name);
    let doc_lines: Vec<&str> = error.doc.lines().collect();

    let method_infos: Vec<serde_json::Value> = error
        .methods
        .iter()
        .map(|m| {
            let java_type = typeref_to_java_type(&m.return_type);
            let getter_name = java_getter_name(&m.name);
            let field_name = java_field_name(&m.name);
            let default_value = java_default_value(&m.return_type);
            serde_json::json!({
                "field_name": field_name,
                "java_type": java_type,
                "getter_name": getter_name,
                "default_value": default_value,
                "nullable": duration_shape(&m.return_type) == Some(DurationShape::Optional),
                "is_duration": duration_shape(&m.return_type).is_some(),
                "doc": m.doc,
            })
        })
        .collect();
    let has_methods = !method_infos.is_empty();
    let (legacy_ctor_methods, has_duration_methods) = split_duration_methods(&method_infos);

    let base = crate::codegen::template_env::render(
        "error_gen/java_error_base.jinja",
        minijinja::context! {
            package => package,
            base_name => base_name.as_str(),
            main_class => main_class,
            doc => !error.doc.is_empty(),
            doc_lines => doc_lines,
            methods => method_infos,
            has_methods => has_methods,
            legacy_ctor_methods => legacy_ctor_methods,
            has_duration_methods => has_duration_methods,
        },
    );
    files.push((base_name.clone(), base));

    for variant in &error.variants {
        let class_name = format!("{}Exception", variant.name);
        let doc_lines: Vec<&str> = variant.doc.lines().collect();

        let content = crate::codegen::template_env::render(
            "error_gen/java_error_variant.jinja",
            minijinja::context! {
                package => package,
                class_name => class_name.as_str(),
                base_name => base_name.as_str(),
                doc => !variant.doc.is_empty(),
                doc_lines => doc_lines,
                has_methods => has_methods,
            },
        );
        files.push((class_name, content));
    }

    files
}

/// Map an IR `TypeRef` to a Java type string for error introspection getters.
fn typeref_to_java_type(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    if duration_shape(ty).is_some() {
        return "java.time.Duration";
    }
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "boolean",
        TypeRef::Primitive(
            PrimitiveType::U8
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::U16
            | PrimitiveType::I32
            | PrimitiveType::U32,
        ) => "int",
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize) => {
            "long"
        }
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::String => "String",
        _ => "String",
    }
}

/// Convert a snake_case method name to a Java getter name.
/// E.g. `status_code` → `getStatusCode`, `is_transient` → `isTransient`.
fn java_getter_name(snake: &str) -> String {
    if let Some(rest) = snake.strip_prefix("is_") {
        let pascal = to_pascal_case(rest);
        format!("is{pascal}")
    } else {
        let pascal = to_pascal_case(snake);
        format!("get{pascal}")
    }
}

/// Convert a snake_case method name to a Java field name (camelCase).
/// E.g. `status_code` → `statusCode`, `is_transient` → `isTransientFlag`.
/// Fields that conflict with Serializable interface methods get a suffix.
fn java_field_name(snake: &str) -> String {
    let parts: Vec<&str> = snake.split('_').collect();
    if parts.is_empty() {
        return snake.to_string();
    }
    let mut out = parts[0].to_string();
    for part in &parts[1..] {
        let mut chars = part.chars();
        match chars.next() {
            None => {}
            Some(first) => {
                out.push_str(&first.to_uppercase().to_string());
                out.push_str(chars.as_str());
            }
        }
    }

    if out == "isTransient" {
        out.push_str("Flag");
    }

    out
}

/// Return the Java zero-value literal for a type (used in the no-args default constructor).
fn java_default_value(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match duration_shape(ty) {
        Some(DurationShape::Optional) => return "null",
        Some(DurationShape::Bare) => return "java.time.Duration.ZERO",
        None => {}
    }
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(_) => "0",
        _ => "\"\"",
    }
}

/// Generate C# exception sub-classes for each error variant.
///
/// Returns a `Vec` of `(class_name, file_content)` tuples: the base exception
/// class followed by one per-variant exception.  The caller writes each to a
/// separate `.cs` file.
///
/// `fallback_class` is the name of the generic library exception class (e.g.
/// `SampleLanguagePackException`) that the base error class should extend so that
/// callers can `catch` the general library exception and catch all typed errors.
///
/// When `error.methods` is non-empty, the base exception class gains get-only
/// properties for each whitelisted introspection method.  Variant classes
/// delegate via `base(…)` and inherit the properties.
pub fn gen_csharp_error_types(
    error: &ErrorDef,
    namespace: &str,
    fallback_class: Option<&str>,
) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(error.variants.len() + 1);

    let base_name = format!("{}Exception", error.name);
    let base_parent = fallback_class.unwrap_or("Exception");
    let sanitized_error_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
        &error.doc,
        crate::codegen::doc_emission::DocTarget::CSharpDoc,
    );
    let error_doc_lines: Vec<&str> = sanitized_error_doc.lines().collect();
    let error_has_doc = !sanitized_error_doc.trim().is_empty();

    let method_infos: Vec<serde_json::Value> = error
        .methods
        .iter()
        .map(|m| {
            let cs_type = typeref_to_csharp_type(&m.return_type);
            let prop_name = to_pascal_case(&m.name);
            let param_name = java_field_name(&m.name);
            let default_value = csharp_default_value(&m.return_type);
            let sanitized_method_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
                &m.doc,
                crate::codegen::doc_emission::DocTarget::CSharpDoc,
            );
            let inline_doc = sanitized_method_doc
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            serde_json::json!({
                "prop_name": prop_name,
                "cs_type": cs_type,
                "param_name": param_name,
                "default_value": default_value,
                "is_duration": duration_shape(&m.return_type).is_some(),
                "doc": inline_doc,
            })
        })
        .collect();
    let has_methods = !method_infos.is_empty();
    let (legacy_ctor_methods, has_duration_methods) = split_duration_methods(&method_infos);

    {
        let out = crate::codegen::template_env::render(
            "error_gen/csharp_error_base.jinja",
            minijinja::context! {
                namespace => namespace,
                base_name => base_name.as_str(),
                base_parent => base_parent,
                doc => error_has_doc,
                doc_lines => error_doc_lines,
                methods => method_infos,
                has_methods => has_methods,
                legacy_ctor_methods => legacy_ctor_methods,
                has_duration_methods => has_duration_methods,
            },
        );
        files.push((base_name.clone(), out));
    }

    for variant in &error.variants {
        let class_name = format!("{}Exception", variant.name);
        let sanitized_variant_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
            &variant.doc,
            crate::codegen::doc_emission::DocTarget::CSharpDoc,
        );
        let variant_doc_lines: Vec<&str> = sanitized_variant_doc.lines().collect();
        let variant_has_doc = !sanitized_variant_doc.trim().is_empty();

        let out = crate::codegen::template_env::render(
            "error_gen/csharp_error_variant.jinja",
            minijinja::context! {
                namespace => namespace,
                class_name => class_name.as_str(),
                base_name => base_name.as_str(),
                doc => variant_has_doc,
                doc_lines => variant_doc_lines,
                has_methods => has_methods,
            },
        );
        files.push((class_name, out));
    }

    files
}

/// Map an IR `TypeRef` to a C# type string for error introspection properties.
fn typeref_to_csharp_type(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match duration_shape(ty) {
        Some(DurationShape::Optional) => return "TimeSpan?",
        Some(DurationShape::Bare) => return "TimeSpan",
        None => {}
    }
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "bool",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::Isize) => "long",
        TypeRef::Primitive(PrimitiveType::U64 | PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::String => "string",
        _ => "string",
    }
}

/// Return the C# zero-value literal for a type (used in the default constructor).
fn csharp_default_value(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match duration_shape(ty) {
        Some(DurationShape::Optional) => return "null",
        Some(DurationShape::Bare) => return "TimeSpan.Zero",
        None => {}
    }
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(_) => "0",
        _ => "string.Empty",
    }
}

/// Whether an error introspection method returns a `Duration`, and if so whether it is optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurationShape {
    Bare,
    Optional,
}

fn duration_shape(ty: &crate::core::ir::TypeRef) -> Option<DurationShape> {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Duration => Some(DurationShape::Bare),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration) => Some(DurationShape::Optional),
        _ => None,
    }
}

/// The methods the pre-`Duration` constructor overload keeps, plus whether any `Duration` method
/// exists at all.
///
/// Java and C# overload their base exception constructor with one parameter per introspection
/// method. A `Duration`-valued method such as `retry_after` was added after that signature
/// shipped, so the original overload keeps taking only the non-`Duration` methods and a second
/// overload takes them all; changing the existing signature would break every caller that
/// constructs the exception directly. ~keep
fn split_duration_methods(methods: &[serde_json::Value]) -> (Vec<serde_json::Value>, bool) {
    let legacy: Vec<serde_json::Value> = methods
        .iter()
        .filter(|m| m["is_duration"] != serde_json::Value::Bool(true))
        .cloned()
        .collect();
    let has_duration = legacy.len() != methods.len();
    (legacy, has_duration)
}
