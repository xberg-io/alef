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
        serde_tag: None,
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
fn emits_singleton_constructor_per_struct_variant() {
    let stub = gen_enum_stub(&shape_enum(), false, true, &std::collections::HashSet::new());

    assert!(stub.contains("  class Shape"), "{stub}");
    assert!(stub.contains("    def self.circle: (Float radius) -> Shape"), "{stub}");
    assert!(
        stub.contains("    def self.rect: (Integer width, Integer height) -> Shape"),
        "{stub}"
    );
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

/// The stub must not document a singleton constructor the runtime binding drops for the same
/// reason (`classes::gen_enum::gen_data_enum_variant_constructors`): a FOREIGN cfg-gated variant's
/// factory has no compile-safe fallback and is dropped unconditionally.
#[test]
fn drops_singleton_constructor_for_foreign_cfg_gated_variant() {
    let stub = gen_enum_stub(&cfg_shape_enum(), false, false, &std::collections::HashSet::new());

    assert!(!stub.contains("def self.rect"), "{stub}");
    assert!(stub.contains("    def self.circle: (Float radius) -> Shape"), "{stub}");
}

/// Control: a host-owned cfg-gated variant's constructor must stay documented.
#[test]
fn keeps_singleton_constructor_for_host_owned_cfg_gated_variant() {
    let stub = gen_enum_stub(&cfg_shape_enum(), false, true, &std::collections::HashSet::new());

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
    let stub = gen_enum_stub(&tagged, false, true, &std::collections::HashSet::new());
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

/// The RBS `type value` union must match the actual `Symbol` the Magnus binding's `IntoValue`
/// emits at runtime. Serde's real default (no `#[serde(rename_all = "...")]`) serializes unit
/// variants verbatim, so a stub that always snake_cases the symbol lies about the runtime type.
#[test]
fn unit_enum_stub_type_value_matches_the_verbatim_wire_symbol_without_rename_all() {
    let def = enum_def(
        "DataNodeKind",
        vec![variant("KeyValue", vec![]), variant("Sequence", vec![])],
    );
    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());
    assert!(
        stub.contains("type value = :KeyValue | :Sequence"),
        "no rename_all declared, so the stub's symbol union must be verbatim: {stub}"
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

    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());

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

    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());

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

    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());

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

    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());

    assert!(
        stub.contains("    def self.ring: (?Float radius, ?Integer width) -> Shape"),
        "{stub}"
    );
}

/// Regression for the `ContentPart` bug: the runtime magnus binding never forwards
/// `enum_def.methods` (a hand-written inherent static method from a separate
/// `impl EnumType { .. }` block) into the generated singleton methods, so skipping the derived
/// factory stub on a name collision matched a runtime binding that also dropped the constructor
/// entirely, with nothing to replace it. The stub must keep emitting the derived factory.
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

    let stub = gen_enum_stub(&def, false, true, &std::collections::HashSet::new());

    assert!(
        stub.contains("    def self.circle: (Float radius) -> Shape"),
        "circle factory stub must stay reachable despite the colliding hand-written method: {stub}"
    );
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

/// `initialize`'s keywords must accept what the generated kwargs constructor actually converts.
///
/// ~keep A genuine cross-artifact check: it generates the *extension* constructor and the *stub*
/// from one TypeDef and compares them, rather than asserting a literal on either side alone. The
/// failure it guards is quiet — `String::try_convert` on a Ruby Hash does not raise, it yields
/// `None` and the field falls back to its default, so a caller steep approved gets a silently
/// wrong value instead of an error.
#[test]
fn initialize_keywords_match_the_kwargs_constructor_contract() {
    use crate::backends::magnus::type_map::MagnusMapper;
    use crate::codegen::config_gen::gen_magnus_kwargs_constructor;
    use crate::codegen::type_mapper::TypeMapper;
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "Payload".to_string(),
        rust_path: "test_lib::Payload".to_string(),
        fields: vec![
            optional_field("payload", TypeRef::Json),
            field("rows", TypeRef::Vec(Box::new(TypeRef::Json))),
        ],
        has_default: true,
        ..Default::default()
    };

    let extension = gen_magnus_kwargs_constructor(&typ, &|ty| MagnusMapper.map_type(ty));
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };
    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );
    let initialize = stub
        .lines()
        .find(|line| line.trim_start().starts_with("def initialize:"))
        .unwrap_or_else(|| panic!("initialize must be declared:\n{stub}"));

    assert!(
        extension.contains("String::try_convert"),
        "the constructor is expected to convert a Json keyword through String:\n{extension}"
    );
    assert!(
        !initialize.contains("json_value"),
        "`{initialize}` promises a parsed document, but the constructor converts with \
         `String::try_convert`, which yields None on a Hash and falls back to the default:\n{extension}"
    );
    assert!(initialize.contains("?payload: String"), "got: {initialize}");
    assert!(initialize.contains("?rows: Array[String]"), "got: {initialize}");
}

