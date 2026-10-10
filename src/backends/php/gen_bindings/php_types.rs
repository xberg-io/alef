use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;

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
        // Duration crosses the FFI boundary as milliseconds (Rust `i64`, see `PhpMapper::duration`
        // in `type_map.rs`), not seconds — the stub type is `int`, not `float`. ~keep
        TypeRef::Duration => "int".to_string(),
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
            crate::alef_context! {
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
            crate::alef_context! {
                indent => indent,
                var_type => var_type,
                doc => line,
            },
        );
    }
    let mut out = format!("{indent}/**\n");
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_indented_phpdoc_empty_line.jinja",
                crate::alef_context! { indent => indent },
            ));
        } else {
            out.push_str(&crate::backends::php::template_env::render(
                "php_prefixed_phpdoc_line.jinja",
                crate::alef_context! {
                    indent => indent,
                    line => trimmed,
                },
            ));
        }
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_empty_line.jinja",
        crate::alef_context! { indent => indent },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_prefixed_phpdoc_line.jinja",
        crate::alef_context! {
            indent => indent,
            line => &format!("@var {var_type}"),
        },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_block_end.jinja",
        crate::alef_context! { indent => indent },
    ));
    out
}

/// `enum_names`-aware counterpart of [`php_type`]: `PhpMapper::named` (`type_map.rs`) lowers a
/// unit-variant enum to `String` (ext-php-rs cannot carry a Rust enum), so any field, property,
/// method/function param, or return of that enum's type crosses the FFI boundary as a plain PHP
/// `string`, not as an instance of the constants-only class `gen_enum_constants` declares for the
/// enum. Typing it as the enum's own class name would promise a value the extension never
/// produces — for the PHPStan stub that's a false type declaration; for the generated `src/`
/// facade and opaque-class files it is worse: a facade class passes the argument straight through
/// to the native `...Api` class, so a bare enum-class type hint makes the method genuinely
/// uncallable (the constants-only class has no instances a caller could ever pass).
/// Shared by every PHP codegen site that types a value against an IR type — stub properties,
/// struct constructor params, stub method params/returns, the runtime facade
/// (`public_api.rs`), and opaque-class method stubs (`opaque_files.rs`) — so none of those sites
/// can independently drift from what `PhpMapper` really emits. ~keep
pub(super) fn enum_aware_php_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => "string".to_string(),
        TypeRef::Optional(inner) => {
            let inner_type = enum_aware_php_type(inner, enum_names);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        _ => php_type(ty),
    }
}

/// PHPDoc counterpart of [`enum_aware_php_type`], keeping the generic value types PHPStan needs on
/// `array` properties (level max rejects a bare `array`).
pub(super) fn enum_aware_php_phpdoc_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Vec(inner) => format!("array<{}>", enum_aware_php_phpdoc_type(inner, enum_names)),
        TypeRef::Map(key, value) => format!(
            "array<{}, {}>",
            enum_aware_php_phpdoc_type(key, enum_names),
            enum_aware_php_phpdoc_type(value, enum_names)
        ),
        TypeRef::Optional(inner) => {
            let inner_type = enum_aware_php_phpdoc_type(inner, enum_names);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        _ => enum_aware_php_type(ty, enum_names),
    }
}
