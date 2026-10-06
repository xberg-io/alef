//! Shared helpers for WASM type generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, TypeRef};
use ahash::AHashSet;

/// Whether a transparent-wrapper field needs an opaque serde bridge at the WASM boundary.
/// Scalar, one-option, and flat-vector strings have native wasm-bindgen shapes. Maps and deeper
/// container paths do not, so they cross as `JsValue` after unwrapping the wrapper leaves. ~keep
pub(in crate::backends::wasm::gen_bindings) fn complex_newtype_field_uses_jsvalue(field: &FieldDef) -> bool {
    let Some(wrapper) = field.newtype_wrapper.as_deref() else {
        return false;
    };
    complex_newtype_wrapper_uses_jsvalue(wrapper)
}

pub(in crate::backends::wasm::gen_bindings) fn complex_newtype_wrapper_uses_jsvalue(wrapper: &str) -> bool {
    let Ok(decoded) = crate::core::ir::NewtypeWrapper::decode(wrapper) else {
        return false;
    };
    decoded.explicit_paths().iter().any(|metadata| {
        metadata.containers.len() > 1
            || metadata.containers.iter().any(|container| {
                matches!(
                    container,
                    crate::core::ir::NewtypeContainer::MapKey | crate::core::ir::NewtypeContainer::MapValue
                )
            })
    })
}

/// Reconstruct an explicit transparent-string wrapper for a WASM input.
///
/// A single root `Optional` path can pass its configured constructor directly to `Option::map`.
/// Nested and multi-path metadata stays on the shared recursive converter because each inner
/// container still needs its own traversal. ~keep
pub(in crate::backends::wasm::gen_bindings) fn apply_newtype_to_core(
    expression: &str,
    ty: &TypeRef,
    optional: bool,
    wrapper: &str,
) -> String {
    if (optional || matches!(ty, TypeRef::Optional(_)))
        && let Some(constructor) = root_optional_transparent_constructor(wrapper)
    {
        return format!("({expression}).map({constructor})");
    }
    crate::codegen::conversions::helpers::apply_field_newtype_to_core(expression, ty, optional, wrapper)
}

pub(in crate::backends::wasm::gen_bindings) fn root_optional_transparent_constructor(wrapper: &str) -> Option<String> {
    let crate::core::ir::NewtypeWrapper::Explicit(paths) = crate::core::ir::NewtypeWrapper::decode(wrapper).ok()?
    else {
        return None;
    };
    let [metadata] = paths.as_slice() else {
        return None;
    };
    if metadata.containers.as_slice() != [crate::core::ir::NewtypeContainer::Optional] {
        return None;
    }
    let crate::core::ir::NewtypeConversion::TransparentString { from, .. } = &metadata.conversion else {
        return None;
    };
    Some(format!("{}::{from}", metadata.rust_path))
}

/// Return a core-to-binding-only field view that avoids the shared optional flatten fallback.
///
/// `FieldDef::optional` and a leading `TypeRef::Optional` are distinct real option layers. The
/// explicit metadata describes both, and the WASM explicit-newtype branch converts both before
/// serializing the inner value. The shared renderer nevertheless follows that correct branch
/// with a generic `.flatten()` rewrite whenever both markers are present, replacing the explicit
/// conversion entirely. Hiding the inner structural marker from shape dispatch prevents only
/// that late rewrite; the emitted expression still reads the original core field and follows the
/// untouched explicit path metadata. Binding-to-core must use the original IR. ~keep
pub(in crate::backends::wasm::gen_bindings) fn suppress_explicit_newtype_flatten_for_core_to_binding(
    typ: &crate::core::ir::TypeDef,
) -> crate::core::ir::TypeDef {
    let mut conversion_view = typ.clone();
    for field in &mut conversion_view.fields {
        if !field.optional
            || !field
                .newtype_wrapper
                .as_deref()
                .is_some_and(crate::codegen::conversions::helpers::is_explicit_newtype)
        {
            continue;
        }
        if let TypeRef::Optional(inner) = &field.ty {
            field.ty = (**inner).clone();
        }
    }
    conversion_view
}

