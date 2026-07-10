use crate::backends::extendr::gen_bindings::ExtendrBackend;
use crate::backends::extendr::gen_bindings::bridges::{
    extendr_enum_variant_constructor_registrations, gen_extendr_enum_variant_constructors,
    gen_extendr_json_passthrough_enum_struct,
};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        ..Default::default()
    }
}

/// A tagged data enum with struct variants — the JSON-passthrough shape.
fn shape_enum() -> EnumDef {
    EnumDef {
        name: "Shape".to_string(),
        rust_path: "test_lib::Shape".to_string(),
        variants: vec![
            variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
            variant(
                "Rect",
                vec![
                    field("width", TypeRef::Primitive(PrimitiveType::F64)),
                    field("height", TypeRef::Primitive(PrimitiveType::F64)),
                ],
            ),
        ],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    }
}

#[test]
fn emits_constructor_per_struct_variant_building_core_then_into() {
    let core_path = "test_lib::Shape";
    let methods = gen_extendr_enum_variant_constructors(&shape_enum(), &ExtendrBackend, core_path);

    let code = methods.join("\n");
    assert!(code.contains("pub fn _factory_circle(radius: f64) -> Shape"), "{code}");
    assert!(code.contains("test_lib::Shape::Circle { radius }.into()"), "{code}");
    assert!(
        code.contains("pub fn _factory_rect(width: f64, height: f64) -> Shape"),
        "{code}"
    );
    assert!(
        code.contains("test_lib::Shape::Rect { width, height }.into()"),
        "{code}"
    );
}

#[test]
fn casts_remapped_primitive_back_to_core() {
    let def = EnumDef {
        name: "Sized_".to_string(),
        rust_path: "test_lib::Sized_".to_string(),
        variants: vec![variant(
            "Big",
            vec![field("count", TypeRef::Primitive(PrimitiveType::U64))],
        )],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Sized_");
    let code = methods.join("\n");
    assert!(code.contains("pub fn _factory_big(count: f64) -> Sized_"), "{code}");
    assert!(
        code.contains("test_lib::Sized_::Big { count: count as u64 }.into()"),
        "{code}"
    );
}

#[test]
fn skips_variant_constructor_with_named_dto_field() {
    // extendr derives `TryFrom<&Robj>` only for `&T` of #[extendr] types, never owned `T`, so a
    // `#[extendr]` proc-macro (`error[E0277]: T: TryFrom<&Robj> not satisfied`). Variants whose
    let def = EnumDef {
        name: "Wrapper".to_string(),
        rust_path: "test_lib::Wrapper".to_string(),
        variants: vec![
            variant("Llm", vec![field("llm", TypeRef::Named("LlmConfig".to_string()))]),
            variant("Tag", vec![field("name", TypeRef::String)]),
        ],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Wrapper");
    let code = methods.join("\n");
    assert!(
        !code.contains("_factory_llm"),
        "variant with a Named DTO field must be skipped: {code}"
    );
    assert!(
        code.contains("pub fn _factory_tag(name: String) -> Wrapper"),
        "primitive/String variant must still be generated: {code}"
    );
}

#[test]
fn skips_variant_constructor_when_any_field_is_unconstructible() {
    // field by value breaks the whole `#[extendr]` constructor.
    let def = EnumDef {
        name: "Job".to_string(),
        rust_path: "test_lib::Job".to_string(),
        variants: vec![
            variant(
                "Run",
                vec![
                    field("config", TypeRef::Named("RunConfig".to_string())),
                    field("retries", TypeRef::Primitive(PrimitiveType::U32)),
                    field("name", TypeRef::String),
                ],
            ),
            variant(
                "Tag",
                vec![field(
                    "entries",
                    TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))),
                )],
            ),
            variant("Ping", vec![field("seq", TypeRef::Primitive(PrimitiveType::U32))]),
        ],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Job");
    let code = methods.join("\n");
    assert!(
        !code.contains("_factory_run"),
        "Named-DTO-field variant must be skipped: {code}"
    );
    assert!(
        !code.contains("_factory_tag"),
        "Vec<DTO>-field variant must be skipped: {code}"
    );
    assert!(
        code.contains("test_lib::Job::Ping { seq: seq as u32 }.into()"),
        "primitive-only variant must still be generated: {code}"
    );
}