/// A defaulted struct must still declare read-only attributes.
///
/// ~keep `attr_accessor` also declares `name=`, and the binding defines no writer for any field —
/// `gen_field_accessor` emits a single zero-arity getter and there is no registration template for
/// a setter. Declaring writers let steep green-light an assignment that raises `NoMethodError`.
#[test]
fn defaulted_struct_attributes_are_read_only() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "Options".to_string(),
        rust_path: "test_lib::Options".to_string(),
        fields: vec![field("retries", TypeRef::Primitive(PrimitiveType::U32))],
        has_default: true,
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );

    assert!(
        stub.contains("attr_reader retries: Integer"),
        "a defaulted struct's fields must be readable:\n{stub}"
    );
    assert!(
        !stub.contains("attr_accessor"),
        "the binding defines no setter, so no attribute may declare one:\n{stub}"
    );
}

/// The declared type of each attribute, as it appears in the emitted stub.
fn declared_attr_types(stub: &str) -> std::collections::BTreeMap<String, String> {
    stub.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("attr_accessor ")
                .or_else(|| line.strip_prefix("attr_reader "))?;
            let (name, declared) = rest.split_once(": ")?;
            Some((name.to_string(), declared.to_string()))
        })
        .collect()
}

/// Every declared accessor type must describe the accessor the binding actually emits.
///
/// ~keep The expectation is derived from `MagnusMapper` — the same mapper
/// `classes::gen_field_accessor` uses to choose the accessor's Rust return type — rather than
/// restated as a literal, so the stub and the ext cannot be edited into agreement with each other
/// while both drift away from what Ruby receives. Two independent failures are covered: `Json`
/// crossing as an already-serialized `String` (six fields shipped as `json_value` while the ext
/// returned `Option<String>`), and nullability, which follows the field rather than the owning
/// type's `has_default`.
#[test]
fn attr_types_match_the_accessor_the_binding_emits() {
    use crate::backends::magnus::type_map::MagnusMapper;
    use crate::codegen::type_mapper::TypeMapper;
    use crate::core::ir::{ApiSurface, TypeDef};

    let fields = vec![
        optional_field("payload", TypeRef::Json),
        field("rows", TypeRef::Vec(Box::new(TypeRef::Json))),
        field(
            "index",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
        ),
        optional_field("label", TypeRef::String),
        field("names", TypeRef::Vec(Box::new(TypeRef::String))),
    ];

    let typ = TypeDef {
        name: "Payload".to_string(),
        rust_path: "test_lib::Payload".to_string(),
        fields: fields.clone(),
        // A defaulted struct is the case that used to nil-wrap every attribute.
        has_default: true,
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );
    let declared = declared_attr_types(&stub);

    for f in &fields {
        // Mirrors `classes::gen_field_accessor`'s return-type choice exactly.
        let accessor_return = if f.optional {
            let inner = match &f.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                ty => ty,
            };
            MagnusMapper.optional(&MagnusMapper.map_type(inner))
        } else {
            MagnusMapper.map_type(&f.ty)
        };
        let declared = declared
            .get(&f.name)
            .unwrap_or_else(|| panic!("`{}` must be declared:\n{stub}", f.name));

        assert_eq!(
            declared.ends_with('?'),
            accessor_return.starts_with("Option<"),
            "`{}` is declared `{declared}` but the accessor returns `{accessor_return}` — \
             nullability must follow the accessor, not the owning type's `has_default`",
            f.name
        );
        assert!(
            !declared.contains("json_value"),
            "`{}` is declared `{declared}`, but the accessor returns `{accessor_return}`: the \
             binding serializes Json before Ruby sees it, so `json_value` promises a parsed \
             document that never arrives",
            f.name
        );
    }

    assert_eq!(declared.get("payload").map(String::as_str), Some("String?"));
    assert_eq!(declared.get("rows").map(String::as_str), Some("Array[String]"));
    assert_eq!(declared.get("index").map(String::as_str), Some("Hash[String, String]"));
    assert_eq!(declared.get("label").map(String::as_str), Some("String?"));
    assert_eq!(declared.get("names").map(String::as_str), Some("Array[String]"));
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

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );

    let attrs = stub.matches("attr_reader providers:").count();
    let methods = stub.matches("def providers:").count();
    assert_eq!(attrs, 1, "the attribute must still be declared once:\n{stub}");
    assert_eq!(
        methods, 0,
        "the same-named method stub must be dropped, found {methods}:\n{stub}"
    );
}

