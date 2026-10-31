use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::field_access::{PhpGetterMap, SwiftFirstClassMap};

fn fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "reuse", "description": "Show declared result fields", "input": {},
        "docs": {"topic": "reuse", "shows": ["title"]}
    }))
    .expect("fixture")
}

fn config() -> E2eConfig {
    serde_json::from_value(serde_json::json!({"call": {"function": "convert", "result_var": "result"}}))
        .expect("configuration")
}

fn types() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Result".into(),
        fields: vec![FieldDef {
            name: "title".into(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

fn enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: "Kind".into(),
        variants: vec![EnumVariant {
            name: "Known".into(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }]
}

fn functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".into(),
        return_type: TypeRef::Named("Result".into()),
        ..FunctionDef::default()
    }]
}

#[test]
fn ordinary_snippets_construct_result_metadata_once() {
    let types = types();
    let functions = functions();
    let enums = enums();
    for language in ["rust", "python", "node", "go"] {
        let factory = ResolverFactory::new(&types, &enums, &functions);
        let operations = resolve_from_factory(&fixture(), &config(), language, &factory);
        assert_eq!(operations.len(), 1, "{language}");
        assert_eq!(factory.root_constructions.get(), 1, "{language}");
    }
}

#[test]
fn ordinary_operations_are_byte_identical_to_the_previous_double_anchor_route() {
    let types = types();
    let functions = functions();
    let enums = enums();
    for language in ["rust", "python", "node", "go"] {
        let factory = ResolverFactory::new(&types, &enums, &functions);
        let config = config();
        let resolver = factory.build(&config, &config.call, language);
        let previous = resolve_with_factory(&fixture(), &config, language, &resolver, &factory);
        assert_eq!(factory.root_constructions.get(), 2, "previous route: {language}");
        let current = resolve(&fixture(), &config, language, &types, &enums, &functions);
        assert_eq!(
            serde_json::to_vec(&current).unwrap(),
            serde_json::to_vec(&previous).unwrap()
        );
        assert_eq!(render_body("rust", &current), render_body("rust", &previous));
        assert!(render_body("rust", &current).contains(&current[0].expression));
    }
}

#[test]
fn caller_supplied_resolvers_still_construct_anchored_metadata() {
    let types = types();
    let functions = functions();
    let enums = enums();
    let config = config();
    for language in ["php", "swift"] {
        let factory = ResolverFactory::new(&types, &enums, &functions);
        let resolver = FieldResolver::new(
            config.effective_fields(&config.call),
            config.effective_fields_optional(&config.call),
            config.effective_result_fields(&config.call),
            config.effective_fields_array(&config.call),
            config.effective_fields_method_calls(&config.call),
        );
        let resolver = match language {
            "php" => resolver.with_php_getter_map(PhpGetterMap {
                getters: [("Result".into(), ["title".into()].into())].into(),
                all_fields: [("Result".into(), ["title".into()].into())].into(),
                root_type: Some("Result".into()),
                ..PhpGetterMap::default()
            }),
            _ => resolver.with_swift_first_class_map(SwiftFirstClassMap {
                first_class_types: ["Result".into()].into(),
                root_type: Some("Result".into()),
                ..SwiftFirstClassMap::default()
            }),
        };
        let operations = resolve_with_factory(&fixture(), &config, language, &resolver, &factory);
        assert_eq!(operations.len(), 1, "{language}");
        let expected = if language == "php" {
            "$result->getTitle()"
        } else {
            "result.title"
        };
        assert_eq!(operations[0].expression, expected);
        let previous =
            anchor_to_declared_result_type(resolver.clone(), &config.call, language, &types, &enums, &functions);
        let previous = resolve_with_factory(
            &fixture(),
            &config,
            language,
            &previous,
            &ResolverFactory::new(&types, &enums, &functions),
        );
        assert_eq!(render_body(language, &operations), render_body(language, &previous));
        assert_eq!(factory.root_constructions.get(), 1, "{language}");
    }
}

fn render_body(language: &str, presentation: &[PresentationOperation]) -> String {
    let template = match language {
        "php" => "php/snippet_body.jinja",
        "swift" => "swift/snippet_body.jinja",
        _ => "rust/snippet_body.rs.jinja",
    };
    let body = if language == "swift" {
        serde_json::json!("let result = convert()")
    } else {
        serde_json::json!(["let result = convert();"])
    };
    crate::e2e::template_env::render(
        template,
        crate::alef_context! {
            imports => Vec::<String>::new(), body => body, is_async => false,
            presentation => presentation, display_result => false, result_var => "result",
            expects_error => false, returns_void => false, module => "Sample", needs_foundation => false,
            stream_item => "", namespace => "Sample", class_name => "Client", client_factory => "",
            imported_types => Vec::<String>::new(), setup_lines => Vec::<String>::new(),
            call_expr => "convert()", is_streaming => false,
        },
    )
}

fn rich_types() -> Vec<TypeDef> {
    let mut types = types();
    types[0].fields[0].optional = true;
    types[0].fields[0].ty = TypeRef::Optional(Box::new(TypeRef::String));
    types[0].fields.extend([
        FieldDef {
            name: "kind".into(),
            ty: TypeRef::Named("Kind".into()),
            ..FieldDef::default()
        },
        FieldDef {
            name: "items".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
            ..FieldDef::default()
        },
    ]);
    types.push(TypeDef {
        name: "Item".into(),
        fields: vec![FieldDef {
            name: "title".into(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    });
    types
}

fn rich_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "rich", "description": "Show optional, enum and collection fields", "input": {},
        "docs": {"topic": "reuse", "shows": ["title", "kind"], "presentation": {"operations": [
            {"op": "iterate", "path": "items", "item": "item", "fields": ["title"]}
        ]}}
    }))
    .expect("rich fixture")
}

#[test]
fn optional_enum_and_collection_operations_preserve_roots_and_output() {
    let types = rich_types();
    let functions = functions();
    let enums = enums();
    let config = config();
    let fixture = rich_fixture();
    let factory = ResolverFactory::new(&types, &enums, &functions);
    let current = resolve_from_factory(&fixture, &config, "rust", &factory);
    assert_eq!(factory.root_constructions.get(), 1);
    assert_eq!(current.len(), 3);
    assert_eq!(
        current.iter().map(|op| op.expression.as_str()).collect::<Vec<_>>(),
        ["result.title", "result.kind", "result.items"]
    );
    assert!(current[0].shown_optional);
    assert_eq!(current[2].kind, "iterate");
    assert_eq!(current[2].fields, ["item.title"]);
    let resolver = factory.build(&config, &config.call, "rust");
    assert!(resolver.is_enum("kind"));
    let previous = resolve_with_factory(&fixture, &config, "rust", &resolver, &factory);
    assert_eq!(current, previous);
    assert_eq!(previous[2].field_optionals, [true]);
    assert_eq!(render_body("rust", &current), render_body("rust", &previous));
}
