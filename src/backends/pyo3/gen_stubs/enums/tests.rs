use super::gen_enum_stub;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
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
        serde_tag: Some("type".to_string()),
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
fn emits_staticmethod_constructor_per_struct_variant() {
    let stub = gen_enum_stub(&shape_enum(), false);

    assert!(stub.contains("class Shape:"), "{stub}");
    assert!(stub.contains("    type: str"), "{stub}");
    assert!(
        stub.contains("    @staticmethod\n    def circle(radius: float) -> Shape: ..."),
        "{stub}"
    );
    assert!(
        stub.contains("    @staticmethod\n    def rect(width: int, height: int) -> Shape: ..."),
        "{stub}"
    );
    // Constructors precede the dunder stubs.
    let circle_at = stub.find("def circle").unwrap();
    let str_at = stub.find("def __str__").unwrap();
    assert!(circle_at < str_at, "constructors must precede dunders: {stub}");
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
        stub.contains("    @staticmethod\n    def llm(config: LlmConfig) -> Source: ..."),
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

    assert!(!stub.contains("def empty("), "{stub}");
    assert!(!stub.contains("def pair("), "{stub}");
    assert!(!stub.contains("def hidden("), "{stub}");
    assert!(!stub.contains("def raw("), "{stub}");
    assert!(
        stub.contains("    @staticmethod\n    def real(value: str) -> Shape: ..."),
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

    assert!(!stub.contains("def circle("), "hand-written method wins: {stub}");
    assert!(
        stub.contains("    @staticmethod\n    def rect(width: int, height: int) -> Shape: ..."),
        "{stub}"
    );
}
