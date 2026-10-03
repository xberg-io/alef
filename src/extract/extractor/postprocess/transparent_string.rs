use crate::core::ir::{NewtypeConversion, NewtypeWrapperMetadata, ReceiverKind, TypeDef, TypeRef};

pub(super) fn validate_transparent_string_methods(
    typ: &TypeDef,
    metadata: &[NewtypeWrapperMetadata],
) -> Result<(), String> {
    let [metadata] = metadata else {
        return Err("transparent_string wrapper metadata must contain exactly one conversion path".to_string());
    };
    let NewtypeConversion::TransparentString { from, into } = &metadata.conversion else {
        return Err("transparent_string wrapper metadata has the wrong conversion kind".to_string());
    };
    validate_from_method(typ, from)?;
    validate_into_method(typ, into)
}

fn validate_from_method(typ: &TypeDef, from: &str) -> Result<(), String> {
    let methods: Vec<_> = typ.methods.iter().filter(|method| method.name == from).collect();
    if methods.len() != 1 {
        return Err(format!(
            "transparent_string requires exactly one public `{from}(String) -> Self` method"
        ));
    }
    let method = methods[0];
    let returns_self = matches!(&method.return_type, TypeRef::Named(name) if name == "Self" || name == &typ.name);
    if method.is_static
        && !method.is_async
        && method.error_type.is_none()
        && method.params.len() == 1
        && !method.params[0].optional
        && method.params[0].ty == TypeRef::String
        && returns_self
        && method.trait_source.is_none()
        && !method.binding_excluded
    {
        return Ok(());
    }
    Err(format!(
        "transparent_string `{from}` must be a public synchronous `fn(String) -> Self`"
    ))
}

fn validate_into_method(typ: &TypeDef, into: &str) -> Result<(), String> {
    let methods: Vec<_> = typ.methods.iter().filter(|method| method.name == into).collect();
    if methods.len() != 1 {
        return Err(format!(
            "transparent_string requires exactly one public `{into}(self) -> String` method"
        ));
    }
    let method = methods[0];
    if !method.is_static
        && !method.is_async
        && method.error_type.is_none()
        && method.receiver == Some(ReceiverKind::Owned)
        && method.params.is_empty()
        && method.return_type == TypeRef::String
        && method.trait_source.is_none()
        && !method.binding_excluded
    {
        return Ok(());
    }
    Err(format!(
        "transparent_string `{into}` must be a public synchronous `fn(self) -> String`"
    ))
}
