use super::*;
use crate::core::ir::{MethodDef, ReceiverKind};

/// A stored property and a nullary method with the same name both land in the first-class
/// struct body, and Swift rejects that with `invalid redeclaration of 'providers()'`. The
/// property is emitted first and wins.
#[test]
fn first_class_struct_skips_nullary_method_colliding_with_a_property() {
    let ty = TypeDef {
        name: "LlmConfig".to_string(),
        rust_path: "demo::LlmConfig".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "providers".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "providers".to_string(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut out = String::new();
    emit_first_class_struct(
        &ty,
        &SwiftMapper,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "DemoError",
        &std::collections::HashSet::new(),
        &Default::default(),
        "AlefWireKey",
        &mut out,
    )
    .unwrap();

    assert!(
        out.contains("public let providers: String?"),
        "the stored property must still be emitted:\n{out}"
    );
    let methods = out.matches("public func providers(").count();
    assert_eq!(
        methods, 0,
        "the same-named nullary method must be skipped, found {methods}:\n{out}"
    );
}

/// Only nullary methods redeclare a property in Swift — `providers(limit:)` has a distinct
/// selector and must survive.
#[test]
fn first_class_struct_keeps_method_with_params_despite_same_base_name() {
    let ty = TypeDef {
        name: "LlmConfig".to_string(),
        rust_path: "demo::LlmConfig".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "providers".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "providers".to_string(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            params: vec![crate::core::ir::ParamDef {
                name: "limit".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut out = String::new();
    emit_first_class_struct(
        &ty,
        &SwiftMapper,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "DemoError",
        &std::collections::HashSet::new(),
        &Default::default(),
        "AlefWireKey",
        &mut out,
    )
    .unwrap();

    assert!(
        out.contains("public func providers(limit: UInt32)"),
        "a method with parameters must not be dropped:\n{out}"
    );
}

/// A field whose Rust default is a `#[serde(default = "path")]` function. Its value is not in
/// the IR, so the decoder must not invent one: `?? 0` on a `UInt32` is a silent disagreement
/// with the source crate (`default_archive_depth()` returns 1 in the crate this was found in),
/// while a plain `decode` makes an absent key a visible `DecodingError`.
#[test]
fn a_function_call_default_never_becomes_a_swift_zero() {
    let field = FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::FunctionCall("default_archive_depth".to_string())),
        ..Default::default()
    };

    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &HashSet::new(), true, false, &mut out);

    assert!(
        out.contains("try container.decode(UInt32.self, forKey: .maxArchiveDepth)"),
        "an unreadable default must leave the key required:\n{out}"
    );
    assert!(
        !out.contains("self.maxArchiveDepth = 0"),
        "the UInt32 zero is not the Rust default and must never stand in for it:\n{out}"
    );
}

/// The apparatus check for the test above. `Empty` *is* `Default::default()`, so the type zero
/// is exact there and must still be emitted — without this, the assertion above would pass
/// just as well against an emitter that had stopped producing `??` fallbacks at all.
#[test]
fn an_empty_default_still_becomes_the_swift_zero() {
    let field = FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };

    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &HashSet::new(), true, false, &mut out);

    assert!(
        out.contains("self.maxArchiveDepth = 0"),
        "`Empty` is the type's own default and keeps the zero fallback:\n{out}"
    );
}

/// `Option<T>` carrying `Some(x)` reaches the IR as the bare literal for `x` — the extractor
/// unwraps `Some` (`extract::extractor::defaults`). Decoding an absent key as `nil` therefore
/// substitutes a value the source crate never produces. 30000 is deliberately not the Swift
/// zero for the type and not `nil`, so a drop cannot look like a render.
#[test]
fn an_optional_field_defaulting_to_some_keeps_the_value() {
    let field = FieldDef {
        name: "timeout_ms".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        optional: true,
        typed_default: Some(DefaultValue::IntLiteral(30_000)),
        ..Default::default()
    };

    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &HashSet::new(), true, false, &mut out);

    assert!(
        out.contains("self.timeoutMs = 30000"),
        "`Some(30_000)` must survive into the decoder:\n{out}"
    );
    assert!(
        !out.contains("?? nil"),
        "`nil` is not what `Some(30_000)` means:\n{out}"
    );
}

