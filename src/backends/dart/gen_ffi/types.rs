use crate::codegen::naming::{PublicIdentifierKind, public_field_name, public_host_identifier};
use crate::codegen::shared::binding_fields;
use crate::core::config::Language;
use crate::core::ir::{EnumDef, TypeDef};

use super::type_map::dart_type;
use crate::backends::dart::template_env;

/// Emit a Dart class for a struct-style type.
///
/// For types with no fields (opaque handles), the class wraps an opaque
/// `Pointer<Void>`. For value types with fields, fields are Dart-typed
/// and the class is annotated with `@freezed` to enable copyWith, toJson,
/// value equality, and hashCode generation via build_runner.
pub(super) fn emit_type(ty: &TypeDef, out: &mut String) {
    if !ty.doc.is_empty() {
        let doc_lines: Vec<String> = ty.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }

    if ty.fields.is_empty() || ty.is_opaque {
        out.push_str(&template_env::render(
            "class_open.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        out.push_str("  final Pointer<Void> _ptr;\n");
        out.push_str(&template_env::render(
            "single_param_constructor.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                param_name => "_ptr",
            },
        ));
        out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
        return;
    }

    let visible_fields: Vec<_> = binding_fields(&ty.fields).collect();

    if visible_fields.len() == 1 {
        let field = visible_fields[0];
        let ty_str = dart_type(&field.ty, field.optional);
        let name = public_field_name(Language::Dart, &field.name, None);
        out.push_str(&template_env::render(
            "freezed_class_single_param.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                param_name => name.as_str(),
                ty_str => ty_str,
            },
        ));
    } else {
        out.push_str(&template_env::render(
            "freezed_class_open.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        for field in &visible_fields {
            let ty_str = dart_type(&field.ty, field.optional);
            let name = public_field_name(Language::Dart, &field.name, None);
            if !field.doc.is_empty() {
                let doc_lines: Vec<String> = field.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "    ",
                        lines => doc_lines,
                    },
                ));
            }
            out.push_str(&template_env::render(
                "freezed_required_param.jinja",
                minijinja::context! {
                    ty_str => ty_str,
                    name => name.as_str(),
                },
            ));
        }
        out.push_str(&template_env::render(
            "freezed_constructor_close.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
    }
}

/// Emit a Dart enum (unit variants only in FFI mode).
///
/// Data variants (tagged unions) cannot be expressed ergonomically via
/// `dart:ffi` since C has no stable tagged-union ABI. Non-unit variants
/// emit an unsupported comment and are skipped.
pub(super) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        let doc_lines: Vec<String> = en.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }

    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&template_env::render(
            "enum_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            if !variant.doc.is_empty() {
                let doc_lines: Vec<String> = variant.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "  ",
                        lines => doc_lines,
                    },
                ));
            }
            let vname = public_host_identifier(Language::Dart, PublicIdentifierKind::Field, &variant.name);
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    vname => vname.as_str(),
                    suffix => suffix,
                },
            ));
        }
        out.push_str(&template_env::render("enum_close.jinja", minijinja::context! {}));
    } else {
        out.push_str(&template_env::render(
            "enum_data_variants_todo.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        out.push_str(&template_env::render(
            "enum_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            let vname = public_host_identifier(Language::Dart, PublicIdentifierKind::Field, &variant.name);
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    vname => vname.as_str(),
                    suffix => suffix,
                },
            ));
        }
        out.push_str(&template_env::render("enum_close.jinja", minijinja::context! {}));
    }
}
