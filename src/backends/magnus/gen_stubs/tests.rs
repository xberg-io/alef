use super::gen_enum_stub;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn optional_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        optional: true,
        ..field(name, ty)
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants,
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

fn shape_enum() -> EnumDef {
    enum_def(
        "Shape",
        vec![
            variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
            variant(
                "Rect",
                vec![
                    field("width", TypeRef::Primitive(PrimitiveType::U32)),
                    field("height", TypeRef::Primitive(PrimitiveType::U32)),
                ],
            ),
        ],
    )
}

#[test]
fn emits_singleton_constructor_per_struct_variant() {
    let stub = gen_enum_stub(&shape_enum(), false);

    assert!(stub.contains("  class Shape"), "{stub}");
    assert!(stub.contains("    def self.circle: (Float radius) -> Shape"), "{stub}");
    assert!(
        stub.contains("    def self.rect: (Integer width, Integer height) -> Shape"),
        "{stub}"
    );
}

#[test]
fn tagged_data_enum_emits_no_singleton_constructors() {
    let tagged = EnumDef {
        serde_content: None,
        serde_tag: Some("type".to_string()),
        ..shape_enum()
    };
    let stub = gen_enum_stub(&tagged, false);
    assert!(stub.contains("  class Shape"), "{stub}");
    assert!(
        !stub.contains("def self.circle"),
        "tagged enum must not declare factories: {stub}"
    );
    assert!(
        !stub.contains("def self.rect"),
        "tagged enum must not declare factories: {stub}"
    );
}

#[test]
fn maps_named_dto_field_to_its_type() {
    let def = enum_def(
        "Source",
        vec![variant(
            "Llm",
            vec![field("config", TypeRef::Named("LlmConfig".to_string()))],
        )],
    );

    let stub = gen_enum_stub(&def, false);

    assert!(
        stub.contains("    def self.llm: (LlmConfig config) -> Source"),
        "{stub}"
    );
}

#[test]
fn skips_unit_tuple_excluded_and_sanitized_variants() {
    let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String)]);
    tuple_variant.is_tuple = true;
    let mut excluded = variant("Hidden", vec![field("value", TypeRef::String)]);
    excluded.binding_excluded = true;
    let mut sanitized_field = field("raw", TypeRef::String);
    sanitized_field.sanitized = true;
    let sanitized_variant = variant("Raw", vec![sanitized_field]);

    let def = enum_def(
        "Shape",
        vec![
            variant("Empty", vec![]),
            tuple_variant,
            excluded,
            sanitized_variant,
            variant("Real", vec![field("value", TypeRef::String)]),
        ],
    );

    let stub = gen_enum_stub(&def, false);

    assert!(!stub.contains("def self.empty"), "{stub}");
    assert!(!stub.contains("def self.pair"), "{stub}");
    assert!(!stub.contains("def self.hidden"), "{stub}");
    assert!(!stub.contains("def self.raw"), "{stub}");
    assert!(stub.contains("    def self.real: (String value) -> Shape"), "{stub}");
}

#[test]
fn optional_field_is_nilable() {
    let def = enum_def(
        "Source",
        vec![variant("Tag", vec![optional_field("label", TypeRef::String)])],
    );

    let stub = gen_enum_stub(&def, false);

    assert!(stub.contains("    def self.tag: (?String label) -> Source"), "{stub}");
}

#[test]
fn param_after_optional_is_promoted_to_nilable() {
    let def = enum_def(
        "Shape",
        vec![variant(
            "Ring",
            vec![
                optional_field("radius", TypeRef::Primitive(PrimitiveType::F64)),
                field("width", TypeRef::Primitive(PrimitiveType::U32)),
            ],
        )],
    );

    let stub = gen_enum_stub(&def, false);

    assert!(
        stub.contains("    def self.ring: (?Float radius, ?Integer width) -> Shape"),
        "{stub}"
    );
}

#[test]
fn yields_to_hand_written_method_of_same_name() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let stub = gen_enum_stub(&def, false);

    assert!(!stub.contains("def self.circle"), "hand-written method wins: {stub}");
    assert!(
        stub.contains("    def self.rect: (Integer width, Integer height) -> Shape"),
        "{stub}"
    );
}

#[test]
fn streaming_method_returns_enumerator_of_adapter_item_type() {
    let method = MethodDef {
        name: "chat_stream".to_string(),
        ..Default::default()
    };
    let mut streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    streaming.insert("chat_stream".to_string(), "ChatCompletionChunk".to_string());
    let excluded = std::collections::HashSet::new();
    let trait_interfaces = std::collections::HashSet::new();

    let stub = super::gen_method_stub(&method, false, false, &streaming, &excluded, &trait_interfaces, "Owner");

    assert!(
        stub.contains("Enumerator[ChatCompletionChunk]"),
        "streaming method must yield the adapter's declared item type: {stub}"
    );
    assert!(
        !stub.contains("Iterator]"),
        "must not emit an undeclared `<Method>Iterator` element type (steep RBS::UnknownTypeName): {stub}"
    );
}

