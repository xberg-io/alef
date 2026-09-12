//! Decides whether an emitted Swift DTO needs an explicit `init(from decoder:)`.
//!
//! Lives in its own file rather than in `gen_bindings/dto.rs` because that file is over the
//! `file-modularization` cap and its ratchet ceiling (`tests/file_size_baseline.txt`), so it may
//! not grow. ~keep

use crate::codegen::shared::binding_fields;
use crate::core::ir::TypeDef;

/// Whether `ty` must carry a hand-emitted `init(from decoder:)` rather than relying on Swift's
/// synthesized `Codable` conformance.
///
/// ~keep The synthesized decoder requires every non-Optional key to be **present** in the JSON.
/// A field carrying `#[serde(skip_serializing_if = "...")]` is omitted from the payload entirely
/// whenever its predicate holds, so such a type decodes only by accident. This gate used to ask
/// `ty.has_default` alone, which is a *type*-level fact and answers a different question: a type
/// that derives no `Default` can still own a skippable field. A document-tree node whose
/// `children`/`annotations` vectors carry `skip_serializing_if = "Vec::is_empty"` with
/// `optional: false` is the canonical shape: both keys are absent for every leaf node, so the
/// synthesized decoder throws `DecodingError.keyNotFound: Key 'children' not found`.
/// `emit_decoder_init` already renders `decodeIfPresent ?? []` for exactly these fields -- it was
/// simply never reached.
pub(crate) fn needs_decoder_init(ty: &TypeDef) -> bool {
    ty.has_default || binding_fields(&ty.fields).any(|field| field.serde_skip_serializing_if)
}

#[cfg(test)]
mod tests {
    use super::needs_decoder_init;
    use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};

    fn vec_field(name: &str, skip_serializing_if: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
            serde_skip_serializing_if: skip_serializing_if,
            ..Default::default()
        }
    }

    fn ty_with(has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: "DocumentNode".to_string(),
            has_default,
            fields,
            ..Default::default()
        }
    }

    #[test]
    fn a_skippable_field_requires_a_custom_decoder_even_without_a_default_derive() {
        assert!(
            needs_decoder_init(&ty_with(false, vec![vec_field("children", true)])),
            "a `skip_serializing_if` field is absent from the JSON, so the synthesized decoder throws"
        );
    }

    #[test]
    fn a_default_deriving_type_still_requires_a_custom_decoder() {
        assert!(needs_decoder_init(&ty_with(true, vec![vec_field("children", false)])));
    }

    #[test]
    fn a_type_with_neither_a_default_nor_a_skippable_field_keeps_the_synthesized_decoder() {
        assert!(
            !needs_decoder_init(&ty_with(false, vec![vec_field("nodes", false)])),
            "every key is always written, so Swift's synthesized Codable decodes it correctly"
        );
    }

    #[test]
    fn a_binding_excluded_skippable_field_does_not_force_a_decoder() {
        let mut excluded = vec_field("children", true);
        excluded.binding_excluded = true;
        assert!(
            !needs_decoder_init(&ty_with(false, vec![excluded])),
            "an excluded field is not emitted, so it cannot make the emitted struct undecodable"
        );
    }
}
