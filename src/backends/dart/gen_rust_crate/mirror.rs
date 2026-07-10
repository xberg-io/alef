use crate::codegen::shared::binding_fields;
use crate::core::ir::{EnumDef, ErrorDef, FieldDef, PrimitiveType, TypeDef, TypeRef};

use super::conversions::{frb_rust_type, frb_rust_type_inner, primitive_name};

/// Emit rustdoc `///` lines above the next item.
///
/// `flutter_rust_bridge` propagates Rust doc comments to the generated Dart
/// classes, so attaching `///` lines to mirror structs, mirror enums, their
/// fields, and their variants makes the doc text reach the Dart side without
/// any post-processing.
fn emit_rust_doc(doc: &str, indent: &str, out: &mut String) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        if line.is_empty() {
            out.push_str("///\n");
        } else {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
}

pub(crate) fn emit_mirror_struct(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    use crate::backends::dart::template_env;

    if ty.is_opaque {
        // Opaque handle types cannot use #[frb(mirror)] because the local mirror struct
        // is zero-sized while the core type has data. Instead, emit a #[frb(opaque)] wrapper
        let source_module = source_crate_name.replace('-', "_");
        let inner_path = if ty.rust_path.is_empty() {
            format!("{source_module}::{}", ty.name)
        } else {
            ty.rust_path.replace('-', "_")
        };
        emit_rust_doc(&ty.doc, "", out);
        let wrapper_cfg = super::helpers::widen_opaque_wrapper_cfg(ty.cfg.as_deref().unwrap_or(""));
        out.push_str(&template_env::render(
            "rust_opaque_wrapper_struct.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                inner_path => inner_path.as_str(),
                source_cfg => wrapper_cfg.as_str(),
            },
        ));
        return;
    }

    emit_rust_doc(&ty.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_struct_attribute.jinja",
        minijinja::context! {
            name => ty.name.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));
    out.push_str(&template_env::render(
        "rust_mirror_struct_open.jinja",
        minijinja::context! {
            name => ty.name.as_str(),
        },
    ));
    for field in binding_fields(&ty.fields) {
        let rust_ty = frb_rust_type(&field.ty, field.optional);
        emit_rust_doc(&field.doc, "    ", out);
        out.push_str(&template_env::render(
            "rust_mirror_struct_field.jinja",
            minijinja::context! {
                field_name => field.name.as_str(),
                rust_ty => rust_ty,
            },
        ));
    }
    out.push_str(&template_env::render(
        "rust_mirror_struct_close.jinja",
        minijinja::context! {},
    ));
}

pub(crate) fn emit_mirror_enum(out: &mut String, en: &EnumDef) {
    use crate::backends::dart::template_env;
    let all_unit = en.variants.iter().all(|v| v.fields.iter().all(|f| f.binding_excluded));
    emit_rust_doc(&en.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_enum_attribute.jinja",
        minijinja::context! {
            name => en.name.as_str(),
            source_cfg => en.cfg.as_deref().unwrap_or(""),
        },
    ));
    out.push_str(&template_env::render(
        "rust_mirror_enum_open.jinja",
        minijinja::context! {
            name => en.name.as_str(),
        },
    ));
    if all_unit {
        for variant in &en.variants {
            emit_rust_doc(&variant.doc, "    ", out);
            out.push_str(&template_env::render(
                "rust_mirror_enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                },
            ));
        }
    } else {
        for variant in &en.variants {
            let visible_fields: Vec<&_> = variant.fields.iter().filter(|f| !f.binding_excluded).collect();
            if visible_fields.is_empty() {
                emit_rust_doc(&variant.doc, "    ", out);
                out.push_str(&template_env::render(
                    "rust_mirror_enum_unit_variant.jinja",
                    minijinja::context! {
                        variant_name => variant.name.as_str(),
                    },
                ));
            } else {
                emit_rust_doc(&variant.doc, "    ", out);
                out.push_str(&template_env::render(
                    "rust_mirror_enum_data_variant_open.jinja",
                    minijinja::context! {
                        variant_name => variant.name.as_str(),
                    },
                ));
                for (idx, f) in visible_fields.iter().enumerate() {
                    let fname = if f.name.is_empty() || f.name.starts_with('_') {
                        format!("field{idx}")
                    } else {
                        f.name.clone()
                    };
                    let rust_ty = frb_rust_type_inner(&f.ty);
                    emit_rust_doc(&f.doc, "        ", out);
                    out.push_str(&template_env::render(
                        "rust_mirror_enum_data_variant_field.jinja",
                        minijinja::context! {
                            field_name => fname,
                            rust_ty => rust_ty,
                        },
                    ));
                }
                out.push_str(&template_env::render(
                    "rust_mirror_enum_data_close.jinja",
                    minijinja::context! {},
                ));
            }
        }
    }
    out.push_str("}\n");
}

