use super::*;

pub(super) fn collect<'a>(
    input: &serde_json::Map<String, serde_json::Value>,
    owner: Option<&'a TypeDef>,
) -> Option<(
    &'a crate::core::ir::FieldDef,
    serde_json::Map<String, serde_json::Value>,
)> {
    let owner = owner?;
    let mut flattened = owner.fields.iter().filter(|field| field.serde_flatten);
    let field = flattened.next()?;
    let object_field =
        matches!(&field.ty, TypeRef::Json) || matches!(&field.ty, TypeRef::Map(key, _) if **key == TypeRef::String);
    if flattened.next().is_some() || !object_field {
        return None;
    }
    let values = input
        .iter()
        .filter(|(key, _)| resolve_owner_field(Some(owner), key).is_none_or(|field| field.serde_flatten))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some((field, values))
}

pub(super) fn expression(values: &serde_json::Map<String, serde_json::Value>) -> String {
    // ~keep JSON.parse preserves arbitrary map keys, including __proto__, without object-literal prototype semantics.
    let json = serde_json::to_string(values).expect("fixture map serializes as JSON");
    let quoted = serde_json::to_string(&json).expect("JSON source serializes as a string");
    format!("JSON.parse({quoted})")
}
