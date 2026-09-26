use crate::codegen::naming::{PublicIdentifierKind, public_field_name, public_host_identifier};
use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, FieldDef, TypeDef};

use super::type_map::dart_type;
use crate::backends::dart::template_env;

/// Emit a Dart class for a struct-style type.
///
/// For types with no fields (opaque handles), the class wraps an opaque
/// `Pointer<Void>`. For value types with fields, fields are Dart-typed
/// and the class is annotated with `@freezed` to enable copyWith, toJson,
/// value equality, and hashCode generation via build_runner.
pub(super) fn emit_type(ty: &TypeDef, config: &ResolvedCrateConfig, out: &mut String) {
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
        emit_opaque_handle_class(ty, out);
        return;
    }

    let visible_fields: Vec<_> = binding_fields(&ty.fields).collect();

    if visible_fields.len() == 1 {
        let field = visible_fields[0];
        let ty_str = dart_type(&field.ty, field.optional);
        let name = dart_field_identifier(ty, field, config);
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
            let name = dart_field_identifier(ty, field, config);
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

/// Emit the opaque-handle class shape: a `Pointer<Void>`-wrapping Dart class with a
/// single `_ptr` field constructor, used for types with no bindable fields.
fn emit_opaque_handle_class(ty: &TypeDef, out: &mut String) {
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
}

/// The variants this Dart enum may advertise: the ones the FFI crate's
/// `{prefix}_{enum}_from_i32` / `_from_str` validators actually accept.
///
/// `codegen::conversions::enum_variant_declaration` is the authority
/// `ffi::gen_bindings::types::declared_variant_indices` filters those validators with, so asking
/// it here is what keeps this Dart declaration from advertising a case the linked native library
/// rejects. A HOST-owned cfg-gated variant resolves to `Keep { cfg: Some(_) }`, which the FFI
/// validator also treats as kept -- dropping it here instead (what `with_cfg_filtered_deep` does
/// for go/java/kotlin/csharp/zig, which filter their own surface rather than mirror the
/// validator's) would trade this over-exposure for an under-exposure. ~keep
fn declared_variants<'a>(
    en: &'a EnumDef,
    host_crate_name: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> Vec<&'a crate::core::ir::EnumVariant> {
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(host_crate_name, &en.rust_path);
    en.variants
        .iter()
        .filter(|variant| {
            !matches!(
                crate::codegen::conversions::enum_variant_declaration(variant, is_host_enum, configured_features),
                crate::codegen::conversions::VariantDeclaration::Drop
            )
        })
        .collect()
}

/// Emit a Dart enum (unit variants only in FFI mode).
///
/// Data variants (tagged unions) cannot be expressed ergonomically via
/// `dart:ffi` since C has no stable tagged-union ABI. Non-unit variants
/// emit an unsupported comment and are skipped.
/// Resolve a `@freezed` constructor parameter name, applying `[crates.dart] rename_fields`.
///
/// `public_field_name`'s third argument was previously hardcoded `None`, which made the seam
/// look wired while silently discarding every configured rename. A `@freezed` named parameter
/// is simultaneously the class's public property, so this one identifier is the whole Dart FFI
/// field surface. ~keep
fn dart_field_identifier(ty: &TypeDef, field: &FieldDef, config: &ResolvedCrateConfig) -> String {
    let renamed = config.resolve_field_name(Language::Dart, &ty.name, &field.name);
    public_field_name(Language::Dart, &field.name, renamed.as_deref())
}

pub(super) fn emit_enum(
    en: &EnumDef,
    host_crate_name: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
    out: &mut String,
) {
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

    let declared = declared_variants(en, host_crate_name, configured_features);
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&template_env::render(
            "enum_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        let count = declared.len();
        for (idx, variant) in declared.iter().enumerate() {
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
        let count = declared.len();
        for (idx, variant) in declared.iter().enumerate() {
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
