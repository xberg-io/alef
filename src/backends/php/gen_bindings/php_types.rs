use crate::core::ir::{PrimitiveType, TypeRef};
use minijinja::context;

pub(super) fn php_phpdoc_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Vec(inner) => format!("array<{}>", php_phpdoc_type(inner)),
        TypeRef::Map(k, v) => format!("array<{}, {}>", php_phpdoc_type(k), php_phpdoc_type(v)),
        TypeRef::Optional(inner) => {
            // Flatten nested Option<Option<T>> to a single nullable type.
            // php_type() already handles nested Optional by returning a string starting with '?',
            // so we check and avoid double-prepending.
            let inner_type = php_phpdoc_type(inner);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        _ => php_type(ty),
    }
}

/// Map an IR [`TypeRef`] to a fully-qualified PHPDoc type string with generics (e.g., `array<\Ns\T>`).
pub(super) fn php_phpdoc_type_fq(ty: &TypeRef, namespace: &str) -> String {
    match ty {
        TypeRef::Vec(inner) => format!("array<{}>", php_phpdoc_type_fq(inner, namespace)),
        TypeRef::Map(k, v) => format!(
            "array<{}, {}>",
            php_phpdoc_type_fq(k, namespace),
            php_phpdoc_type_fq(v, namespace)
        ),
        TypeRef::Named(name) => format!("\\{}\\{}", namespace, name),
        TypeRef::Optional(inner) => format!("?{}", php_phpdoc_type_fq(inner, namespace)),
        _ => php_type(ty),
    }
}

/// Map an IR [`TypeRef`] to a fully-qualified PHP type-hint string for use outside the namespace.
pub(super) fn php_type_fq(ty: &TypeRef, namespace: &str) -> String {
    match ty {
        TypeRef::Named(name) => format!("\\{}\\{}", namespace, name),
        TypeRef::Optional(inner) => {
            let inner_type = php_type_fq(inner, namespace);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        _ => php_type(ty),
    }
}

pub(super) fn php_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Json | TypeRef::Bytes | TypeRef::Path => "string".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float".to_string(),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => "int".to_string(),
        },
        TypeRef::Optional(inner) => {
            // Flatten nested Option<Option<T>> to a single nullable type.
            // PHP has no double-nullable concept; ?T already covers null.
            let inner_type = php_type(inner);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => "array".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Duration => "float".to_string(),
    }
}

/// Build an inline PHPDoc block for a class property or constructor-promoted parameter.
///
/// - When `doc` is non-empty and multi-line, emits a multi-line block with description lines
///   followed by an `@var` tag.
/// - When `doc` is non-empty and single-line, emits a compact `/** @var T Description. */` form.
/// - When `doc` is empty, emits the type-only compact form `/** @var T */`.
///
/// `indent` is prepended to every line of the output (typically 4 or 8 spaces).
pub(super) fn php_property_phpdoc(var_type: &str, doc: &str, indent: &str) -> String {
    let doc = doc.trim();
    if doc.is_empty() {
        return crate::backends::php::template_env::render(
            "php_inline_property_phpdoc.jinja",
            context! {
                indent => indent,
                var_type => var_type,
                doc => "",
            },
        );
    }
    let lines: Vec<&str> = doc.lines().collect();
    if lines.len() == 1 {
        let line = lines[0].trim();
        return crate::backends::php::template_env::render(
            "php_inline_property_phpdoc.jinja",
            context! {
                indent => indent,
                var_type => var_type,
                doc => line,
            },
        );
    }
    // Multi-line: description block + @var tag.
    let mut out = format!("{indent}/**\n");
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_indented_phpdoc_empty_line.jinja",
                context! { indent => indent },
            ));
        } else {
            out.push_str(&crate::backends::php::template_env::render(
                "php_prefixed_phpdoc_line.jinja",
                context! {
                    indent => indent,
                    line => trimmed,
                },
            ));
        }
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_empty_line.jinja",
        context! { indent => indent },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_prefixed_phpdoc_line.jinja",
        context! {
            indent => indent,
            line => &format!("@var {var_type}"),
        },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_block_end.jinja",
        context! { indent => indent },
    ));
    out
}
