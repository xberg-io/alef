use super::gen_enum_stub;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};
use ahash::AHashSet;

/// No dataclass-backed config DTOs — factory params map exactly as `python_type` would.
fn no_dtos() -> AHashSet<&'static str> {
    AHashSet::new()
}

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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
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
    let stub = gen_enum_stub(&shape_enum(), false, &no_dtos(), true);

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
    let circle_at = stub.find("def circle").unwrap();
    let str_at = stub.find("def __str__").unwrap();
    assert!(circle_at < str_at, "constructors must precede dunders: {stub}");
}

fn cfg_shape_enum() -> EnumDef {
    let mut gated = variant(
        "Rect",
        vec![
            field("width", TypeRef::Primitive(PrimitiveType::U32)),
            field("height", TypeRef::Primitive(PrimitiveType::U32)),
        ],
    );
    gated.cfg = Some(r#"feature = "extra-shapes""#.to_string());
    enum_def(
        "Shape",
        vec![
            variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
            gated,
        ],
    )
}

/// The stub must not advertise a `@staticmethod` the runtime binding drops for the same reason
/// (`gen_pyo3_enum_variant_constructors_content`, `codegen::generators::enums`): a FOREIGN
/// cfg-gated variant's factory is unreachable (compiled out via `#[cfg(...)]` naming an undeclared
/// feature) and dropped unconditionally.
#[test]
fn drops_staticmethod_for_foreign_cfg_gated_variant() {
    let stub = gen_enum_stub(&cfg_shape_enum(), false, &no_dtos(), false);

    assert!(!stub.contains("def rect("), "{stub}");
    assert!(
        stub.contains("    @staticmethod\n    def circle(radius: float) -> Shape: ..."),
        "{stub}"
    );
}

/// Control: a host-owned cfg-gated variant's factory must stay documented.
#[test]
fn keeps_staticmethod_for_host_owned_cfg_gated_variant() {
    let stub = gen_enum_stub(&cfg_shape_enum(), false, &no_dtos(), true);

    assert!(
        stub.contains("    @staticmethod\n    def rect(width: int, height: int) -> Shape: ..."),
        "{stub}"
    );
}

#[test]
fn dunder_stubs_carry_no_unused_noqa() {
    let stub = gen_enum_stub(&shape_enum(), false, &no_dtos(), true);

    assert!(stub.contains("    def __str__(self) -> str: ..."), "{stub}");
    assert!(stub.contains("    def __repr__(self) -> str: ..."), "{stub}");
    // A `# noqa: PYI029` here is flagged by ruff RUF100 (unused, PYI029 not enabled).
    assert!(
        !stub.contains("noqa"),
        "dunder stubs must not carry a suppression comment: {stub}"
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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

    assert!(
        stub.contains("    @staticmethod\n    def llm(config: LlmConfig) -> Source: ..."),
        "{stub}"
    );
}

#[test]
fn widens_dataclass_backed_config_dto_factory_param() {
    let def = enum_def(
        "EmbeddingModelType",
        vec![
            variant("Llm", vec![field("llm", TypeRef::Named("LlmConfig".to_string()))]),
            variant("Preset", vec![field("name", TypeRef::String)]),
        ],
    );
    let coercible: AHashSet<&str> = ["LlmConfig"].into_iter().collect();

    let stub = gen_enum_stub(&def, false, &coercible, true);

    assert!(
        stub.contains(
            "    @staticmethod\n    def llm(llm: options.LlmConfig | dict[str, Any]) -> EmbeddingModelType: ..."
        ),
        "coercible DTO factory param must accept the public dataclass or a dict: {stub}"
    );
    assert!(
        stub.contains("    @staticmethod\n    def preset(name: str) -> EmbeddingModelType: ..."),
        "primitive factory param must be unchanged: {stub}"
    );
}

#[test]
fn qualifies_builtin_shadowed_by_a_variant_factory_name() {
    let def = enum_def(
        "NodeContent",
        vec![
            variant("List", vec![field("ordered", TypeRef::String)]),
            variant(
                "MetadataBlock",
                vec![field(
                    "entries",
                    TypeRef::Vec(Box::new(TypeRef::Named("MetadataEntry".to_string()))),
                )],
            ),
        ],
    );

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

    assert!(
        stub.contains("    @staticmethod\n    def list(ordered: str) -> NodeContent: ..."),
        "{stub}"
    );
    assert!(
        stub.contains("def metadata_block(entries: builtins.list[MetadataEntry]) -> NodeContent: ..."),
        "builtin shadowed by the `list` factory must be qualified as builtins.list: {stub}"
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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

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
fn optional_field_is_nilable_with_default() {
    let def = enum_def(
        "Source",
        vec![variant("Tag", vec![optional_field("label", TypeRef::String)])],
    );

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

    assert!(
        stub.contains("    @staticmethod\n    def tag(label: str | None = None) -> Source: ..."),
        "{stub}"
    );
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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

    assert!(
        stub.contains(
            "    @staticmethod\n    def ring(radius: float | None = None, width: int | None = None) -> Shape: ..."
        ),
        "{stub}"
    );
}

/// Regression for the `ContentPart` bug: the runtime pyo3 binding never forwards
/// `enum_def.methods` (a hand-written inherent static method from a separate
/// `impl EnumType { .. }` block) into the generated `#[pymethods]` block, so skipping the derived
/// factory stub on a name collision matched a runtime binding that also dropped the constructor —
/// `ContentPart.text(...)` raised `AttributeError`. The stub must keep emitting the derived
/// factory, matching the now-fixed runtime binding.
#[test]
fn emits_factory_stub_even_with_colliding_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let stub = gen_enum_stub(&def, false, &no_dtos(), true);

    assert!(
        stub.contains("    @staticmethod\n    def circle(radius: float) -> Shape: ..."),
        "circle factory stub must stay reachable despite the colliding hand-written method: {stub}"
    );
    assert!(
        stub.contains("    @staticmethod\n    def rect(width: int, height: int) -> Shape: ..."),
        "{stub}"
    );
}

/// The ground truth for the stub below: serde decides where an adjacently tagged variant's payload
/// goes, and the TypedDict has to describe that document rather than the internal-tagged one.
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "body")]
enum AdjacentShape {
    Circle { radius: f64 },
}

fn adjacent_shape_enum() -> EnumDef {
    EnumDef {
        serde_content: Some("body".to_string()),
        serde_tag: Some("kind".to_string()),
        ..enum_def(
            "AdjacentShape",
            vec![
                variant("Empty", vec![]),
                variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
            ],
        )
    }
}

#[test]
fn adjacent_variant_typeddict_nests_the_payload_under_the_content_key() {
    let stub = gen_enum_stub(&adjacent_shape_enum(), false, &no_dtos(), true);
    let wire: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&AdjacentShape::Circle { radius: 1.0 }).expect("serializes"))
            .expect("serde output is JSON");
    let content_key = wire
        .as_object()
        .expect("adjacent tagging writes an object")
        .keys()
        .find(|key| key.as_str() != "kind")
        .expect("serde writes a content key for a data variant");

    assert!(
        stub.contains("class AdjacentShapeCirclePayload(TypedDict):\n    radius: float"),
        "a struct variant's payload is an object in its own right and needs its own TypedDict: {stub}"
    );
    assert!(
        stub.contains(&format!(
            "class AdjacentShapeCircleVariant(TypedDict):\n    kind: Literal[\"Circle\"]\n    {content_key}: AdjacentShapeCirclePayload"
        )),
        "the variant TypedDict must nest the payload under serde's content key {content_key:?}: {stub}"
    );
    assert!(
        !stub.contains("class AdjacentShapeCircleVariant(TypedDict):\n    kind: Literal[\"Circle\"]\n    radius:"),
        "declaring the payload's fields flat beside the tag is serde's *internal* form: {stub}"
    );
    assert!(
        stub.contains("class AdjacentShapeEmptyVariant(TypedDict):\n    kind: Literal[\"Empty\"]\n"),
        "a unit variant gets no content key at all, because serde writes none: {stub}"
    );
}
