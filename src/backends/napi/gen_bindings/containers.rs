//! NAPI-only container policy without extending shared conversion configuration.

use super::transparent_newtypes::{self, Direction};
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{ApiSurface, FieldDef, ParamDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use quote::ToTokens;

pub(in crate::backends::napi) struct ContainerContext {
    enum_keys: AHashMap<String, String>,
    fallible_types: AHashSet<String>,
    type_paths: AHashMap<String, String>,
}

impl ContainerContext {
    pub(in crate::backends::napi) fn new(api: &ApiSurface, core_import: &str) -> Self {
        let mut context = Self {
            type_paths: crate::codegen::conversions::build_type_path_map(api, core_import),
            enum_keys: api
                .enums
                .iter()
                .map(|definition| {
                    (
                        definition.name.clone(),
                        crate::codegen::conversions::core_enum_path_remapped(definition, core_import, &[]),
                    )
                })
                .collect(),
            fallible_types: api
                .enums
                .iter()
                .filter(|definition| {
                    super::enums::is_json_passthrough_data_enum(definition, &api.types)
                        && !super::enums::is_tagged_data_enum(definition, &api.types)
                })
                .map(|definition| definition.name.clone())
                .collect(),
        };
        loop {
            let previous = context.fallible_types.len();
            for typ in api.types.iter().filter(|typ| !typ.is_opaque && !typ.is_trait) {
                if typ
                    .fields
                    .iter()
                    .filter(|field| !field.binding_excluded)
                    .any(|field| context.is_fallible(&field.ty))
                {
                    context.fallible_types.insert(typ.name.clone());
                }
            }
            for definition in &api.enums {
                if definition
                    .variants
                    .iter()
                    .flat_map(|variant| &variant.fields)
                    .any(|field| context.is_fallible(&field.ty))
                {
                    context.fallible_types.insert(definition.name.clone());
                }
            }
            if previous == context.fallible_types.len() {
                break;
            }
        }
        context
    }

    pub(in crate::backends::napi) fn is_fallible(&self, ty: &TypeRef) -> bool {
        self.policy().is_fallible(ty)
    }

    fn policy(&self) -> transparent_newtypes::ContainerPolicy<'_> {
        transparent_newtypes::ContainerPolicy {
            enum_keys: &self.enum_keys,
            fallible_types: &self.fallible_types,
        }
    }

    pub(in crate::backends::napi) fn binding_type_ref(&self, ty: &TypeRef) -> TypeRef {
        match ty {
            TypeRef::Map(key, value) => {
                let key = if matches!(key.as_ref(), TypeRef::Named(name) if self.enum_keys.contains_key(name)) {
                    TypeRef::String
                } else {
                    self.binding_type_ref(key)
                };
                TypeRef::Map(Box::new(key), Box::new(self.binding_type_ref(value)))
            }
            TypeRef::Optional(inner) => TypeRef::Optional(Box::new(self.binding_type_ref(inner))),
            TypeRef::Vec(inner) => TypeRef::Vec(Box::new(self.binding_type_ref(inner))),
            _ => ty.clone(),
        }
    }

    pub(super) fn binding_type(&self, typ: &TypeDef) -> TypeDef {
        let mut binding = typ.clone();
        for field in &mut binding.fields {
            field.ty = self.binding_type_ref(&field.ty);
        }
        binding
    }

    pub(in crate::backends::napi) fn input_expression(&self, parameter: &ParamDef, optional_default: bool) -> String {
        let ty = if parameter.optional {
            TypeRef::Optional(Box::new(parameter.ty.clone()))
        } else {
            parameter.ty.clone()
        };
        use transparent_newtypes::ConversionExpression;

        let policy = self.policy();
        let converted = if optional_default {
            let result = transparent_newtypes::convert_expression("value", &ty, &[], Direction::ToCore, Some(&policy))
                .into_result();
            ConversionExpression::Value(format!(
                "({}).map(|value| -> std::result::Result<_, serde_json::Error> {{ {result} }}).transpose()?.unwrap_or_default()",
                parameter.name
            ))
        } else {
            transparent_newtypes::convert_expression(&parameter.name, &ty, &[], Direction::ToCore, Some(&policy))
        };
        let converted = if let Some(wrapper) = &parameter.newtype_wrapper {
            ConversionExpression::Value(crate::codegen::conversions::helpers::apply_field_newtype_to_core(
                &converted.into_value(),
                &parameter.ty,
                parameter.optional,
                wrapper,
            ))
        } else {
            converted
        };
        let core = self.input_error_context(&parameter.ty);
        match converted {
            ConversionExpression::SerdeResult(result) => {
                format!("{result}{}?", super::inputs::invalid_serde_arg(&core))
            }
            ConversionExpression::Value(value) => format!(
                "(|| -> std::result::Result<_, serde_json::Error> {{ Ok({value}) }})(){}?",
                super::inputs::invalid_arg(&core)
            ),
        }
    }

    /// The core type a converted input has, when it can be named: an adapter body reads the
    /// converted value through `.into()`, which gives `try_into()` no target type to infer. ~keep
    pub(in crate::backends::napi) fn core_input_type(&self, parameter: &ParamDef) -> Option<String> {
        let path = match &parameter.ty {
            TypeRef::Named(name) => self.type_paths.get(name)?.clone(),
            _ => return None,
        };
        Some(if parameter.optional {
            format!("Option<{path}>")
        } else {
            path
        })
    }

    fn input_error_context(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Named(name) => self.type_paths.get(name).unwrap_or(name).clone(),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => self.input_error_context(inner),
            TypeRef::Map(key, _) if matches!(key.as_ref(), TypeRef::Named(name) if self.enum_keys.contains_key(name)) => {
                self.input_error_context(key)
            }
            TypeRef::Map(_, value) => self.input_error_context(value),
            _ => "input value".into(),
        }
    }

    pub(super) fn binding_enum(&self, definition: &crate::core::ir::EnumDef) -> crate::core::ir::EnumDef {
        let mut binding = definition.clone();
        for field in binding.variants.iter_mut().flat_map(|variant| &mut variant.fields) {
            field.ty = self.binding_type_ref(&field.ty);
        }
        binding
    }

    pub(super) fn variant_input(&self, name: &str, field: &FieldDef, mixed: bool) -> String {
        let source = if field.optional {
            format!("val.{name}")
        } else {
            format!(
                "val.{name}.ok_or_else(|| <serde_json::Error as serde::de::Error>::custom(\"missing required variant field {name}\"))?"
            )
        };
        if mixed && let TypeRef::Named(typ) = &field.ty {
            let path = self
                .type_paths
                .get(typ)
                .expect("mixed variant payload must have a core path");
            return if field.optional {
                format!("({source}).map(|value| serde_json::from_str::<{path}>(&value)).transpose()?")
            } else {
                format!("serde_json::from_str::<{path}>(&({source}))?")
            };
        }
        let ty = if field.optional {
            TypeRef::Optional(Box::new(field.ty.clone()))
        } else {
            field.ty.clone()
        };
        let converted =
            transparent_newtypes::convert_with_enum_keys(&source, &ty, &[], Direction::ToCore, Some(&self.policy()));
        wrap_core_field(converted, field)
    }

    pub(super) fn variant_output(&self, name: &str, field: &FieldDef) -> String {
        let source = unwrap_core_field(name.to_string(), field);
        let ty = if field.optional {
            TypeRef::Optional(Box::new(field.ty.clone()))
        } else {
            field.ty.clone()
        };
        let converted =
            transparent_newtypes::convert_with_enum_keys(&source, &ty, &[], Direction::FromCore, Some(&self.policy()));
        if field.optional {
            format!("{name}: {converted}")
        } else {
            format!("{name}: Some({converted})")
        }
    }

    fn needs_conversion(&self, ty: &TypeRef) -> bool {
        if self.is_fallible(ty) {
            return true;
        }
        match ty {
            TypeRef::Map(key, value) => {
                matches!(key.as_ref(), TypeRef::Named(name) if self.enum_keys.contains_key(name))
                    || self.needs_conversion(value)
            }
            TypeRef::Optional(inner) => contains_json(inner) || self.needs_conversion(inner),
            TypeRef::Vec(inner) => self.needs_conversion(inner),
            _ => false,
        }
    }

    fn needs_output_conversion(&self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Map(key, value) => {
                matches!(key.as_ref(), TypeRef::Named(name) if self.enum_keys.contains_key(name))
                    || self.needs_output_conversion(value)
            }
            TypeRef::Optional(inner) => contains_json(inner) || self.needs_output_conversion(inner),
            TypeRef::Vec(inner) => self.needs_output_conversion(inner),
            _ => false,
        }
    }

    fn expression(
        &self,
        typ: &TypeDef,
        field: &FieldDef,
        config: &ConversionConfig<'_>,
        direction: Direction,
        provided: bool,
    ) -> String {
        let binding_name = config.binding_field_name_owned(&typ.name, &field.name);
        let source_name = match direction {
            Direction::ToCore => &binding_name,
            Direction::FromCore => &field.name,
        };
        let mut source = if provided {
            "__v".to_string()
        } else {
            format!("val.{source_name}")
        };
        if matches!(direction, Direction::FromCore) {
            source = unwrap_core_field(source, field);
        }
        let nested_optional = field.optional && matches!(field.ty, TypeRef::Optional(_));
        if nested_optional && matches!(direction, Direction::FromCore) {
            source = format!("({source}).flatten()");
        }
        let binding_optional = !provided && super::types::napi_field_is_optional(field, typ);
        let missing_default = !field.optional
            && !matches!(field.ty, TypeRef::Optional(_))
            && binding_optional
            && matches!(direction, Direction::ToCore);
        if missing_default {
            source = "value".to_string();
        }
        let ty = if field.optional && !nested_optional {
            TypeRef::Optional(Box::new(field.ty.clone()))
        } else {
            field.ty.clone()
        };
        let converted =
            transparent_newtypes::convert_with_enum_keys(&source, &ty, &[], direction, Some(&self.policy()));
        let converted = finish_field_conversion(converted, field, nested_optional, binding_optional, direction);
        if missing_default {
            format!(
                "val.{binding_name}.map(|value| -> std::result::Result<_, serde_json::Error> {{ Ok({converted}) }}).transpose()?.unwrap_or_default()"
            )
        } else {
            converted
        }
    }

    pub(super) fn gen_from_binding_to_core(
        &self,
        typ: &TypeDef,
        core_import: &str,
        config: &ConversionConfig<'_>,
    ) -> String {
        let generated = transparent_newtypes::gen_from_binding_to_core(typ, core_import, config);
        let generated = self.repair(generated, typ, config, Direction::ToCore);
        if self.fallible_types.contains(&typ.name) {
            fallible_impl(generated)
        } else {
            generated
        }
    }

    pub(super) fn gen_from_core_to_binding(
        &self,
        typ: &TypeDef,
        core_import: &str,
        opaque_types: &ahash::AHashSet<String>,
        config: &ConversionConfig<'_>,
    ) -> String {
        let generated = transparent_newtypes::gen_from_core_to_binding(typ, core_import, opaque_types, config);
        self.repair(generated, typ, config, Direction::FromCore)
    }

    fn repair(
        &self,
        mut generated: String,
        typ: &TypeDef,
        config: &ConversionConfig<'_>,
        direction: Direction,
    ) -> String {
        for field in typ
            .fields
            .iter()
            .filter(|field| !field.binding_excluded && !field.sanitized)
        {
            let required = match direction {
                Direction::ToCore => self.needs_conversion(&field.ty),
                Direction::FromCore => self.needs_output_conversion(&field.ty),
            };
            if !required {
                continue;
            }
            let provided = matches!(direction, Direction::ToCore)
                && generated.contains(&format!(
                    "if let Some(__v) = val.{} {{",
                    config.binding_field_name_owned(&typ.name, &field.name)
                ))
                && generated.contains(&format!("__result.{} = ", field.name));
            let expression = self.expression(typ, field, config, direction, provided);
            let binding_name = config.binding_field_name_owned(&typ.name, &field.name);
            generated = match direction {
                Direction::ToCore => transparent_newtypes::replace_to_core_conversion(
                    &generated,
                    &field.name,
                    &binding_name,
                    &expression,
                ),
                Direction::FromCore => {
                    transparent_newtypes::replace_field_conversion(generated, &binding_name, &expression)
                }
            }
            .unwrap_or_else(|| panic!("missing NAPI container initializer for {}.{}", typ.name, field.name));
        }
        generated
    }
}