/// Return a WASM binding surface whose struct fields and methods match the backend feature set.
///
/// The extractor can retain a cfg-gated field or method when the source crate was extracted with
/// a broader feature set than the WASM binding crate uses. Downstream struct, method and
/// conversion generators expect one coherent list, so drop inactive members and clear cfg markers
/// from active ones before generating WASM bindings.
///
/// Methods matter as much as fields here: `wasm_bindgen` emits one JS-visible export per method
/// with no Rust-cfg gating of its own, so a method left in place under a feature the binding
/// crate does not enable becomes a call into a core method that is not compiled. ~keep
pub(in crate::backends::wasm::gen_bindings) fn filter_cfg_fields_for_features(
    api: &ApiSurface,
    enabled_features: &[String],
) -> ApiSurface {
    let mut filtered = api.clone();
    for typ in &mut filtered.types {
        let mut fields = Vec::with_capacity(typ.fields.len());

        for mut field in std::mem::take(&mut typ.fields) {
            let Some(cfg) = field.cfg.as_deref() else {
                fields.push(field);
                continue;
            };

            if super::super::cfg::cfg_condition_enabled(cfg, enabled_features) {
                field.cfg = None;
                fields.push(field);
            }
        }

        typ.fields = fields;
        typ.methods = retain_enabled_methods(std::mem::take(&mut typ.methods), enabled_features);
    }
    for enum_def in &mut filtered.enums {
        enum_def.methods = retain_enabled_methods(std::mem::take(&mut enum_def.methods), enabled_features);
    }
    for error in &mut filtered.errors {
        error.methods = retain_enabled_methods(std::mem::take(&mut error.methods), enabled_features);
    }
    filtered
}

fn retain_enabled_methods(methods: Vec<MethodDef>, enabled_features: &[String]) -> Vec<MethodDef> {
    methods
        .into_iter()
        .filter_map(|mut method| {
            let Some(cfg) = method.cfg.as_deref() else {
                return Some(method);
            };
            if !super::super::cfg::cfg_condition_enabled(cfg, enabled_features) {
                return None;
            }
            method.cfg = None;
            Some(method)
        })
        .collect()
}

/// Returns `true` when `ty` is `Vec<Named>` where `Named` is a tagged-data enum.
pub(super) fn is_vec_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Returns `true` when `ty` is a bare `Named` that is a tagged-data enum.
pub(super) fn is_bare_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(n) if tagged_data_enum_names.contains(n))
}

/// Returns `true` when `ty` is `Option<Named>` where `Named` is a tagged-data enum.
pub(super) fn is_option_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Check if a TypeRef is a Copy type that shouldn't be cloned.
pub(super) fn is_copy_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) => true,
        TypeRef::Duration => true,
        TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path | TypeRef::Json => false,
        TypeRef::Optional(inner) => is_copy_type(inner, enum_names),
        TypeRef::Vec(_) | TypeRef::Map(_, _) => false,
        TypeRef::Named(n) => enum_names.contains(n),
        TypeRef::Unit => true,
    }
}