/// The memberwise `init` used to spell no default at all: every defaulted field was a required
/// argument, and every optional one was `= nil` regardless of what Rust said. Both fixture
/// values differ from the Swift zero for their type, so a dropped default cannot pass as a
/// rendered one.
#[test]
fn the_memberwise_init_carries_the_rust_default() {
    let ty = TypeDef {
        name: "RetryPolicy".to_string(),
        rust_path: "demo::RetryPolicy".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            FieldDef {
                name: "attempts".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                typed_default: Some(DefaultValue::IntLiteral(7)),
                ..Default::default()
            },
            FieldDef {
                name: "timeout_ms".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: true,
                typed_default: Some(DefaultValue::IntLiteral(30_000)),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let mut out = String::new();
    emit_first_class_struct(
        &ty,
        &SwiftMapper,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "DemoError",
        &std::collections::HashSet::new(),
        &Default::default(),
        "AlefWireKey",
        &mut out,
    )
    .unwrap();

    assert!(
        out.contains("attempts: UInt32 = 7"),
        "a defaulted field must not stay a required init argument:\n{out}"
    );
    assert!(
        out.contains("timeoutMs: UInt32? = 30000"),
        "an optional field defaulting to `Some(30_000)` must not initialise to nil:\n{out}"
    );
    assert!(
        !out.contains("timeoutMs: UInt32? = nil"),
        "`nil` is not what `Some(30_000)` means:\n{out}"
    );
}

fn string_map() -> TypeRef {
    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String))
}

fn named_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn serde_struct(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        has_serde: true,
        fields,
        ..Default::default()
    }
}

#[test]
fn should_accept_a_string_keyed_map_field_when_classifying_first_class() {
    let known = HashSet::new();
    assert!(
        first_class_field_supported(&string_map(), &known),
        "`Map<String, String>` is a Swift `[String: String]`, which is Codable"
    );
    assert!(
        first_class_field_supported(&TypeRef::Optional(Box::new(string_map())), &known),
        "`Option<Map<String, String>>` must be accepted too"
    );
    let map_of_vec = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Vec(Box::new(TypeRef::String))),
    );
    assert!(
        first_class_field_supported(&map_of_vec, &known),
        "a Map value recurses like any other accepted type"
    );
}

#[test]
fn should_reject_a_map_field_when_the_key_is_not_a_string() {
    let known: HashSet<String> = ["Tag".to_string()].into_iter().collect();
    let u32_keyed = TypeRef::Map(
        Box::new(TypeRef::Primitive(PrimitiveType::U32)),
        Box::new(TypeRef::String),
    );
    assert!(
        !first_class_field_supported(&u32_keyed, &known),
        "Swift encodes a non-String/Int-keyed Dictionary as an array, not the serde_json object"
    );
    assert!(
        !first_class_field_supported(
            &TypeRef::Map(Box::new(TypeRef::Named("Tag".to_string())), Box::new(TypeRef::String)),
            &known
        ),
        "a generated enum is Hashable but not CodingKeyRepresentable, so it hits the array encoding"
    );
}

#[test]
fn should_reject_a_string_keyed_map_when_the_value_type_is_unknown() {
    let known = HashSet::new();
    assert!(
        !first_class_field_supported(
            &TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Mystery".to_string()))
            ),
            &known
        ),
        "an unknown Named value is still a typealias to an opaque class and cannot be Codable"
    );
    assert!(
        !first_class_field_supported(
            &TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Bytes)),
            &known
        ),
        "the value side must keep the same rejections as any other field position"
    );
}

