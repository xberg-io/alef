use super::gen_enum_stub;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;

/// No dataclass-backed config DTOs — factory params map exactly as `python_type` would.
fn no_dtos() -> AHashSet<&'static str> {
    AHashSet::new()
}

#[test]
fn untagged_and_external_payloads_do_not_invent_discriminator_dictionaries() {
    for untagged in [true, false] {
        let def = EnumDef {
            name: "Record".into(),
            serde_untagged: untagged,
            variants: vec![EnumVariant {
                name: "Canonical".into(),
                is_tuple: true,
                fields: vec![field("_0", TypeRef::Named("Payload".into()))],
                ..Default::default()
            }],
            ..Default::default()
        };
        let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);
        assert!(!stub.contains("CanonicalVariant(TypedDict)"), "{stub}");
        assert!(!stub.contains("Literal["), "{stub}");
        assert!(stub.contains("canonical: Payload | None"), "{stub}");
        assert!(stub.contains("def from_canonical(value: Payload) -> Record"), "{stub}");
    }
}

#[test]
fn builtin_named_factory_qualifies_sibling_factory_annotations() {
    let enum_def = EnumDef {
        name: "BinaryPayload".to_string(),
        variants: vec![
            EnumVariant {
                name: "Bytes".to_string(),
                fields: vec![field("marker", TypeRef::Primitive(PrimitiveType::U8))],
                ..Default::default()
            },
            EnumVariant {
                name: "Data".to_string(),
                fields: vec![field("payload", TypeRef::Bytes)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let stub = gen_enum_stub(&enum_def, false, &no_dtos(), true, &[]);

    assert!(
        stub.contains("def bytes("),
        "fixture must emit the shadowing factory:\n{stub}"
    );
    assert!(
        stub.contains("payload: builtins.bytes"),
        "factory names shadow builtin annotations across the enum class:\n{stub}"
    );
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
        serde_untagged: false,
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
    let stub = gen_enum_stub(&shape_enum(), false, &no_dtos(), true, &[]);

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
    let stub = gen_enum_stub(&cfg_shape_enum(), false, &no_dtos(), false, &[]);

    assert!(!stub.contains("def rect("), "{stub}");
    assert!(
        stub.contains("    @staticmethod\n    def circle(radius: float) -> Shape: ..."),
        "{stub}"
    );
}

/// Control: a host-owned cfg-gated variant's factory must stay documented.
#[test]
fn keeps_staticmethod_for_host_owned_cfg_gated_variant() {
    let stub = gen_enum_stub(&cfg_shape_enum(), false, &no_dtos(), true, &[]);

    assert!(
        stub.contains("    @staticmethod\n    def rect(width: int, height: int) -> Shape: ..."),
        "{stub}"
    );
}

#[test]
fn dunder_stubs_carry_no_unused_noqa() {
    let stub = gen_enum_stub(&shape_enum(), false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &coercible, true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

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
    let stub = gen_enum_stub(&adjacent_shape_enum(), false, &no_dtos(), true, &[]);
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

/// The ground truth for the stubs below: serde's INTERNAL form merges a newtype variant's
/// payload into the tag object, so the wire has the payload's own keys and nothing named `_0`.
#[derive(serde::Serialize)]
struct ExcelPayload {
    sheet_count: u32,
}

#[derive(serde::Serialize)]
#[serde(tag = "format_type", rename_all = "snake_case")]
enum FlattenedMetadata {
    Excel(ExcelPayload),
}

fn flattened_metadata_enum() -> EnumDef {
    EnumDef {
        serde_tag: Some("format_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..enum_def(
            "FlattenedMetadata",
            vec![EnumVariant {
                is_tuple: true,
                ..variant("Excel", vec![field("_0", TypeRef::Named("ExcelPayload".to_string()))])
            }],
        )
    }
}

fn excel_payload_type() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "ExcelPayload".to_string(),
        rust_path: "test_lib::ExcelPayload".to_string(),
        fields: vec![field("sheet_count", TypeRef::Primitive(PrimitiveType::U32))],
        has_serde: true,
        ..Default::default()
    }]
}

#[test]
fn should_not_emit_underscore_zero_key_for_flattened_newtype_variant() {
    let stub = gen_enum_stub(
        &flattened_metadata_enum(),
        false,
        &no_dtos(),
        true,
        &excel_payload_type(),
    );
    let wire: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&FlattenedMetadata::Excel(ExcelPayload { sheet_count: 2 })).expect("serializes"),
    )
    .expect("serde output is JSON");
    let keys: Vec<&str> = wire
        .as_object()
        .expect("internal tagging writes an object")
        .keys()
        .map(String::as_str)
        .collect();

    assert_eq!(
        keys,
        vec!["format_type", "sheet_count"],
        "serde writes the payload's own keys beside the tag, never `_0`"
    );
    let expected = concat!(
        "class FlattenedMetadataExcelVariant(TypedDict):\n",
        "    format_type: Literal[\"excel\"]\n",
        "    sheet_count: int"
    );
    assert!(
        stub.contains(expected),
        "the flattened variant's TypedDict must declare the payload's fields: {stub}"
    );
    assert!(
        !stub.contains("_0:"),
        "`_0` is synthesized by the extractor and appears on no wire form of this enum: {stub}"
    );
}

/// A payload the surface does not declare leaves the fields unknowable. The stub then says less
/// than the wire carries, which is incomplete but true — inventing the `_0` key back is not.
#[test]
fn should_emit_the_tag_alone_when_a_flattened_payload_type_is_not_on_the_surface() {
    let stub = gen_enum_stub(&flattened_metadata_enum(), false, &no_dtos(), true, &[]);

    assert!(
        stub.contains("class FlattenedMetadataExcelVariant(TypedDict):\n    format_type: Literal[\"excel\"]\n"),
        "the discriminator is still known: {stub}"
    );
    assert!(!stub.contains("_0:"), "no fabricated positional key: {stub}");
}

/// External tagging keys a single-field tuple payload on the variant name itself and untagged
/// writes it bare -- neither ever puts a `_0` key on the wire, so a TypedDict declaring one
/// describes a shape the runtime constructor never accepts and nothing else in the stub
/// references. Corrects an earlier version of this test (`git blame` predates this fix), which
/// asserted the opposite and thereby enshrined the exact defect this suppresses: `_0: ExcelPayload`
/// on `FormatMetadataExcelVariant` while the wrapper class already exposes the identical value,
/// correctly typed, as `excel: ExcelPayload | None`.
#[test]
fn should_not_declare_the_positional_field_for_representations_serde_does_not_flatten() {
    for (case, untagged) in [("external tagging", false), ("untagged", true)] {
        let def = EnumDef {
            serde_tag: None,
            serde_untagged: untagged,
            ..flattened_metadata_enum()
        };
        let stub = gen_enum_stub(&def, false, &no_dtos(), true, &excel_payload_type());
        assert!(
            !stub.contains("_0:"),
            "{case} must not fabricate a positional key no wire form of this variant has: {stub}"
        );
    }
}

/// `FormatMetadata`-shaped fixture: a plain single-tuple-Named variant (`Excel`), a boxed one
/// gated behind a host feature (`Docx`), an acronym-ish multi-word PascalCase name
/// (`FictionBook`), a unit variant (`Empty`), and a multi-field tuple variant (`Pair`) whose
/// payload isn't a single `Named` type. Mirrors `crates/xberg-py/src/lib.rs`'s real
/// `FormatMetadata` getters closely enough to catch a naming-transform regression.
fn metadata_enum() -> EnumDef {
    let mut docx = variant(
        "Docx",
        vec![{
            let mut f = field("_0", TypeRef::Named("DocxMetadata".to_string()));
            f.is_boxed = true;
            f
        }],
    );
    docx.cfg = Some(r#"feature = "office""#.to_string());

    enum_def(
        "FormatMetadata",
        vec![
            variant("Excel", vec![field("_0", TypeRef::Named("ExcelMetadata".to_string()))]),
            docx,
            variant(
                "FictionBook",
                vec![field("_0", TypeRef::Named("FictionBookMetadata".to_string()))],
            ),
            variant("Empty", vec![]),
            variant("Pair", vec![field("_0", TypeRef::String), field("_1", TypeRef::String)]),
        ],
    )
}

/// The stub must declare a bare-annotation property for every variant `write_pyo3_variant_accessors`
/// (`codegen::generators::enums`) gives a typed `#[getter]` — including the boxed, cfg-gated, and
/// multi-word-PascalCase cases — using the same `pascal_to_snake` transform the runtime getter uses.
#[test]
fn should_declare_a_stub_property_for_every_runtime_variant_getter() {
    let stub = gen_enum_stub(&metadata_enum(), false, &no_dtos(), true, &[]);

    assert!(stub.contains("    excel: ExcelMetadata | None"), "{stub}");
    assert!(
        stub.contains("    docx: DocxMetadata | None"),
        "boxed field must still resolve to the unboxed inner type: {stub}"
    );
    assert!(
        stub.contains("    fiction_book: FictionBookMetadata | None"),
        "multi-word PascalCase variant name must snake_case correctly: {stub}"
    );
}

/// Negative control: a unit variant and a multi-field tuple variant get no runtime typed
/// `#[getter]` (they fall back to the untyped dict-shaped getter instead), so the stub must not
/// declare a property for either.
#[test]
fn should_not_declare_a_stub_property_for_a_variant_with_no_runtime_typed_getter() {
    let stub = gen_enum_stub(&metadata_enum(), false, &no_dtos(), true, &[]);

    assert!(!stub.contains("    empty:"), "unit variant: {stub}");
    assert!(!stub.contains("    pair:"), "multi-field tuple variant: {stub}");
}

/// `write_pyo3_variant_accessors` emits the `#[getter]` for a foreign-cfg-gated variant
/// unconditionally — only the match ARM inside it is dropped when `is_host_enum` is false, never
/// the getter method itself — so the stub property must stay declared regardless of
/// `is_host_enum`, unlike the `@staticmethod` variant constructors which really do disappear.
#[test]
fn should_keep_the_stub_property_for_a_foreign_cfg_gated_variant_even_when_unreachable() {
    let stub = gen_enum_stub(&metadata_enum(), false, &no_dtos(), false, &[]);

    assert!(
        stub.contains("    docx: DocxMetadata | None"),
        "the getter method exists in every build; only its match arm is cfg-conditional: {stub}"
    );
}

/// An externally tagged data enum (`EntityCategory`, `PiiCategory`, `OutputFormat` -- no
/// `#[serde(tag = "...")]` anywhere) has no `type` field anywhere on its wire form, and
/// `write_pyo3_serde_tag_getter` (`codegen::generators::enums`) is only ever called when
/// `enum_def.serde_tag` is `Some`, so the runtime pyclass exposes no `.type` attribute for this
/// enum at all. Declaring one in the stub was `AttributeError` waiting to happen on first use.
#[test]
fn external_tagging_does_not_declare_a_type_attribute() {
    let def = EnumDef {
        serde_tag: None,
        ..enum_def(
            "EntityCategory",
            vec![
                variant("Person", vec![]),
                EnumVariant {
                    is_tuple: true,
                    ..variant("Custom", vec![field("_0", TypeRef::String)])
                },
            ],
        )
    };

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

    assert!(!stub.contains("    type: str"), "{stub}");
    assert!(
        stub.contains("    def __init__(self, value: dict[str, Any] | str | None = None, **kwargs: Any) -> None: ..."),
        "the `#[new]` constructor is genuinely unconditional (no sanitized fields), unlike `.type`: {stub}"
    );
}

/// An internally or adjacently tagged enum (`serde_tag: Some(..)`) keeps its `type`-shaped
/// attribute: `write_pyo3_serde_tag_getter` really does emit that getter for this repr.
#[test]
fn internal_tagging_keeps_the_tag_attribute() {
    let stub = gen_enum_stub(&shape_enum(), false, &no_dtos(), true, &[]);

    assert!(stub.contains("    type: str"), "{stub}");
}

/// `EntityCategory::Custom(String)` / `PiiCategory::Custom(String)` shaped fixture: a single-field
/// tuple variant whose payload is a primitive, not a `TypeRef::Named` type, so it never qualifies
/// for the typed accessor property either -- only the untyped dict-shaped runtime getter covers
/// it. The synthetic `_0` TypedDict key still describes no real wire form (external tagging keys
/// the payload on the variant name itself) and was referenced by nothing else in the stub.
#[test]
fn primitive_payload_single_tuple_variant_does_not_declare_a_positional_field() {
    let def = EnumDef {
        serde_tag: None,
        ..enum_def(
            "EntityCategory",
            vec![
                variant("Person", vec![]),
                EnumVariant {
                    is_tuple: true,
                    ..variant("Custom", vec![field("_0", TypeRef::String)])
                },
            ],
        )
    };

    let stub = gen_enum_stub(&def, false, &no_dtos(), true, &[]);

    assert!(!stub.contains("_0:"), "{stub}");
}

/// A genuine multi-field tuple variant keeps its declared positional shape -- only the
/// single-field case (which never has a `_0` key on any wire form) is suppressed.
#[test]
fn multi_field_tuple_variant_keeps_its_positional_fields() {
    let stub = gen_enum_stub(&metadata_enum(), false, &no_dtos(), true, &[]);

    assert!(stub.contains("class FormatMetadataPairVariant(TypedDict):"), "{stub}");
    assert!(stub.contains("    _0: str"), "{stub}");
    assert!(stub.contains("    _1: str"), "{stub}");
}