/// Return the conversion expression to reconstruct a real-type field value from a
/// mirror field binding.
///
/// Mirror fields use FRB-widened types: integers → `i64`, floats → `f64`,
/// `Duration` → `i64` millis, and optional primitive/Duration fields collapse to
/// their non-optional widened form. String/Bytes/Vec optional fields retain
/// `Option<...>` wrapping in the mirror because FRB handles those correctly.
///
/// `field_expr` is the pattern-binding identifier (e.g. `"f_status"`). The
/// caller binds it via `ref f_<name>` so its type is `&MirrorFieldType`.
fn field_from_expr(field: &FieldDef, field_expr: &str) -> String {
    match &field.ty {
        TypeRef::Primitive(prim) => {
            let native = primitive_name(prim);
            let base = match prim {
                PrimitiveType::I64 | PrimitiveType::F64 | PrimitiveType::Bool => {
                    format!("*{field_expr}")
                }
                _ => format!("*{field_expr} as {native}"),
            };
            if field.optional { format!("Some({base})") } else { base }
        }
        TypeRef::Duration => {
            let base = format!("std::time::Duration::from_millis(*{field_expr} as u64)");
            if field.optional { format!("Some({base})") } else { base }
        }
        TypeRef::String | TypeRef::Bytes => {
            if field.optional {
                format!("Some({field_expr}.clone())")
            } else {
                format!("{field_expr}.clone()")
            }
        }
        TypeRef::Char => {
            let base = format!("{field_expr}.chars().next().unwrap_or('\\0')");
            if field.optional { format!("Some({base})") } else { base }
        }
        TypeRef::Optional(inner) => {
            let inner_field = FieldDef {
                name: field.name.clone(),
                ty: *inner.clone(),
                optional: false,
                ..field.clone()
            };
            let inner_expr = field_from_expr(&inner_field, "v");
            format!("{field_expr}.as_ref().map(|v| {inner_expr})")
        }
        TypeRef::Vec(inner) => {
            let inner_field = FieldDef {
                name: "_x".to_string(),
                ty: *inner.clone(),
                optional: false,
                ..field.clone()
            };
            let inner_expr = field_from_expr(&inner_field, "x");
            format!("{field_expr}.iter().map(|x| {inner_expr}).collect()")
        }
        _ => format!("{field_expr}.clone()"),
    }
}

/// Return true if every field in the variant can be safely reconstructed in the
/// `From<&MirrorEnum>` impl.
///
/// Sanitized fields represent types that were erased to `String` during
/// extraction (e.g. `serde_json::Error`). Such originals cannot be recovered
/// from the mirror, so the entire variant must be skipped in the From impl.
fn variant_is_reconstructible(fields: &[&FieldDef]) -> bool {
    fields.iter().all(|f| !f.sanitized)
}

