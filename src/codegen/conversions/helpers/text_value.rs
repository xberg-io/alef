use ahash::AHashSet;

use crate::core::ir::{EnumDef, FieldDef, TypeRef};

/// The text-projected unions that can carry a lossless companion in a binding.
///
/// The companion is filled with `serde_wasm_bindgen`, which needs `Serialize`, and a union
/// excluded from the binding has no generated type to store. `text_names` holds the unions listed
/// in `untagged_union_text_types`. The WASM backend and the TypeScript e2e generator both build
/// the `names` argument of [`text_value_companion`] here. ~keep
pub(crate) fn text_value_enum_names(
    enums: &[EnumDef],
    text_names: &AHashSet<String>,
    exclude_types: &[String],
) -> AHashSet<String> {
    enums
        .iter()
        .filter(|e| text_names.contains(&e.name) && e.has_serde && !exclude_types.contains(&e.name))
        .map(|e| e.name.clone())
        .collect()
}

/// The name of the lossless companion field that a text-projected union field keeps beside it.
///
/// A field whose type is a union listed in `untagged_union_text_types` is stored as display text
/// in the WASM struct, which loses the parts of a list. The backend stores the whole union as a
/// second `JsValue` field named `<field>_value` and exposes it as a getter. Every site that
/// declares, builds or converts that field asks this function, so they agree on which fields
/// have a companion. `names` holds the unions that can be serialized.
///
/// The function answers `None` for a cfg-gated, sanitized or excluded field, and when the struct
/// already has a field with the companion name. ~keep
pub(crate) fn text_value_companion(
    siblings: &[FieldDef],
    field: &FieldDef,
    names: &AHashSet<String>,
) -> Option<String> {
    if field.binding_excluded || field.sanitized || field.cfg.is_some() {
        return None;
    }
    let named = match &field.ty {
        TypeRef::Named(name) => name,
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => name,
            _ => return None,
        },
        _ => return None,
    };
    if !names.contains(named) {
        return None;
    }
    let companion = format!("{}_value", field.name);
    (!siblings.iter().any(|sibling| sibling.name == companion)).then_some(companion)
}

/// Whether the companion holds an `Option`, because the field itself can be absent.
pub(crate) fn text_value_field_is_optional(field: &FieldDef) -> bool {
    field.optional || matches!(field.ty, TypeRef::Optional(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn names() -> AHashSet<String> {
        std::iter::once("Content".to_string()).collect()
    }

    #[test]
    fn a_text_union_field_gets_a_value_companion() {
        let content = field("content", TypeRef::Named("Content".into()));
        let optional = field("body", TypeRef::Optional(Box::new(TypeRef::Named("Content".into()))));
        let siblings = vec![content.clone(), optional.clone()];
        assert_eq!(
            text_value_companion(&siblings, &content, &names()).as_deref(),
            Some("content_value")
        );
        assert_eq!(
            text_value_companion(&siblings, &optional, &names()).as_deref(),
            Some("body_value")
        );
        assert!(text_value_field_is_optional(&optional));
        assert!(!text_value_field_is_optional(&content));
    }

    #[test]
    fn only_serializable_unexcluded_text_unions_are_named() {
        let union = |name: &str, has_serde: bool| EnumDef {
            name: name.to_string(),
            has_serde,
            ..EnumDef::default()
        };
        let enums = [
            union("Content", true),
            union("Plain", true),
            union("Opaque", false),
            union("Hidden", true),
        ];
        let text: AHashSet<String> = ["Content", "Opaque", "Hidden"].iter().map(|n| n.to_string()).collect();
        let named = text_value_enum_names(&enums, &text, &["Hidden".to_string()]);
        assert_eq!(named, std::iter::once("Content".to_string()).collect());
    }

    #[test]
    fn other_fields_and_colliding_names_get_no_companion() {
        let plain = field("name", TypeRef::String);
        let other = field("kind", TypeRef::Named("Kind".into()));
        let content = field("content", TypeRef::Named("Content".into()));
        let clash = field("content_value", TypeRef::String);
        let mut gated = content.clone();
        gated.cfg = Some("feature = \"x\"".into());
        let mut sanitized = content.clone();
        sanitized.sanitized = true;
        let siblings = vec![plain.clone(), other.clone(), content.clone(), clash];
        for skipped in [&plain, &other, &content, &gated, &sanitized] {
            assert_eq!(
                text_value_companion(&siblings, skipped, &names()),
                None,
                "{}",
                skipped.name
            );
        }
    }
}
