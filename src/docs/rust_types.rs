//! Rust-facing type and signature rendering for the `api-rust` reference page.
//!
//! Every other language page describes a *binding*, so it is rendered from the normalized IR
//! through [`crate::docs::type_mapping::doc_type`]. The Rust page describes the crate itself, so
//! it must show what the source declared: borrows, `&mut`, the receiver the method actually
//! takes, and type names the sanitizer replaced with bindable stand-ins. Keeping that logic here
//! rather than in `type_mapping` is deliberate — normalization is still correct for the other
//! sixteen languages, and this module must not change what they emit. ~keep

use crate::core::config::Language;
use crate::core::ir::{FieldDef, MethodDef, ParamDef, ReceiverKind, TypeRef};
use crate::docs::type_mapping::doc_type;

/// Render `param` as the Rust source type the function signature declares.
///
/// The IR cannot distinguish `Option<&T>` from `&Option<T>`: `extract_params` sets `is_ref` for
/// both, one from `syn::Type::Reference` and one from `option_inner_is_ref`. This resolves the
/// ambiguity toward `Option<&T>`, the shape `option_inner_is_ref` exists to detect and by far
/// the more common Rust API. ~keep
pub(crate) fn rust_param_type(param: &ParamDef, ffi_prefix: &str) -> String {
    let inner = match param.original_type.as_deref() {
        Some(original) if param.is_ref => {
            let borrow = if param.is_mut { "&mut " } else { "&" };
            format!("{borrow}{original}")
        }
        Some(original) => original.to_string(),
        None => rust_borrowed_type(
            &param.ty,
            param.is_ref,
            param.is_mut,
            param.vec_inner_is_ref,
            ffi_prefix,
        ),
    };
    if param.optional && !inner.starts_with("Option<") {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// Render `field` as the Rust source type the struct declares.
///
/// A field whose named type is not part of the binding surface reaches codegen as `String`; the
/// sanitizer records what it was in `FieldDef::original_type`, which is what makes the real name
/// recoverable here.
///
/// Trusts `original_type` whenever it is set, not only when `sanitized` is also `true`. The
/// sanitizer sets `original_type` in exactly two places (`sanitize_field`'s lossy-rewrite branch,
/// which also sets `sanitized`, and a lossless rewrite such as a known-typed fixed-size array
/// (`[Point; 4]`) collapsing to `Vec<Point>`, which does not) -- both exist for the same reason:
/// recovering what the crate actually declared once `field.ty` has been normalized away from it.
/// Gating on `sanitized` hid the second population, so a lossless array lowering rendered the Rust
/// reference page as `Vec<Point>` instead of `[Point; 4]`, the same "shows the binding shape, not
/// canonical Rust" defect this module exists to avoid everywhere else. ~keep
pub(crate) fn rust_field_type(field: &FieldDef, ffi_prefix: &str) -> String {
    let inner = match field.original_type.as_deref() {
        Some(original) => original.to_string(),
        None => doc_type(&field.ty, Language::Rust, ffi_prefix),
    };
    if field.optional && !inner.starts_with("Option<") {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// The receiver `method` declares, or `None` for an associated function.
///
/// `receiver` is `None` on IR built before the extractor recorded receivers and on synthetic
/// methods; `&self` is the safe default there because it is what the previous unconditional
/// rendering assumed, and it is the receiver the overwhelming majority of bound methods take. ~keep
pub(crate) fn rust_receiver(method: &MethodDef) -> Option<&'static str> {
    if method.is_static {
        return None;
    }
    Some(match method.receiver {
        Some(ReceiverKind::RefMut) => "&mut self",
        Some(ReceiverKind::Owned) => "self",
        Some(ReceiverKind::Ref) | None => "&self",
    })
}

/// Render `ty` as Rust source, applying `&`/`&mut` when the source borrowed it.
///
/// A borrowed `String`/`Char` is `&str` and a borrowed `Bytes` is `&[u8]` — the unsized borrow
/// forms, not `&String`/`&Vec<u8>`. `element_borrow` carries `ParamDef::vec_inner_is_ref`, which
/// is what separates `&[&str]` from `&[String]`. ~keep
fn rust_borrowed_type(ty: &TypeRef, is_ref: bool, is_mut: bool, element_borrow: bool, ffi_prefix: &str) -> String {
    if !is_ref {
        return doc_type(ty, Language::Rust, ffi_prefix);
    }
    let borrow = if is_mut { "&mut " } else { "&" };
    let borrowed = match ty {
        TypeRef::String | TypeRef::Char => "str".to_string(),
        TypeRef::Bytes => "[u8]".to_string(),
        TypeRef::Vec(inner) => {
            let element = rust_borrowed_type(inner, element_borrow, false, false, ffi_prefix);
            format!("[{element}]")
        }
        _ => doc_type(ty, Language::Rust, ffi_prefix),
    };
    format!("{borrow}{borrowed}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::PrimitiveType;
    use crate::docs::test_helpers::{TEST_PREFIX, make_param, make_ref_param};

    #[test]
    fn borrowed_str_and_slice_params_use_the_unsized_forms() {
        assert_eq!(
            rust_param_type(&make_ref_param("name", TypeRef::String, false), TEST_PREFIX),
            "&str"
        );
        assert_eq!(
            rust_param_type(&make_ref_param("data", TypeRef::Bytes, false), TEST_PREFIX),
            "&[u8]"
        );
        assert_eq!(
            rust_param_type(
                &make_ref_param(
                    "ids",
                    TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                    false
                ),
                TEST_PREFIX
            ),
            "&[u32]"
        );
    }

    #[test]
    fn a_slice_of_borrowed_elements_borrows_the_element_too() {
        let param = ParamDef {
            vec_inner_is_ref: true,
            ..make_ref_param("names", TypeRef::Vec(Box::new(TypeRef::String)), false)
        };
        assert_eq!(rust_param_type(&param, TEST_PREFIX), "&[&str]");
    }

    #[test]
    fn an_optional_borrowed_param_borrows_inside_the_option() {
        let param = make_ref_param("options", TypeRef::Named("TextOptions".to_string()), true);
        assert_eq!(rust_param_type(&param, TEST_PREFIX), "Option<&TextOptions>");
    }

    #[test]
    fn an_owned_param_keeps_its_owned_type() {
        assert_eq!(
            rust_param_type(&make_param("name", TypeRef::String, false), TEST_PREFIX),
            "String"
        );
        assert_eq!(
            rust_param_type(&make_param("data", TypeRef::Bytes, false), TEST_PREFIX),
            "Vec<u8>"
        );
    }

    /// ~keep Reproduces the lossless fixed-size-array lowering the extract sanitizer performs
    /// for a field declared `[Point; 4]`: `field.ty` becomes `Vec<Point>` (a lossless
    /// representation -- only the fixed length is not itself tracked in the IR) and
    /// `field.original_type` records what the crate actually declared, but `field.sanitized`
    /// stays `false` because nothing was lost. `"[Point ; 4]"` (space before the semicolon) is
    /// asserted verbatim, not the prettier `"[Point; 4]"`, because `TypeRef::Named`'s payload is
    /// exactly what `type_resolver::normalize_type_string` produces: it strips the cosmetic
    /// spaces `quote` inserts around `< > [ ] ( ) , * & :`, and `;` is not in that set.
    #[test]
    fn rust_field_type_prefers_original_type_for_a_lossless_array_lowering() {
        let field = FieldDef {
            ty: TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))),
            original_type: Some("[Point ; 4]".to_string()),
            sanitized: false,
            ..crate::docs::test_helpers::make_field(
                "points",
                TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))),
                false,
                None,
            )
        };
        assert_eq!(rust_field_type(&field, TEST_PREFIX), "[Point ; 4]");
    }

    /// ~keep Control: an ordinary field with no `original_type` at all must keep rendering from
    /// `field.ty` through `doc_type` -- guards against a fix that trusts `original_type`
    /// unconditionally instead of falling back when it is genuinely absent.
    #[test]
    fn rust_field_type_falls_back_to_doc_type_when_original_type_is_absent() {
        let field = crate::docs::test_helpers::make_field("name", TypeRef::String, false, None);
        assert_eq!(rust_field_type(&field, TEST_PREFIX), "String");
    }
}
