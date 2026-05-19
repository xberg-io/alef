use alef_codegen::shared::binding_fields;
use alef_core::ir::{EnumDef, ErrorDef, TypeDef};

use super::conversions::{frb_rust_type, frb_rust_type_inner};

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
    use crate::template_env;

    if ty.is_opaque {
        // Opaque handle types cannot use #[frb(mirror)] because the local mirror struct
        // is zero-sized while the core type has data. Instead, emit a #[frb(opaque)] wrapper
        // struct so FRB v2 manages the value as a reference-counted opaque handle (RustAutoOpaque).
        // Bridge functions use `.inner` to access the wrapped core type.
        //
        // Prefer the IR-recorded `rust_path` (e.g. `kreuzberg::extractors::HwpxExtractor`)
        // over the naive `{source_crate}::{name}` form, which only resolves for types
        // re-exported at the crate root.
        let source_module = source_crate_name.replace('-', "_");
        let inner_path = if ty.rust_path.is_empty() {
            format!("{source_module}::{}", ty.name)
        } else {
            ty.rust_path.replace('-', "_")
        };
        emit_rust_doc(&ty.doc, "", out);
        out.push_str(&template_env::render(
            "rust_opaque_wrapper_struct.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                inner_path => inner_path.as_str(),
            },
        ));
        return;
    }

    // FRB v2 mirror convention: the mirror struct keeps the same name as the
    // original; the `mirror` attribute argument tells FRB which type this
    // declaration shadows for codegen purposes.
    emit_rust_doc(&ty.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_struct_attribute.jinja",
        minijinja::context! {
            name => ty.name.as_str(),
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
    use crate::template_env;
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    emit_rust_doc(&en.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_enum_attribute.jinja",
        minijinja::context! {
            name => en.name.as_str(),
        },
    ));
    if all_unit {
        out.push_str(&template_env::render(
            "rust_mirror_enum_open.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        for variant in &en.variants {
            emit_rust_doc(&variant.doc, "    ", out);
            out.push_str(&template_env::render(
                "rust_mirror_enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                },
            ));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&template_env::render(
            "rust_mirror_enum_open.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        for variant in &en.variants {
            if variant.fields.is_empty() {
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
                for (idx, f) in variant.fields.iter().enumerate() {
                    // Tuple-variant fields land in the IR as "_0", "_1", ... but
                    // flutter_rust_bridge strips a leading underscore from Dart
                    // field names — leaving an invalid bare digit. Rename any
                    // empty or "_N"-style field to a Dart-safe "fieldN".
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
        out.push_str("}\n");
    }
}

/// Emit a `#[frb(mirror(ErrorName))]` enum + `impl ErrorName` block with `#[frb]`
/// introspection methods that delegate to the core error type.
///
/// flutter_rust_bridge translates the mirrored enum into a Dart sealed class with
/// per-variant subclasses. The `impl` block methods annotated with `#[frb]` are
/// surfaced as Dart instance methods on the sealed class.
///
/// # SAFETY
///
/// Each introspection method casts `self` to the core error type via a raw pointer
/// transmute. This is sound because `#[frb(mirror(T))]` guarantees that the local
/// mirror enum has the same memory layout as the core type `T` — FRB's codegen
/// relies on this invariant and panics at compile time if layouts differ.
pub(crate) fn emit_mirror_error(out: &mut String, error: &ErrorDef, source_crate_name: &str) {
    use crate::template_env;

    emit_rust_doc(&error.doc, "", out);
    out.push_str(&template_env::render(
        "rust_mirror_enum_attribute.jinja",
        minijinja::context! {
            name => error.name.as_str(),
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
        } else {
            out.push_str(&template_env::render(
                "rust_mirror_enum_data_variant_open.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
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
    out.push_str("}\n");

    // Emit introspection methods only when the error has whitelisted methods.
    let bridge_methods: Vec<&alef_core::ir::MethodDef> = error.methods.iter().filter(|m| !m.sanitized).collect();
    if bridge_methods.is_empty() {
        return;
    }

    // Resolve the fully-qualified core type path, preferring the IR-recorded `rust_path`
    // (e.g. `liter_llm::error::LiterLlmError`) over the naive `{crate}::{Name}` fallback.
    let core_path = if error.rust_path.is_empty() {
        format!("{source_crate_name}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };

    out.push_str(&format!("\nimpl {} {{\n", error.name));
    for method in bridge_methods {
        emit_rust_doc(&method.doc, "    ", out);
        let ret_ty = frb_rust_type_inner(&method.return_type);
        out.push_str("    #[frb]\n");
        out.push_str(&format!("    pub fn {}(&self) -> {ret_ty} {{\n", method.name));
        // SAFETY: `#[frb(mirror(T))]` guarantees identical layout between
        // the local mirror enum and the core type `T`. Casting `self` via a
        // raw pointer is equivalent to the transmute FRB itself performs when
        // encoding/decoding values across the bridge.
        out.push_str(&format!(
            "        // SAFETY: mirror layout is identical to {core_path} (FRB invariant).\n"
        ));
        // Build any coercion suffix needed to reconcile the core return type with the FRB
        // bridge return type declared above:
        //   - `&str` (returns_ref=true + String TypeRef) → `.to_string()`
        //   - narrow integer or float (e.g. u16) → ` as i64` / ` as f64`
        let call_suffix: String =
            if method.returns_ref && matches!(method.return_type, alef_core::ir::TypeRef::String) {
                // Core returns &str; bridge declares String.
                ".to_string()".to_string()
            } else if let alef_core::ir::TypeRef::Primitive(ref prim) = method.return_type {
                use super::conversions::primitive_name;
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
        out.push_str(&format!(
            "        unsafe {{ &*(self as *const Self as *const {core_path}) }}.{}(){call_suffix}\n",
            method.name
        ));
        out.push_str("    }\n");
    }
    out.push_str("}\n");
}
