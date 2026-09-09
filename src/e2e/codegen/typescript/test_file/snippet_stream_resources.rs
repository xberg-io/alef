use crate::codegen::naming::to_node_name;
use crate::core::ir::{TypeDef, TypeRef};
use crate::e2e::codegen::presentation::PresentationOperation;

#[derive(Debug, serde::Serialize)]
pub(crate) struct OwnedGetter {
    pub(crate) binding: String,
    pub(crate) initializer: String,
    pub(crate) release: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ShowResources {
    pub(crate) expression: String,
    pub(crate) resources: Vec<OwnedGetter>,
}

pub(crate) fn item_binding(
    fixture: &crate::e2e::fixture::Fixture,
    config: &crate::e2e::config::E2eConfig,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
) -> Option<String> {
    crate::e2e::codegen::presentation::stream_item_binding(fixture, config, language).or_else(|| {
        (!call.returns_void
            && fixture
                .assertions
                .iter()
                .any(|assertion| assertion.assertion_type == "error")
            && crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled()))
        .then(|| format!("{}Chunk", call.effective_result_var()))
    })
}

pub(crate) fn item_type<'a>(
    call: &'a crate::core::config::e2e::CallConfig,
    config: &'a crate::core::config::ResolvedCrateConfig,
    function_name: &str,
    types: &[TypeDef],
) -> Option<&'a str> {
    let core_name = call.core_lookup_name("wasm");
    let names = [function_name, core_name.as_deref().unwrap_or(function_name)];
    crate::e2e::codegen::recipe::streaming_item_type(call, &config.adapters, &names)
        .filter(|name| types.iter().any(|ty| ty.name == *name))
}

pub(crate) fn plans(
    operations: &[PresentationOperation],
    item_type: &str,
    types: &[TypeDef],
    root: &str,
) -> Vec<ShowResources> {
    operations
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            let fallback = || ShowResources {
                expression: operation.expression.clone(),
                resources: Vec::new(),
            };
            if operation.kind != "show" || !operation.guard_condition.is_empty() {
                return fallback();
            }
            plan(&operation.expression, item_type, types, root, index).unwrap_or_else(fallback)
        })
        .collect()
}

fn plan(expression: &str, item_type: &str, types: &[TypeDef], root: &str, index: usize) -> Option<ShowResources> {
    let path = expression.replace("?.", ".");
    let parts: Vec<_> = path.split('.').collect();
    if parts.len() < 3 || parts[0] != root || !parts.iter().all(|part| is_identifier(part)) {
        return None;
    }
    let mut owner = types.iter().find(|ty| ty.name == item_type)?;
    let mut resources = Vec::new();
    let mut receiver = root.to_string();
    let mut optional = false;
    for (depth, field_name) in parts[1..parts.len() - 1].iter().enumerate() {
        let field = owner
            .fields
            .iter()
            .find(|field| to_node_name(&field.name) == *field_name)?;
        let (name, field_optional) = wrapped_type(&field.ty)?;
        owner = types.iter().find(|ty| ty.name == name)?;
        let binding = format!("{root}Show{index}Resource{depth}");
        let separator = if optional { "?." } else { "." };
        let initializer = format!("{receiver}{separator}{field_name}");
        optional |= field_optional || field.optional;
        let separator = if optional { "?." } else { "." };
        resources.push(OwnedGetter {
            release: format!("{binding}{separator}free();"),
            binding: binding.clone(),
            initializer,
        });
        receiver = binding;
    }
    let separator = if optional { "?." } else { "." };
    Some(ShowResources {
        expression: format!("{receiver}{separator}{}", parts.last()?),
        resources,
    })
}

fn wrapped_type(ty: &TypeRef) -> Option<(&str, bool)> {
    match ty {
        TypeRef::Named(name) => Some((name, false)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some((name, true)),
            _ => None,
        },
        _ => None,
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || matches!(character, '_' | '$'))
        && chars.all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeRef};

    fn operation(expression: &str) -> PresentationOperation {
        PresentationOperation {
            kind: "show",
            expression: expression.into(),
            item: String::new(),
            fields: vec![],
            optional: false,
            display: false,
            destructure_source: String::new(),
            destructure_item: String::new(),
            shown_optional: true,
            field_optionals: vec![],
            field_displays: vec![],
            guard_binding: String::new(),
            guard_source: String::new(),
            guard_condition: String::new(),
        }
    }

    fn types() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "Chunk".into(),
                fields: vec![FieldDef {
                    name: "usage".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("Usage".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Usage".into(),
                fields: vec![FieldDef {
                    name: "details".into(),
                    ty: TypeRef::Named("Details".into()),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Details".into(),
                ..TypeDef::default()
            },
        ]
    }

    #[test]
    fn optional_owned_getter_is_acquired_once_and_released() {
        let result = plans(
            &[operation("responseChunk.usage?.totalTokens")],
            "Chunk",
            &types(),
            "responseChunk",
        );
        assert_eq!(result[0].expression, "responseChunkShow0Resource0?.totalTokens");
        assert_eq!(result[0].resources.len(), 1);
        let resource = &result[0].resources[0];
        assert_eq!(resource.initializer, "responseChunk.usage");
        assert_eq!(resource.release, "responseChunkShow0Resource0?.free();");
    }

    #[test]
    fn nested_owned_getters_propagate_optional_receiver_and_keep_acquisition_order() {
        let result = plans(
            &[operation("responseChunk.usage?.details.totalTokens")],
            "Chunk",
            &types(),
            "responseChunk",
        );
        assert_eq!(result[0].expression, "responseChunkShow0Resource1?.totalTokens");
        assert_eq!(result[0].resources.len(), 2);
        assert_eq!(
            result[0].resources[1].initializer,
            "responseChunkShow0Resource0?.details"
        );
        let releases: Vec<_> = result[0]
            .resources
            .iter()
            .rev()
            .map(|value| value.release.as_str())
            .collect();
        assert_eq!(
            releases,
            [
                "responseChunkShow0Resource1?.free();",
                "responseChunkShow0Resource0?.free();"
            ]
        );
    }

    #[test]
    fn arrays_unknown_paths_and_guarded_operations_remain_unchanged() {
        let mut guarded = operation("responseChunk.usage?.totalTokens");
        guarded.guard_condition = "someGuard".into();
        let inputs = [
            operation("responseChunk.choices[0].text"),
            operation("responseChunk.unknown.value"),
            guarded,
        ];
        let result = plans(&inputs, "Chunk", &types(), "responseChunk");
        for (original, actual) in inputs.iter().zip(result) {
            assert_eq!(actual.expression, original.expression);
            assert!(actual.resources.is_empty());
        }
    }
}
