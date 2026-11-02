use super::{ResolvedNewtype, resolve_typeref};
use crate::core::ir::{NewtypeContainer, NewtypeConversion, NewtypeWrapperMetadata, ParamDef, TypeRef};
use ahash::AHashMap;

pub(super) fn resolve_function_typeref(
    newtype_map: &AHashMap<String, (TypeRef, ResolvedNewtype)>,
    ty: &mut TypeRef,
) -> ResolvedNewtype {
    match ty {
        TypeRef::Optional(inner) => resolve_function_typeref(newtype_map, inner)
            .promote_legacy()
            .prepend(NewtypeContainer::Optional),
        TypeRef::Vec(inner) => resolve_function_typeref(newtype_map, inner)
            .promote_legacy()
            .prepend(NewtypeContainer::Vec),
        TypeRef::Map(key, value) => {
            let key = resolve_function_typeref(newtype_map, key)
                .promote_legacy()
                .prepend(NewtypeContainer::MapKey);
            let value = resolve_function_typeref(newtype_map, value)
                .promote_legacy()
                .prepend(NewtypeContainer::MapValue);
            key.merge(value)
        }
        _ => resolve_typeref(newtype_map, ty, false),
    }
}

impl ResolvedNewtype {
    pub(super) fn promote_legacy(mut self) -> Self {
        if let Some(rust_path) = self.legacy.take() {
            self.explicit.push(NewtypeWrapperMetadata {
                rust_path,
                conversion: NewtypeConversion::TupleField,
                containers: vec![],
            });
        }
        self
    }
}

pub(super) fn resolve_parameter(
    newtype_map: &AHashMap<String, (TypeRef, ResolvedNewtype)>,
    param: &mut ParamDef,
) -> Option<String> {
    let mut resolved = resolve_function_typeref(newtype_map, &mut param.ty);
    if param.optional {
        resolved = resolved.promote_legacy();
    }
    resolved.prepend_outer_optional(param.optional);
    resolved.encode()
}