/// Extract the inner type of an `Optional` wrapper, or return the type itself.
pub(super) fn optional_inner(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

/// The generated wasm-bindgen class a field is stored as, when it is stored as one.
///
/// Returns the mapped `{prefix}{Name}` class for a field typed `Named` or `Option<Named>` whose
/// name the backend emits as a `#[wasm_bindgen]` struct. Returns `None` when a `type_overrides`
/// entry redirects the name somewhere else (`JsValue`, `String`, ...): the accessor then has to
/// keep using the mapper's rendering, because no such class exists to borrow. ~keep
pub(in crate::backends::wasm::gen_bindings) fn class_backed_field_type(
    field: &FieldDef,
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
) -> Option<String> {
    let TypeRef::Named(name) = optional_inner(&field.ty) else {
        return None;
    };
    if !class_type_names.contains(name.as_str()) {
        return None;
    }
    let mapped = mapper.named(name).into_owned();
    (mapped == format!("{}{name}", mapper.prefix)).then_some(mapped)
}

/// The generated wasm-bindgen class stored in each element of a `Vec`, when the field is a
/// `Vec<Named>` or `Option<Vec<Named>>` and the name was not redirected by a type override.
/// ~keep
pub(in crate::backends::wasm::gen_bindings) fn class_backed_vec_element_type(
    field: &FieldDef,
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
) -> Option<String> {
    let ty = optional_inner(&field.ty);
    let TypeRef::Vec(inner) = ty else {
        return None;
    };
    let TypeRef::Named(name) = inner.as_ref() else {
        return None;
    };
    if !class_type_names.contains(name.as_str()) {
        return None;
    }
    let mapped = mapper.named(name).into_owned();
    (mapped == format!("{}{name}", mapper.prefix)).then_some(mapped)
}

#[cfg(test)]
mod tests {
    use super::{apply_newtype_to_core, suppress_explicit_newtype_flatten_for_core_to_binding};
    use crate::codegen::conversions::{ConversionConfig, gen_from_binding_to_core_cfg, gen_from_core_to_binding_cfg};
    use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, TypeDef, TypeRef};
    use ahash::{AHashMap, AHashSet};

    #[test]
    fn root_optional_constructor_item_does_not_collapse_nested_or_multi_path_metadata() {
        let root = NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "fixture::Credential",
            "new_secret",
            "expose_secret",
            vec![NewtypeContainer::Optional],
        )]);
        let nested = NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "fixture::Credential",
            "new_secret",
            "expose_secret",
            vec![NewtypeContainer::Optional, NewtypeContainer::Optional],
        )]);
        let multi_path = NewtypeWrapper::encode_explicit(&[
            NewtypeWrapperMetadata::transparent_string(
                "fixture::MapKey",
                "new_key",
                "expose_key",
                vec![NewtypeContainer::Optional, NewtypeContainer::MapKey],
            ),
            NewtypeWrapperMetadata::transparent_string(
                "fixture::MapValue",
                "new_value",
                "expose_value",
                vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
            ),
        ]);

        assert_eq!(
            apply_newtype_to_core("value", &TypeRef::String, true, &root),
            "(value).map(fixture::Credential::new_secret)"
        );
        let nested_out = apply_newtype_to_core("value", &TypeRef::Optional(Box::new(TypeRef::String)), true, &nested);
        assert!(nested_out.contains("map(|value|"), "{nested_out}");
        assert!(
            nested_out.contains("fixture::Credential::new_secret(value)"),
            "{nested_out}"
        );

        let map_ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let multi_path_out = apply_newtype_to_core("value", &map_ty, true, &multi_path);
        assert!(multi_path_out.contains("map(|value|"), "{multi_path_out}");
        assert!(
            multi_path_out.contains("fixture::MapKey::new_key(key)"),
            "{multi_path_out}"
        );
        assert!(
            multi_path_out.contains("fixture::MapValue::new_value(value)"),
            "{multi_path_out}"
        );
    }

    fn nested_optional_explicit_type() -> TypeDef {
        let credential_wrapper = NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "fixture::Credential",
            "from_secret",
            "expose_secret",
            vec![NewtypeContainer::Optional, NewtypeContainer::Optional],
        )]);
        let segment_key_wrapper = NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "fixture::SecretString",
            "from_key",
            "expose_key",
            vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Vec,
                NewtypeContainer::MapKey,
            ],
        )]);
        TypeDef {
            name: "Options".to_string(),
            rust_path: "fixture::Options".to_string(),
            has_default: true,
            fields: vec![
                FieldDef {
                    name: "nested_optional_credential".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    newtype_wrapper: Some(credential_wrapper),
                    ..Default::default()
                },
                FieldDef {
                    name: "nested_optional_segments".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Named("Segment".to_string())),
                    ))),
                    optional: true,
                    newtype_wrapper: Some(segment_key_wrapper),
                    ..Default::default()
                },
                FieldDef {
                    name: "excluded_secret".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    newtype_wrapper: Some(NewtypeWrapper::encode_explicit(&[
                        NewtypeWrapperMetadata::transparent_string(
                            "fixture::Credential",
                            "from_excluded",
                            "expose_excluded",
                            vec![NewtypeContainer::Optional, NewtypeContainer::Optional],
                        ),
                    ])),
                    binding_excluded: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn explicit_nested_optional_conversions_keep_configured_method_names() {
        let typ = nested_optional_explicit_type();
        let core_to_binding_typ = suppress_explicit_newtype_flatten_for_core_to_binding(&typ);
        let type_paths = AHashMap::from_iter([("Segment".to_string(), "fixture::Segment".to_string())]);
        let config = ConversionConfig {
            type_name_prefix: "Wasm",
            map_uses_jsvalue: true,
            wasm_explicit_newtype_containers_use_jsvalue: true,
            core_type_paths: Some(&type_paths),
            ..Default::default()
        };

        let to_binding = gen_from_core_to_binding_cfg(&core_to_binding_typ, "fixture", &AHashSet::new(), &config);
        let to_core = gen_from_binding_to_core_cfg(&typ, "fixture", &config);

        assert!(to_binding.contains("expose_secret()"), "{to_binding}");
        assert!(to_binding.contains("expose_key()"), "{to_binding}");
        assert!(!to_binding.contains(".flatten()"), "{to_binding}");
        assert!(!to_binding.contains("v.to_string()"), "{to_binding}");
        assert!(to_core.contains("fixture::Credential::from_secret(value)"), "{to_core}");
        assert!(to_core.contains("fixture::SecretString::from_key(key)"), "{to_core}");
        assert!(
            to_core.contains("Vec<std::collections::HashMap<String, fixture::Segment>>"),
            "named values must cross the opaque boundary through their resolved core type: {to_core}"
        );
        assert!(!to_binding.contains("excluded_secret"), "{to_binding}");
        assert!(!to_core.contains("excluded_secret"), "{to_core}");
    }
}
