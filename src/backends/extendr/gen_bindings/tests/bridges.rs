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
    // Wrapper-convert model: build the CORE variant then `.into()` the JSON-passthrough wrapper.
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
    // extendr maps u64 → f64; the constructor must cast it back when building the core variant.
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
fn converts_named_dto_field_via_core_let_binding() {
    // A Named-DTO field arrives as a binding type; convert via a `<field>_core` let binding then
    // build the core variant with that.
    let def = EnumDef {
        name: "Wrapper".to_string(),
        rust_path: "test_lib::Wrapper".to_string(),
        variants: vec![variant(
            "Llm",
            vec![field("llm", TypeRef::Named("LlmConfig".to_string()))],
        )],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let methods = gen_extendr_enum_variant_constructors(&def, &ExtendrBackend, "test_lib::Wrapper");
    let code = methods.join("\n");
    assert!(
        code.contains("let llm_core: test_lib::LlmConfig = llm.into();"),
        "{code}"
    );
    assert!(
        code.contains("test_lib::Wrapper::Llm { llm: llm_core }.into()"),
        "{code}"
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
    // per-variant constructors.
    let code = gen_extendr_json_passthrough_enum_struct(&shape_enum(), &ExtendrBackend, "test_lib");
    assert!(code.contains("pub fn default() -> Shape"), "{code}");
    assert!(code.contains("pub fn from_json(json: String)"), "{code}");
    assert!(code.contains("pub fn _factory_circle(radius: f64) -> Shape"), "{code}");
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
