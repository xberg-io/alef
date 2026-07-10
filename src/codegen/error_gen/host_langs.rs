use crate::core::ir::ErrorDef;

use super::shared::{error_base_prefix, variant_display_message};

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

/// Generate the structured error type (struct + Error() method) for a single
/// error definition. Sentinel errors are emitted separately by
/// [`gen_go_sentinel_errors`].
///
/// When `error.methods` is non-empty, each whitelisted introspection method
/// produces an exported struct field of the matching Go type plus a receiver
/// method that returns that field.
pub fn gen_go_error_struct(error: &ErrorDef, pkg_name: &str) -> String {
    let go_type_name = strip_package_prefix(&error.name, pkg_name);

    let methods: Vec<serde_json::Value> = error
        .methods
        .iter()
        .map(|m| {
            let go_type = typeref_to_go_type(&m.return_type);
            let method_name = to_pascal_case(&m.name);
            let doc_summary = if m.doc.is_empty() {
                String::new()
            } else {
                let first = crate::codegen::doc_emission::doc_first_paragraph_joined(&m.doc);
                first.trim_end_matches('.').trim_end().to_string()
            };
            serde_json::json!({
                "field_name": method_name,
                "go_type": go_type,
                "method_name": method_name,
                "doc": doc_summary,
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
        },
    )
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

/// Strip the package-name prefix from a type name to avoid revive's stutter lint.
///
/// Revive reports `exported: type name will be used as pkg.PkgFoo by other packages,
/// and that stutters` when a type name begins with the package name. This function
/// removes the prefix when it matches (case-insensitively) so that the exported name
/// does not repeat the package name.
///
/// Examples:
/// - `("SampleLlmError", "samplellm")` → `"Error"` (lowercased `samplellm` is a prefix
///   of lowercased `samplellmerror`)
/// - `("ConversionError", "converter")` → `"ConversionError"` (no match)
fn strip_package_prefix(type_name: &str, pkg_name: &str) -> String {
    let type_lower = type_name.to_lowercase();
    let pkg_lower = pkg_name.to_lowercase();
    if type_lower.starts_with(&pkg_lower) && type_lower.len() > pkg_lower.len() {
        type_name[pkg_lower.len()..].to_string()
    } else {
        type_name.to_string()
    }
}

/// Generate Java exception sub-classes for each error variant.
///
/// Returns a `Vec` of `(class_name, file_content)` tuples: the base exception
/// class followed by one per-variant exception.  The caller writes each to a
/// separate `.java` file.
///
/// When `error.methods` is non-empty, the base exception class gains private
/// final fields, an extended constructor, and public getter methods for each
/// whitelisted introspection method.  Variant classes delegate via `super(…)`.
pub fn gen_java_error_types(error: &ErrorDef, package: &str) -> Vec<(String, String)> {
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
                "doc": m.doc,
            })
        })
        .collect();
    let has_methods = !method_infos.is_empty();

    let base = crate::codegen::template_env::render(
        "error_gen/java_error_base.jinja",
        minijinja::context! {
            package => package,
            base_name => base_name.as_str(),
            doc => !error.doc.is_empty(),
            doc_lines => doc_lines,
            methods => method_infos,
            has_methods => has_methods,
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
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64) => "long",
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
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::String => "\"\"",
        _ => "0",
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
                "doc": inline_doc,
            })
        })
        .collect();
    let has_methods = !method_infos.is_empty();

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
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "bool",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::String => "string",
        _ => "string",
    }
}

/// Return the C# zero-value literal for a type (used in the default constructor).
fn csharp_default_value(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::String => "string.Empty",
        _ => "0",
    }
}