/// Regression for the dropped-static-method bug: `gen_struct_methods` (classes/mod.rs) never
/// generates a wrapper fn for a non-opaque struct's static method — the loop there skips every
/// `is_static` method with no else branch, and the type's only static registration (`new`) is a
/// synthesized kwargs constructor sourced from the struct's fields, not from `typ.methods`. A
/// stub that still declared `def self.<name>` for an arbitrary static method promised a Ruby
/// method the runtime never defined (`ConversionOptions.default`, `NodeContext.with_owned_attributes`
/// in the wild): `undefined method 'default' for class ...`.
#[test]
fn non_opaque_static_method_other_than_new_is_never_stubbed() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "test_lib::ConversionOptions".to_string(),
        fields: vec![field("retries", TypeRef::Primitive(PrimitiveType::U32))],
        methods: vec![MethodDef {
            name: "default".to_string(),
            is_static: true,
            return_type: TypeRef::Named("ConversionOptions".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );

    assert!(
        !stub.contains("def self.default"),
        "no wrapper fn backs this static method — the stub must not promise it:\n{stub}"
    );
}

/// Same bug, opaque side: `gen_opaque_struct_methods` (classes/mod.rs) also skips every
/// `is_static` method with no else branch, so an opaque type's non-constructor static method
/// (e.g. `NodeContext.with_owned_attributes`) has no generated wrapper either.
#[test]
fn opaque_static_method_other_than_variant_wrapper_new_is_never_stubbed() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "test_lib::NodeContext".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "with_owned_attributes".to_string(),
            is_static: true,
            return_type: TypeRef::Named("NodeContext".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );

    assert!(
        !stub.contains("def self.with_owned_attributes"),
        "no wrapper fn backs this static method — the stub must not promise it:\n{stub}"
    );
}

/// The one real exception: a variant-wrapper opaque type's `new` constructor gets a genuine
/// generated wrapper (`magnus_variant_wrapper_constructor` in tagged_enums.rs) and registration
/// (`has_variant_wrapper_ctor` in module_init.rs), so the stub must keep declaring it.
#[test]
fn opaque_variant_wrapper_new_constructor_is_still_stubbed() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "Circle".to_string(),
        rust_path: "test_lib::Circle".to_string(),
        is_opaque: true,
        is_variant_wrapper: true,
        methods: vec![MethodDef {
            name: "new".to_string(),
            is_static: true,
            return_type: TypeRef::Named("Circle".to_string()),
            params: vec![crate::core::ir::ParamDef {
                name: "radius".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &std::collections::HashSet::new(),
    );

    assert!(
        stub.contains("def self.new: (Float radius) -> Circle"),
        "the variant-wrapper constructor has a real generated+registered wrapper and must stay stubbed:\n{stub}"
    );
}

/// When a `[client_constructors]` override claims the type, `module_init`'s
/// `has_variant_wrapper_ctor` is false (see `functions/module_init.rs`) — the override's own
/// generator produces the wrapper instead, and nothing in `ruby_init` registers it under
/// `new`. The stub must mirror that exact exclusion, not just `is_variant_wrapper`.
#[test]
fn opaque_variant_wrapper_new_is_not_stubbed_when_client_constructor_overrides_it() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let typ = TypeDef {
        name: "Circle".to_string(),
        rust_path: "test_lib::Circle".to_string(),
        is_opaque: true,
        is_variant_wrapper: true,
        methods: vec![MethodDef {
            name: "new".to_string(),
            is_static: true,
            return_type: TypeRef::Named("Circle".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![typ],
        ..Default::default()
    };
    let client_constructor_types: std::collections::HashSet<&str> = ["Circle"].into_iter().collect();

    let stub = super::gen_stubs(
        &api,
        &crate::core::config::ResolvedCrateConfig::default(),
        "test_lib",
        false,
        &ahash::AHashMap::new(),
        &[],
        &client_constructor_types,
    );

    assert!(
        !stub.contains("def self.new"),
        "a client-constructor override means `has_variant_wrapper_ctor` is false and nothing \
         registers `new` — the stub must not promise it:\n{stub}"
    );
}
