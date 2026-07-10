use super::gen_data_enum_variant_constructor_stubs;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
            variant(
                "Circle",
                vec![field("radius", TypeRef::Primitive(PrimitiveType::F64), false)],
            ),
            variant(
                "Rect",
                vec![
                    field("width", TypeRef::Primitive(PrimitiveType::U32), false),
                    field("height", TypeRef::Primitive(PrimitiveType::U32), false),
                ],
            ),
        ],
    )
}

#[test]
fn emits_static_factory_per_struct_variant() {
    let stubs = gen_data_enum_variant_constructor_stubs(&shape_enum()).join("");

    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
    assert!(
        stubs.contains("public static function rect(int $width, int $height): Shape"),
        "{stubs}"
    );
}

#[test]
fn maps_named_dto_field_to_its_type() {
    let def = enum_def(
        "Source",
        vec![variant(
            "Llm",
            vec![field("config", TypeRef::Named("LlmConfig".to_string()), false)],
        )],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("public static function llm(LlmConfig $config): Source"),
        "{stubs}"
    );
}

#[test]
fn emits_param_phpdoc_for_map_and_vec_variant_fields() {
    // `@param array<...>` PHPDoc, otherwise PHPStan (level max) flags the bare `array`
    let def = enum_def(
        "CacheBackend",
        vec![
            variant(
                "OpenDal",
                vec![
                    field("scheme", TypeRef::String, false),
                    field(
                        "config",
                        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                        false,
                    ),
                ],
            ),
            variant(
                "Tags",
                vec![field("labels", TypeRef::Vec(Box::new(TypeRef::String)), false)],
            ),
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("/** @param array<string, string> $config */"),
        "map parameter should get a typed @param PHPDoc:\n{stubs}"
    );
    assert!(
        stubs.contains("/** @param array<string> $labels */"),
        "vec parameter should get a typed @param PHPDoc:\n{stubs}"
    );
    assert!(
        stubs.contains("public static function openDal(string $scheme, array $config): CacheBackend"),
        "{stubs}"
    );
}

#[test]
fn optional_field_is_nullable_with_default() {
    let def = enum_def(
        "Source",
        vec![variant("Tag", vec![field("label", TypeRef::String, true)])],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("public static function tag(?string $label = null): Source"),
        "{stubs}"
    );
}

#[test]
fn skips_unit_tuple_excluded_and_sanitized_variants() {
    let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String, false)]);
    tuple_variant.is_tuple = true;
    let mut excluded = variant("Hidden", vec![field("value", TypeRef::String, false)]);
    excluded.binding_excluded = true;
    let mut sanitized_field = field("raw", TypeRef::String, false);
    sanitized_field.sanitized = true;
    let sanitized_variant = variant("Raw", vec![sanitized_field]);

    let def = enum_def(
        "Shape",
        vec![
            variant("Empty", vec![]),
            tuple_variant,
            excluded,
            sanitized_variant,
            variant("Real", vec![field("value", TypeRef::String, false)]),
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(!stubs.contains("function empty("), "{stubs}");
    assert!(!stubs.contains("function pair("), "{stubs}");
    assert!(!stubs.contains("function hidden("), "{stubs}");
    assert!(!stubs.contains("function raw("), "{stubs}");
    assert!(
        stubs.contains("public static function real(string $value): Shape"),
        "{stubs}"
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

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(!stubs.contains("function circle("), "hand-written method wins: {stubs}");
    assert!(
        stubs.contains("public static function rect(int $width, int $height): Shape"),
        "{stubs}"
    );
}