#[test]
fn skips_unit_tuple_and_excluded_variants() {
    let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String)]);
    tuple_variant.is_tuple = true;
    let mut excluded = variant("Hidden", vec![field("value", TypeRef::String)]);
    excluded.binding_excluded = true;

    let def = EnumDef {
        name: "Mixed".to_string(),
        rust_path: "test_lib::Mixed".to_string(),
        variants: vec![
            variant("Empty", vec![]),
            tuple_variant,
            excluded,
            variant("Real", vec![field("value", TypeRef::String)]),
        ],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Mixed");
    let code = methods.join("\n");
    assert!(!code.contains("_factory_empty"), "{code}");
    assert!(!code.contains("_factory_pair"), "{code}");
    assert!(!code.contains("_factory_hidden"), "{code}");
    assert!(code.contains("pub fn _factory_real(value: String) -> Mixed"), "{code}");
}

#[test]
fn yields_to_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Shape");
    let code = methods.join("\n");
    assert!(!code.contains("_factory_circle"), "consumer method wins: {code}");
    assert!(code.contains("pub fn _factory_rect"), "{code}");
}

#[test]
fn struct_embeds_constructors_in_impl_block() {
    // End to end: the generated `#[extendr] impl` block carries default/from_json AND the
    let code = gen_extendr_json_passthrough_enum_struct(&shape_enum(), &ExtendrBackend, "test_lib");
    assert!(code.contains("pub fn default() -> Shape"), "{code}");
    assert!(code.contains("pub fn from_json(json: String)"), "{code}");
    assert!(code.contains("pub fn _factory_circle(radius: f64) -> Shape"), "{code}");
}

#[test]
fn casts_optional_remapped_primitive_back_to_core() {
    let mut max_field = field("max", TypeRef::Primitive(PrimitiveType::U64));
    max_field.optional = true;
    let def = EnumDef {
        name: "Bounded".to_string(),
        rust_path: "test_lib::Bounded".to_string(),
        variants: vec![variant("Limit", vec![max_field])],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Bounded");
    let code = methods.join("\n");
    assert!(
        code.contains("pub fn _factory_limit(max: Option<f64>) -> Bounded"),
        "{code}"
    );
    assert!(
        code.contains("test_lib::Bounded::Limit { max: max.map(|v| v as u64) }.into()"),
        "{code}"
    );
}

#[test]
fn registrations_pair_r_name_with_factory_fn() {
    let regs = extendr_enum_variant_constructor_registrations(&shape_enum());
    assert_eq!(
        regs,
        vec![
            (
                "circle".to_string(),
                "_factory_circle".to_string(),
                vec!["radius".to_string()]
            ),
            (
                "rect".to_string(),
                "_factory_rect".to_string(),
                vec!["width".to_string(), "height".to_string()]
            ),
        ]
    );
}

#[test]
fn r_wrapper_binds_variant_constructor_under_snake_name() {
    use crate::core::backend::Backend;

    let backend = ExtendrBackend;
    let config = super::make_config();
    let mut api = super::make_api_surface();
    api.enums = vec![shape_enum()];

    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;

    assert!(
        content.contains("Shape$circle <- function(radius)"),
        "variant ctor must bind under the bare snake name: {content}"
    );
    assert!(
        content.contains(".Call(\"wrap__Shape___factory_circle\", radius"),
        "variant ctor must call the _factory_ symbol: {content}"
    );
    assert!(content.contains("Shape$rect <- function(width, height)"), "{content}");
    assert!(
        content.contains(".Call(\"wrap__Shape___factory_rect\", width, height"),
        "{content}"
    );
}