/// Emit a safe `impl From<&MirrorEnum> for CorePath` conversion.
///
/// Each reconstructible variant is matched arm-by-arm with explicit field casts
/// from FRB-widened types (i64/f64) to the real primitive widths. Variants whose
/// fields include sanitized (erased) types are skipped — a wildcard arm with
/// `unreachable!` is emitted to cover them so the match stays exhaustive.
/// `#[allow(unreachable_patterns)]` is emitted unconditionally to suppress the
/// compiler warning when all variants are in fact reconstructible.
fn emit_from_impl(out: &mut String, error: &ErrorDef, core_path: &str, error_cfg: &str) {
    use crate::backends::dart::template_env;

    let any_skipped = error.variants.iter().any(|v| {
        let visible_fields: Vec<&FieldDef> = v.fields.iter().filter(|f| !f.binding_excluded).collect();
        !v.is_unit && !visible_fields.is_empty() && !variant_is_reconstructible(&visible_fields)
    });

    out.push_str(&template_env::render(
        "rust_mirror_error_from_impl_open.rs.jinja",
        minijinja::context! {
            name => error.name.as_str(),
            core_path => core_path,
            source_cfg => error_cfg,
        },
    ));
    for variant in &error.variants {
        let vname = &variant.name;
        if variant.is_unit {
            out.push_str(&template_env::render(
                "rust_mirror_error_unit_from_arm.rs.jinja",
                minijinja::context! {
                    name => error.name.as_str(),
                    vname => vname.as_str(),
                },
            ));
        } else if !variant.is_unit && variant.is_tuple && variant.fields.iter().all(|f| f.binding_excluded) {
            out.push_str(&template_env::render(
                "rust_mirror_error_excluded_from_arm.rs.jinja",
                minijinja::context! {
                    name => error.name.as_str(),
                    vname => vname.as_str(),
                },
            ));
        } else if !variant.is_unit && variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "rust_mirror_error_unit_from_arm.rs.jinja",
                minijinja::context! {
                    name => error.name.as_str(),
                    vname => vname.as_str(),
                },
            ));
        } else if variant.fields.iter().all(|f| f.binding_excluded) {
            out.push_str(&template_env::render(
                "rust_mirror_error_excluded_from_arm.rs.jinja",
                minijinja::context! {
                    name => error.name.as_str(),
                    vname => vname.as_str(),
                },
            ));
        } else {
            let visible_fields: Vec<&FieldDef> = variant.fields.iter().filter(|f| !f.binding_excluded).collect();

            if !variant_is_reconstructible(&visible_fields) {
                continue;
            }

            let is_tuple_variant = visible_fields
                .iter()
                .all(|f| f.name.is_empty() || f.name.starts_with('_'));

            let field_names: Vec<String> = visible_fields
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    if f.name.is_empty() || f.name.starts_with('_') {
                        format!("field{idx}")
                    } else {
                        f.name.clone()
                    }
                })
                .collect();

            let pat_fields: String = field_names
                .iter()
                .map(|fname| format!("{fname}: f_{fname}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&template_env::render(
                "rust_mirror_error_struct_pattern_arm.rs.jinja",
                minijinja::context! {
                    name => error.name.as_str(),
                    vname => vname.as_str(),
                    pat_fields => pat_fields.as_str(),
                },
            ));

            if is_tuple_variant {
                let mut args: Vec<String> = visible_fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let fname = &field_names[i];
                        field_from_expr(f, &format!("f_{fname}"))
                    })
                    .collect();
                let excluded_count = variant.fields.iter().filter(|f| f.binding_excluded).count();
                for _ in 0..excluded_count {
                    args.push("Default::default()".to_string());
                }
                out.push_str(&template_env::render(
                    "rust_mirror_error_tuple_return.rs.jinja",
                    minijinja::context! {
                        vname => vname.as_str(),
                        args => args.join(", "),
                    },
                ));
            } else {
                let mut real_fields: Vec<String> = Vec::new();
                for (i, f) in visible_fields.iter().enumerate() {
                    let fname = &field_names[i];
                    let expr = field_from_expr(f, &format!("f_{fname}"));
                    real_fields.push(format!("                    {fname}: {expr}"));
                }
                for f in variant.fields.iter().filter(|f| f.binding_excluded) {
                    real_fields.push(format!("                    {}: Default::default()", f.name));
                }
                out.push_str(&template_env::render(
                    "rust_mirror_error_struct_return.rs.jinja",
                    minijinja::context! {
                        vname => vname.as_str(),
                        real_fields => real_fields.join(",\n"),
                    },
                ));
            }
            out.push_str("            }\n");
        }
    }
    if any_skipped {
        out.push_str(&template_env::render(
            "rust_mirror_error_sanitized_wildcard_arm.rs.jinja",
            minijinja::context! {},
        ));
    }
    out.push_str(&template_env::render(
        "rust_mirror_error_from_impl_close.rs.jinja",
        minijinja::context! {},
    ));
}