/// Apparatus check for the two tests above: widening `Map` must not have widened anything
/// else. Without this, an emitter that had started accepting every type would pass them.
#[test]
fn should_still_reject_path_bytes_duration_char_and_json_fields() {
    let known = HashSet::new();
    for ty in [
        TypeRef::Path,
        TypeRef::Bytes,
        TypeRef::Duration,
        TypeRef::Char,
        TypeRef::Json,
        TypeRef::Unit,
    ] {
        assert!(
            !first_class_field_supported(&ty, &known),
            "{ty:?} has no idiomatic Swift stored-property form and must still fall back"
        );
    }
}

/// The shipped defect: `HtmlMetadata` carries three `BTreeMap<String, String>` fields, so it
/// was not first-class, so `FormatMetadata::Html(HtmlMetadata)` failed
/// `all_variants_codable_safe` and *all* of the enum's variants collapsed into a single
/// payload-erased `typealias`. Both halves are asserted: the struct must enter the
/// first-class set, and the enum that carries it must survive as a real Swift enum.
#[test]
fn should_keep_a_data_enum_first_class_when_a_variant_payload_carries_map_fields() {
    let html_metadata = serde_struct(
        "HtmlMetadata",
        vec![
            named_field("title", TypeRef::String),
            named_field("meta_tags", string_map()),
            named_field("open_graph", string_map()),
            named_field("twitter_card", string_map()),
        ],
    );
    let api = ApiSurface {
        types: vec![html_metadata],
        enums: vec![crate::core::ir::EnumDef {
            name: "FormatMetadata".to_string(),
            rust_path: "demo::FormatMetadata".to_string(),
            has_serde: true,
            variants: vec![crate::core::ir::EnumVariant {
                name: "Html".to_string(),
                is_tuple: true,
                fields: vec![named_field("0", TypeRef::Named("HtmlMetadata".to_string()))],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let known = compute_first_class_dto_names(&api, &HashSet::new());

    assert!(
        known.contains("HtmlMetadata"),
        "a struct whose only exotic fields are String-keyed maps must be first-class, got {known:?}"
    );
    assert!(
        super::super::enums::all_variants_codable_safe(&api.enums[0], &known),
        "the payload struct being first-class is what keeps the enum's variants from collapsing"
    );
}

/// A `Map` field must render as a Swift `Dictionary` property and be read back through the
/// getter's serde_json `String`, which is what `needs_json_bridge_for_swift` already routes
/// it to. A property typed `RustBridge.…` or an `rb.metaTags()` read with no JSON decode
/// would both compile-fail in the consuming package, so both are pinned.
#[test]
fn should_render_a_map_field_as_a_swift_dictionary_property() {
    let ty = serde_struct(
        "HtmlMetadata",
        vec![
            named_field("meta_tags", string_map()),
            named_field("title", TypeRef::String),
        ],
    );

    let mut out = String::new();
    emit_first_class_struct(
        &ty,
        &SwiftMapper,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "DemoError",
        &std::collections::HashSet::new(),
        &Default::default(),
        "AlefWireKey",
        &mut out,
    )
    .unwrap();

    assert!(
        out.contains("public let metaTags: [String: String]"),
        "the Map field must be a Swift Dictionary stored property:\n{out}"
    );
    assert!(
        out.contains("self.metaTags = try JSONDecoder().decode([String: String].self"),
        "the FFI init must JSON-decode the getter's serde_json String:\n{out}"
    );
}

/// GH#1594: `CodeDataNode { children: Vec<CodeDataNode>, .. }` is legitimately recursive --
/// `struct Node: Codable { var children: [Node] }` compiles in Swift because `Array` boxes
/// its elements. The fixed-point loop in `compute_first_class_dto_names` seeds only serde
/// enums, so without `is_self_reference_through_indirection` a type can never enter `known`
/// before checking its own fields, and a self-referencing struct is permanently excluded no
/// matter how many iterations run.
#[test]
fn a_struct_recursing_only_through_vec_of_self_stays_first_class() {
    let node = serde_struct(
        "CodeDataNode",
        vec![
            named_field("key", TypeRef::String),
            named_field(
                "children",
                TypeRef::Vec(Box::new(TypeRef::Named("CodeDataNode".to_string()))),
            ),
        ],
    );
    let api = ApiSurface {
        types: vec![node],
        ..Default::default()
    };

    let known = compute_first_class_dto_names(&api, &HashSet::new());

    assert!(
        known.contains("CodeDataNode"),
        "Vec<Self> is Swift-legal recursion and must not block first-class status: {known:?}"
    );
}

/// A *bare* self-reference (`Option<Self>` etc.) is not Swift-legal for a struct — Swift's
/// value-type `Optional` does not indirect, so `struct Foo { var next: Foo? }` fails to
/// compile. `is_self_reference_through_indirection` must not treat this the same as the
/// `Vec<Self>` case above; the type stays excluded.
#[test]
fn a_struct_recursing_through_a_bare_optional_self_is_not_first_class() {
    let node = serde_struct(
        "LinkedNode",
        vec![named_field(
            "next",
            TypeRef::Optional(Box::new(TypeRef::Named("LinkedNode".to_string()))),
        )],
    );
    let api = ApiSurface {
        types: vec![node],
        ..Default::default()
    };

    let known = compute_first_class_dto_names(&api, &HashSet::new());

    assert!(
        !known.contains("LinkedNode"),
        "a bare Optional<Self> is not Swift-legal recursion and must stay excluded: {known:?}"
    );
}

/// GH#1594's second blocker: `DocxMetadata.custom_properties: Option<HashMap<String,
/// serde_json::Value>>`. `serde_json::Value` alone stays rejected (`TypeRef::Json` has no
/// Codable Swift form), but as a Map *value* it substitutes the Codable `JSONValue` stand-in,
/// so the field — and the struct carrying it — must stay first-class.
#[test]
fn a_map_with_arbitrary_json_values_stays_first_class() {
    let map_of_json = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    let known = HashSet::new();
    assert!(
        first_class_field_supported(&map_of_json, &known),
        "Map<String, Json> has a Codable JSONValue stand-in and must be accepted"
    );
    assert!(
        first_class_field_supported(&TypeRef::Optional(Box::new(map_of_json)), &known),
        "Option<Map<String, Json>> must be accepted too"
    );

    let docx_metadata = serde_struct(
        "DocxMetadata",
        vec![named_field(
            "custom_properties",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
        )],
    );
    let api = ApiSurface {
        types: vec![docx_metadata],
        ..Default::default()
    };
    let known = compute_first_class_dto_names(&api, &HashSet::new());
    assert!(
        known.contains("DocxMetadata"),
        "a struct whose only exotic field is Map<String, Json> must be first-class: {known:?}"
    );
}

/// The Map<String, Json> value position must render as `JSONValue`, not the `String` a bare
/// Json leaf renders as (`SwiftMapper::json()`) — `[String: String]` would throw decoding the
/// first non-string JSON value the real wire format actually carries.
#[test]
fn should_render_a_json_valued_map_field_using_the_json_value_stand_in() {
    let ty = serde_struct(
        "DocxMetadata",
        vec![named_field(
            "customProperties",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
        )],
    );

    let mut out = String::new();
    emit_first_class_struct(
        &ty,
        &SwiftMapper,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "DemoError",
        &std::collections::HashSet::new(),
        &Default::default(),
        "AlefWireKey",
        &mut out,
    )
    .unwrap();

    assert!(
        out.contains("public let customProperties: [String: JSONValue]"),
        "the Map<String, Json> field must be a [String: JSONValue] stored property:\n{out}"
    );
    assert!(
        !out.contains("[String: String]"),
        "the Json value must not fall back to the leaf-JSON String convention:\n{out}"
    );
}
