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
        // Non-tagged: per-variant singleton constructors only apply to data enums WITHOUT a serde
        // tag. Tagged data enums are represented as a Ruby `module` (variant `Data` classes) and get
        // no Rust factory class — see `tagged_data_enum_emits_no_singleton_constructors`.
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
    // Tagged data enums are represented on the Ruby side as a `module` with variant `Data` classes,
    // not a Rust factory class — defining one collides at load (`TypeError: <Name> is not a module`).
    // So the rbs declares no `self.<variant>` singleton constructors for them.
    let tagged = EnumDef {
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
    // `width` is not optional in the IR, but it follows the optional `radius`, so the runtime magnus
    // binding wraps it in `Option<T>`. The RBS stub must mark it nilable too to match.
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
