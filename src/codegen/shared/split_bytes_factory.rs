use super::{TypeDef, TypeRef, split_bytes_field};

/// Select a DTO whose factory result can be separated into metadata and raw bytes.
pub(crate) fn split_bytes_factory_field<'a>(
    typ: &'a TypeDef,
    api: &crate::core::ir::ApiSurface,
    prefix: &str,
) -> Option<&'a crate::core::ir::FieldDef> {
    let source_type = api.types.iter().find(|source_type| source_type.name == typ.name)?;
    let field = split_bytes_field(typ)?;
    if typ.is_trait
        || split_bytes_field(source_type)?.name != field.name
        || field.is_boxed
        || field.sanitized
        || field.newtype_wrapper.is_some()
        || field.core_wrapper != crate::core::ir::CoreWrapper::None
        || field.serde_with.is_some()
        || field.serde_flatten
        || source_type
            .fields
            .iter()
            .any(|field| field.name == "alef_take_factory_bytes")
        || !source_type
            .methods
            .iter()
            .any(|method| is_split_bytes_factory(source_type, method))
        || !typ.methods.iter().any(|method| is_split_bytes_factory(typ, method))
    {
        return None;
    }
    let symbol = crate::codegen::c_consumer::method_symbol(prefix, &typ.name, "alef_take_factory_bytes");
    let free_bytes = crate::codegen::c_consumer::free_function_symbol(prefix, "free_bytes");
    (!has_public_callable_symbol(api, prefix, &symbol) && !has_public_callable_symbol(api, prefix, &free_bytes))
        .then_some(field)
}

pub(crate) fn is_split_bytes_factory(typ: &TypeDef, method: &crate::core::ir::MethodDef) -> bool {
    method.name == "from_bytes"
        && !method.is_async
        && !method.returns_ref
        && method.is_static
        && method.receiver.is_none()
        && method.cfg.is_none()
        && !method.sanitized
        && !method.binding_excluded
        && method.return_type == TypeRef::Named(typ.name.clone())
        && method.params.iter().any(|param| param.ty == TypeRef::Bytes)
}

pub(super) fn has_public_callable_symbol(api: &crate::core::ir::ApiSurface, prefix: &str, symbol: &str) -> bool {
    use crate::codegen::c_consumer::{free_function_symbol, method_symbol};
    api.functions
        .iter()
        .any(|func| free_function_symbol(prefix, &func.name) == symbol)
        || api.types.iter().any(|typ| {
            typ.methods
                .iter()
                .any(|method| method_symbol(prefix, &typ.name, &method.name) == symbol)
        })
}
