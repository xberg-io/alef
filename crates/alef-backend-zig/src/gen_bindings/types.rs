use alef_codegen::keywords::zig_ident;
use alef_codegen::shared::binding_fields;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef, TypeRef};

use crate::type_map::ZigMapper;

use super::helpers::emit_cleaned_zig_doc;

pub(crate) fn emit_type(ty: &TypeDef, out: &mut String) {
    emit_cleaned_zig_doc(out, &ty.doc, "");
    out.push_str(&crate::template_env::render(
        "type_header.jinja",
        minijinja::context! {
            type_name => &ty.name,
        },
    ));
    for field in binding_fields(&ty.fields) {
        // Struct fields inherit `///` doc comments inside the struct body — the
        // four-space indent matches the field declaration emitted by
        // `type_field.jinja`.
        emit_cleaned_zig_doc(out, &field.doc, "    ");
        let ty_str = zig_field_type(&field.ty, field.optional);
        out.push_str(&crate::template_env::render(
            "type_field.jinja",
            minijinja::context! {
                field_name => zig_ident(&field.name),
                field_type => ty_str,
            },
        ));
    }
    out.push_str("};\n");
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    emit_cleaned_zig_doc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::template_env::render(
            "enum_unit_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            // Variant docstrings render as `///` comments immediately above the
            // tag declaration. Empty docs no-op via `emit_cleaned_zig_doc`.
            emit_cleaned_zig_doc(out, &variant.doc, "    ");
            let tag_value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| to_snake_case(&variant.name));
            out.push_str(&crate::template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => zig_ident(&tag_value),
                },
            ));
        }
        out.push_str("};\n");
    } else {
        out.push_str(&crate::template_env::render(
            "enum_tagged_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            // Tagged-union variants carry their rustdoc as `///` above the tag.
            emit_cleaned_zig_doc(out, &variant.doc, "    ");
            let tag_value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| to_snake_case(&variant.name));
            let tag = zig_ident(&tag_value);
            if variant.fields.is_empty() {
                out.push_str(&crate::template_env::render(
                    "enum_variant_void.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
            } else if variant.fields.len() == 1 {
                let ty_str = zig_field_type(&variant.fields[0].ty, variant.fields[0].optional);
                out.push_str(&crate::template_env::render(
                    "enum_variant_single.jinja",
                    minijinja::context! {
                        tag => &tag,
                        type_str => ty_str,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "enum_variant_struct_header.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
                for f in &variant.fields {
                    let name = if f.name.is_empty() {
                        "value".into()
                    } else {
                        zig_ident(&f.name)
                    };
                    let ty_str = zig_field_type(&f.ty, f.optional);
                    out.push_str(&crate::template_env::render(
                        "enum_variant_struct_field.jinja",
                        minijinja::context! {
                            field_name => name,
                            field_type => ty_str,
                        },
                    ));
                }
                out.push_str("    },\n");
            }
        }
        out.push_str("};\n");
    }
}

pub(crate) fn zig_field_type(ty: &TypeRef, optional: bool) -> String {
    let mapper = ZigMapper;
    let inner = mapper.map_type(ty);
    // Flatten double-optional: if the mapped type is already `?T` (from a
    // TypeRef::Optional inner), do not prepend another `?` — Zig does not
    // support `??T` syntax.
    if optional && !inner.starts_with('?') {
        format!("?{inner}")
    } else {
        inner
    }
}

/// Convert a PascalCase identifier to snake_case with acronym awareness.
///
/// Consecutive uppercase letters (≥2) are treated as a single acronym word.
/// The last uppercase in an all-caps run is the first letter of the next word
/// when followed by a lowercase letter.
///
/// # Examples
/// - `Rdfa`          → `rdfa`
/// - `MyType`        → `my_type`
/// - `HTMLParser`    → `html_parser`
/// - `IOError`       → `io_error`
/// - `URLPath`       → `url_path`
/// - `XMLHttpRequest`→ `xml_http_request`
/// - `JSONLD`        → `jsonld`
pub(crate) fn to_snake_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n + 4);
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if ch.is_ascii_uppercase() {
            // Determine how long this uppercase run is.
            let run_start = i;
            while i < n && chars[i].is_ascii_uppercase() {
                i += 1;
            }
            let run_end = i; // exclusive
            let run_len = run_end - run_start;
            if run_len == 1 {
                // Single uppercase letter — normal word boundary.
                if !out.is_empty() {
                    out.push('_');
                }
                out.extend(chars[run_start].to_lowercase());
            } else {
                // Multi-char uppercase run = acronym.
                // If followed by lowercase, the last char of the run starts
                // the next word: split as acronym[0..run_len-1] + next_word.
                let split = if i < n && chars[i].is_ascii_lowercase() {
                    run_len - 1
                } else {
                    run_len
                };
                // Emit acronym portion.
                if !out.is_empty() {
                    out.push('_');
                }
                for &ch in chars.iter().skip(run_start).take(split) {
                    out.extend(ch.to_lowercase());
                }
                // Emit trailing char as new word if applicable.
                if split < run_len {
                    out.push('_');
                    out.extend(chars[run_start + split].to_lowercase());
                }
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod case_tests {
    use super::to_snake_case;

    #[test]
    fn rdfa_single_word() {
        assert_eq!(to_snake_case("Rdfa"), "rdfa");
    }

    #[test]
    fn my_type_normal() {
        assert_eq!(to_snake_case("MyType"), "my_type");
    }

    #[test]
    fn html_parser_acronym_prefix() {
        assert_eq!(to_snake_case("HTMLParser"), "html_parser");
    }

    #[test]
    fn io_error_two_char_acronym() {
        assert_eq!(to_snake_case("IOError"), "io_error");
    }

    #[test]
    fn url_path_acronym_prefix() {
        assert_eq!(to_snake_case("URLPath"), "url_path");
    }

    #[test]
    fn xml_http_request_compound() {
        assert_eq!(to_snake_case("XMLHttpRequest"), "xml_http_request");
    }

    #[test]
    fn jsonld_all_caps() {
        assert_eq!(to_snake_case("JSONLD"), "jsonld");
    }

    #[test]
    fn empty_string() {
        assert_eq!(to_snake_case(""), "");
    }

    #[test]
    fn single_lowercase() {
        assert_eq!(to_snake_case("value"), "value");
    }

    #[test]
    fn single_uppercase() {
        assert_eq!(to_snake_case("A"), "a");
    }
}