/// Emit a `#[frb(mirror(ErrorName))]` enum + safe `impl From` conversion +
/// `impl ErrorName` block with `#[frb]` introspection methods.
///
/// flutter_rust_bridge translates the mirrored enum into a Dart sealed class with
/// per-variant subclasses. The `impl` block methods annotated with `#[frb]` are
/// surfaced as Dart instance methods on the sealed class.
///
/// Introspection methods convert `self` to the core error type via a safe
/// `From<&MirrorEnum>` impl that reconstructs each variant field-by-field with
/// explicit primitive casts. This avoids the unsound raw-pointer transmute that
/// would arise from mismatched field widths (e.g. `i64` in the mirror vs `u16`
/// in the real type).
pub(crate) fn emit_mirror_error(out: &mut String, error: &ErrorDef, source_crate_name: &str) {
    use crate::backends::dart::template_env;

    emit_rust_doc(&error.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_enum_attribute.jinja",
        minijinja::context! {
            name => error.name.as_str(),
            source_cfg => "",
        },
    ));
    out.push_str(&template_env::render(
        "rust_mirror_enum_open.jinja",
        minijinja::context! {
            name => error.name.as_str(),
        },
    ));

    for variant in &error.variants {
        emit_rust_doc(&variant.doc, "    ", out);
        if variant.is_unit {
            out.push_str(&template_env::render(
                "rust_mirror_enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                },
            ));
        } else if !variant.is_unit && variant.is_tuple && variant.fields.iter().all(|f| f.binding_excluded) {
            out.push_str(&template_env::render(
                "rust_mirror_enum_data_variant_open.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                },
            ));
            out.push_str(&template_env::render(
                "rust_mirror_enum_data_variant_field.jinja",
                minijinja::context! {
                    field_name => "field0",
                    rust_ty => "String",
                },
            ));
            out.push_str(&template_env::render(
                "rust_mirror_enum_data_close.jinja",
                minijinja::context! {},
            ));
        } else {
            let visible_fields: Vec<&FieldDef> = variant.fields.iter().filter(|f| !f.binding_excluded).collect();
            if visible_fields.is_empty() {
                out.push_str(&template_env::render(
                    "rust_mirror_enum_unit_variant.jinja",
                    minijinja::context! {
                        variant_name => variant.name.as_str(),
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "rust_mirror_enum_data_variant_open.jinja",
                    minijinja::context! {
                        variant_name => variant.name.as_str(),
                    },
                ));
                for (idx, f) in visible_fields.iter().enumerate() {
                    let fname = if f.name.is_empty() || f.name.starts_with('_') {
                        format!("field{idx}")
                    } else {
                        f.name.clone()
                    };
                    let rust_ty = frb_rust_type_inner(&f.ty);
                    out.push_str(&template_env::render(
                        "rust_mirror_enum_data_variant_field.jinja",
                        minijinja::context! {
                            field_name => fname,
                            rust_ty => rust_ty,
                        },
                    ));
                }
                out.push_str(&template_env::render(
                    "rust_mirror_enum_data_close.jinja",
                    minijinja::context! {},
                ));
            }
        }
    }
    out.push_str("}\n");

    let bridge_methods: Vec<&crate::core::ir::MethodDef> = error.methods.iter().filter(|m| !m.sanitized).collect();
    if bridge_methods.is_empty() {
        return;
    }

    let core_path = if error.rust_path.is_empty() {
        format!("{source_crate_name}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    emit_from_impl(out, error, &core_path, "");

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_error_impl_open.rs.jinja",
        minijinja::context! {
            error_name => error.name.as_str(),
            source_cfg => "",
        },
    ));
    for method in bridge_methods {
        emit_rust_doc(&method.doc, "    ", out);
        let ret_ty = frb_rust_type_inner(&method.return_type);
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_error_method_open.rs.jinja",
            minijinja::context! {
                method_name => method.name.as_str(),
                ret_ty => ret_ty.as_str(),
            },
        ));
        let call_suffix: String =
            if method.returns_ref && matches!(method.return_type, crate::core::ir::TypeRef::String) {
                ".to_string()".to_string()
            } else if let crate::core::ir::TypeRef::Primitive(ref prim) = method.return_type {
                let native = primitive_name(prim);
                let frb_ty = frb_rust_type_inner(&method.return_type);
                if native != frb_ty.as_str() {
                    format!(" as {frb_ty}")
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_error_method_body.rs.jinja",
            minijinja::context! {
                core_path => core_path.as_str(),
                method_name => method.name.as_str(),
                call_suffix => call_suffix.as_str(),
            },
        ));
        out.push_str("    }\n");
    }
    out.push_str("}\n");
}