#[test]
fn method_param_of_excluded_type_is_substituted_to_json_value() {
    let method = MethodDef {
        name: "register_document_extractor".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "extractor".to_string(),
            ty: TypeRef::Named("DocumentExtractor".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let excluded: std::collections::HashSet<&str> = ["DocumentExtractor"].into_iter().collect();
    let trait_interfaces = std::collections::HashSet::new();

    let stub = super::gen_method_stub(&method, false, false, &streaming, &excluded, &trait_interfaces, "Owner");

    assert!(
        !stub.contains("DocumentExtractor"),
        "an excluded type must not leak into the RBS stub (steep RBS::UnknownTypeName): {stub}"
    );
    assert!(
        stub.contains("json_value extractor"),
        "excluded param type must be substituted to the declared json_value alias: {stub}"
    );
}

#[test]
fn method_param_of_trait_interface_type_is_substituted_to_underscore_prefixed_name() {
    let method = MethodDef {
        name: "register_document_extractor".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "extractor".to_string(),
            ty: TypeRef::Named("DocumentExtractor".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let excluded = std::collections::HashSet::new();
    let trait_interfaces: std::collections::HashSet<&str> = ["DocumentExtractor"].into_iter().collect();

    let stub = super::gen_method_stub(&method, false, false, &streaming, &excluded, &trait_interfaces, "Owner");

    assert!(
        stub.contains("_DocumentExtractor extractor"),
        "a trait-typed param must reference the host-implementable `_TraitName` interface: {stub}"
    );
    assert!(
        !stub.contains("(DocumentExtractor extractor)"),
        "the bare trait name is never declared as an RBS type (steep RBS::UnknownTypeName): {stub}"
    );
}

#[test]
fn builder_method_returning_owning_type_emits_owning_class_not_json_value() {
    // Regression test: the owner type (e.g. a service owner managed by the services ~keep
    // extraction pass) is `binding_excluded` at the IR level, so it lands in `excluded` — ~keep
    // but `gen_method_stub` is called here *while emitting that very class's stub*, so a ~keep
    // `Self`-returning method (already resolved to `Named("App")` during extraction) must ~keep
    // reference the real "App" class, not fall back to the `json_value` alias. ~keep
    let method = MethodDef {
        name: "on_request".to_string(),
        is_static: false,
        return_type: TypeRef::Named("App".to_string()),
        params: vec![crate::core::ir::ParamDef {
            name: "hook".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let excluded: std::collections::HashSet<&str> = ["App"].into_iter().collect();
    let trait_interfaces = std::collections::HashSet::new();

    let stub = super::gen_method_stub(&method, false, false, &streaming, &excluded, &trait_interfaces, "App");

    assert!(stub.contains("-> App"), "{stub}");
    assert!(!stub.contains("json_value"), "{stub}");
}

#[test]
fn static_constructor_returning_owning_type_emits_owning_class_not_json_value() {
    let method = MethodDef {
        name: "new".to_string(),
        is_static: true,
        return_type: TypeRef::Named("App".to_string()),
        ..Default::default()
    };
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let excluded: std::collections::HashSet<&str> = ["App"].into_iter().collect();
    let trait_interfaces = std::collections::HashSet::new();

    let stub = super::gen_method_stub(&method, true, false, &streaming, &excluded, &trait_interfaces, "App");

    assert!(stub.contains("def self.new: () -> App"), "{stub}");
}

#[test]
fn function_param_of_trait_interface_type_is_substituted_to_underscore_prefixed_name() {
    let func = crate::core::ir::FunctionDef {
        name: "register_document_extractor".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "extractor".to_string(),
            ty: TypeRef::Named("DocumentExtractor".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let excluded = std::collections::HashSet::new();
    let trait_interfaces: std::collections::HashSet<&str> = ["DocumentExtractor"].into_iter().collect();

    let stub = super::gen_function_stub(&func, &streaming, &excluded, &trait_interfaces);

    assert!(
        stub.contains("_DocumentExtractor extractor"),
        "a trait-typed param must reference the host-implementable `_TraitName` interface: {stub}"
    );
    assert!(
        !stub.contains("(DocumentExtractor extractor)"),
        "the bare trait name is never declared as an RBS type (steep RBS::UnknownTypeName): {stub}"
    );
}

/// RBS rejects a class that declares an attribute and a same-named method
/// (`RBS::DuplicatedMethodDefinition`). A core type with a public `providers` field and an
/// inherent `providers()` method feeds both emitters, so the attribute wins and the method
/// stub is dropped — matching what the magnus binding emits.
#[test]
fn attr_and_same_named_method_emit_the_name_once() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "LlmConfig".to_string(),
        rust_path: "test_lib::LlmConfig".to_string(),
        fields: vec![optional_field("providers", TypeRef::String)],
        methods: vec![MethodDef {
            name: "providers".to_string(),
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(&api, "test_lib", false, &ahash::AHashMap::new(), &[], false);

    let attrs = stub.matches("attr_reader providers:").count();
    let methods = stub.matches("def providers:").count();
    assert_eq!(attrs, 1, "the attribute must still be declared once:\n{stub}");
    assert_eq!(
        methods, 0,
        "the same-named method stub must be dropped, found {methods}:\n{stub}"
    );
}