fn contains_json(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_json(inner),
        TypeRef::Map(key, value) => contains_json(key) || contains_json(value),
        _ => false,
    }
}

fn unwrap_core_field(mut source: String, field: &FieldDef) -> String {
    source = match field.core_wrapper {
        crate::core::ir::CoreWrapper::Arc => {
            if field.optional {
                format!("({source}).map(|value| (*value).clone())")
            } else {
                format!("(*{source}).clone()")
            }
        }
        crate::core::ir::CoreWrapper::ArcMutex => {
            if field.optional {
                format!("({source}).map(|value| value.lock().unwrap().clone())")
            } else {
                format!("({source}).lock().unwrap().clone()")
            }
        }
        _ => source,
    };
    if field.is_boxed {
        source = if field.optional {
            format!("({source}).map(|value| *value)")
        } else {
            format!("(*{source})")
        };
    }
    if let Some(wrapper) = &field.newtype_wrapper {
        source = crate::codegen::conversions::helpers::apply_field_newtype_from_core(
            &source,
            &field.ty,
            field.optional,
            wrapper,
        );
    }
    source
}

fn wrap_core_field(mut converted: String, field: &FieldDef) -> String {
    if field.vec_inner_core_wrapper != crate::core::ir::CoreWrapper::None {
        converted = if field.optional {
            format!(
                "({converted}).map(|values| {})",
                wrap_vector_inner("values", &field.ty, &field.vec_inner_core_wrapper)
            )
        } else {
            wrap_vector_inner(&converted, &field.ty, &field.vec_inner_core_wrapper)
        };
    }

    if let Some(wrapper) = &field.newtype_wrapper {
        converted = crate::codegen::conversions::helpers::apply_field_newtype_to_core(
            &converted,
            &field.ty,
            field.optional,
            wrapper,
        );
    }
    let constructor = match field.core_wrapper {
        crate::core::ir::CoreWrapper::Arc => Some("std::sync::Arc::new"),
        crate::core::ir::CoreWrapper::ArcMutex => Some("|value| std::sync::Arc::new(std::sync::Mutex::new(value))"),
        _ => None,
    };
    if let Some(constructor) = constructor {
        converted = if field.optional {
            format!("({converted}).map({constructor})")
        } else {
            format!("({constructor})({converted})")
        };
    }
    if field.is_boxed {
        converted = if field.optional {
            format!("({converted}).map(Box::new)")
        } else {
            format!("Box::new({converted})")
        };
    }
    converted
}

fn fallible_impl(generated: String) -> String {
    let mut implementation: syn::ItemImpl = syn::parse_str(&generated).expect("generated conversion must parse");
    let (path, _) = implementation
        .trait_
        .as_mut()
        .expect("generated conversion must implement From");
    let segment = path.segments.last_mut().expect("conversion trait must have a name");
    assert_eq!(
        segment.ident, "From",
        "only the generated input From impl is made fallible"
    );
    segment.ident = syn::Ident::new("TryFrom", segment.ident.span());
    let function = implementation
        .items
        .iter_mut()
        .find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == "from" => Some(function),
            _ => None,
        })
        .expect("generated conversion must contain its from function");
    function.sig.ident = syn::Ident::new("try_from", function.sig.ident.span());
    // ~keep Concrete error type, not `Self::Error`, which is ambiguous when `Self` has an `Error` variant.
    function.sig.output = syn::parse_quote!(-> std::result::Result<Self, serde_json::Error>);
    let body = function.block.clone();
    function.block = syn::parse_quote!({ Ok(#body) });
    implementation.items.insert(
        0,
        syn::parse_quote!(
            type Error = serde_json::Error;
        ),
    );
    implementation.to_token_stream().to_string()
}

fn wrap_vector_inner(source: &str, ty: &TypeRef, wrapper: &crate::core::ir::CoreWrapper) -> String {
    if let TypeRef::Optional(inner) = ty {
        return format!(
            "({source}).map(|values| {})",
            wrap_vector_inner("values", inner, wrapper)
        );
    }
    let constructor = match wrapper {
        crate::core::ir::CoreWrapper::Arc => "std::sync::Arc::new",
        crate::core::ir::CoreWrapper::ArcMutex => "|value| std::sync::Arc::new(std::sync::Mutex::new(value))",
        _ => return source.to_string(),
    };
    // The fallible collection needs its Vec shape before ownership wrapping removes its contextual type. ~keep
    format!("{{ let __alef_values: Vec<_> = {source}; __alef_values.into_iter().map({constructor}).collect() }}")
}

fn finish_field_conversion(
    converted: String,
    field: &FieldDef,
    nested_optional: bool,
    binding_optional: bool,
    direction: Direction,
) -> String {
    let mut converted = if nested_optional && matches!(direction, Direction::ToCore) {
        format!("({converted}).map(Some)")
    } else if !field.optional
        && !matches!(field.ty, TypeRef::Optional(_))
        && binding_optional
        && matches!(direction, Direction::FromCore)
    {
        format!("Some({converted})")
    } else {
        converted
    };
    if matches!(direction, Direction::ToCore) {
        converted = wrap_core_field(converted, field);
    }
    converted
}
