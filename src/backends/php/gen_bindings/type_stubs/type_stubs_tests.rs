mod kwargs_constructor_stub_tests;

use super::{
    StubConstructorShape, gen_data_enum_property_declarations, gen_data_enum_variant_constructor_stubs, gen_enum_stub,
    gen_kwargs_constructor_stub_params, gen_kwargs_property_declarations, gen_struct_constructor_stub_params,
    struct_needs_from_json_stub, stub_constructor_shape,
};
use crate::backends::php::gen_bindings::functions::has_unsupported_static_params;
use crate::core::ir::{
    CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};
use ahash::AHashSet;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
    let stubs = gen_data_enum_variant_constructor_stubs(&shape_enum(), &AHashSet::new(), true).join("");

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

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

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

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

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

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

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

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

    assert!(!stubs.contains("function empty("), "{stubs}");
    assert!(!stubs.contains("function pair("), "{stubs}");
    assert!(!stubs.contains("function hidden("), "{stubs}");
    assert!(!stubs.contains("function raw("), "{stubs}");
    assert!(
        stubs.contains("public static function real(string $value): Shape"),
        "{stubs}"
    );
}

/// The stub must not document a static factory the runtime drops for the identical reason
/// (`gen_flat_data_enum_variant_constructors`, `gen_bindings/types/enums.rs`): a FOREIGN
/// `#[cfg(...)]`-gated variant's factory builds `core_path::<Variant>` directly with no
/// compile-safe fallback. `is_host_enum: false` here mirrors a `core_import` that does not
/// prefix-match the enum's `rust_path`.
#[test]
fn drops_stub_for_foreign_cfg_gated_variant() {
    let mut gated = variant(
        "Rect",
        vec![field("width", TypeRef::Primitive(PrimitiveType::U32), false)],
    );
    gated.cfg = Some(r#"feature = "extra-shapes""#.to_string());
    let def = enum_def(
        "Shape",
        vec![
            variant(
                "Circle",
                vec![field("radius", TypeRef::Primitive(PrimitiveType::F64), false)],
            ),
            gated,
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), false).join("");

    assert!(!stubs.contains("function rect("), "{stubs}");
    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
}

/// Control: the identical gate on a HOST-owned enum must never be dropped from the stub, matching
/// `enum_variant_declaration`'s authority (a host-owned gate never resolves to `Drop`).
#[test]
fn keeps_stub_for_host_owned_cfg_gated_variant() {
    let mut gated = variant(
        "Rect",
        vec![field("width", TypeRef::Primitive(PrimitiveType::U32), false)],
    );
    gated.cfg = Some(r#"feature = "extra-shapes""#.to_string());
    let def = enum_def(
        "Shape",
        vec![
            variant(
                "Circle",
                vec![field("radius", TypeRef::Primitive(PrimitiveType::F64), false)],
            ),
            gated,
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

    assert!(
        stubs.contains("public static function rect(int $width): Shape"),
        "a host-owned cfg-gated variant's factory stub must stay:\n{stubs}"
    );
    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
}

/// Regression for the `ContentPart` bug: a hand-written inherent static method
/// (`enum_def.methods`, extracted from a separate `impl EnumType { .. }` block) is never forwarded
/// into the generated `#[php_impl]` block, so suppressing the derived factory stub on a name
/// collision left the stub disagreeing with (and hiding) a reachable runtime method. The stub must
/// declare a factory for every data-carrying variant, matching `gen_flat_data_enum_variant_constructors`.
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

    let stubs = gen_data_enum_variant_constructor_stubs(&def, &AHashSet::new(), true).join("");

    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
    assert!(
        stubs.contains("public static function rect(int $width, int $height): Shape"),
        "{stubs}"
    );
}

/// Regression: the real extension (`gen_bindings/types/structs.rs`'s `use_from_json` gate)
/// emits `#[php(name = "from_json")]` for a struct with a non-scalar (named/complex) field in a
/// serde-capable binding crate, since `#[php(constructor)]` can't represent that field. The PHPStan
/// stub must declare the same static constructor or the method is invisible to editors and static
/// analysis even though it's the only way to build the type's nested config from PHP.
///
/// Positive control for the common case — crate serde AND the core type's own derives both present,
/// which is every real consumer crate today.
#[test]
fn needs_from_json_stub_for_struct_with_named_field() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        ..Default::default()
    };

    assert!(struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), true));
}

/// Direction 1 of the crate-probe/per-type disagreement: the binding crate HAS serde but the core
/// type derives nothing. `gen_php_struct` still stamps `#[derive(serde::Serialize,
/// serde::Deserialize)]` on the generated MIRROR struct (it gates that derive on the crate-level
/// probe alone), and `from_json` deserializes the mirror — never the core type — so the extension
/// really does expose `from_json` here. Keying the stub on `TypeDef::has_serde` hid it.
#[test]
fn needs_from_json_stub_when_only_the_crate_has_serde() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        has_serde: false,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        ..Default::default()
    };

    assert!(struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), true));
}

/// Direction 2: the core type derives `Serialize`/`Deserialize` but the binding crate has no serde
/// dependency, so `gen_php_struct` emits a mirror with no `Deserialize` and
/// `gen_struct_methods_impl` emits no `from_json` at all. The type's own derives cannot conjure one.
#[test]
fn does_not_need_from_json_stub_when_only_the_type_has_serde() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        ..Default::default()
    };

    assert!(!struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), false));
}

/// A struct with only scalar fields, no `Default` impl, and no field defaults is fully
/// constructible via `#[php(constructor)]` alone — the extension does not emit `from_json`
/// for it, so the stub must not claim one exists either.
#[test]
fn does_not_need_from_json_stub_for_plain_scalar_struct() {
    let typ = TypeDef {
        name: "Point".to_string(),
        has_serde: true,
        fields: vec![
            field("x", TypeRef::Primitive(PrimitiveType::F64), false),
            field("y", TypeRef::Primitive(PrimitiveType::F64), false),
        ],
        ..Default::default()
    };

    assert!(!struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), true));
}

/// Regression for the enum-branch copy of the stub/runtime disagreement: `gen_flat_data_enum_methods`
/// adds a `#[php(name = "from_json")]` to EVERY flat data enum's `#[php_impl]` block, unconditionally
/// — the flat mirror struct's `serde::Serialize`/`Deserialize` derives are stamped by
/// `php_flat_enum_struct_start.jinja` itself, not by the crate-level `php_serde_available` probe the
/// struct path keys on, so there is no serde gate to mirror here. The stub's own enum loop rendered
/// only per-variant factories, leaving `Message::from_json(..)` and `ResponseFormat::from_json(..)`
/// undefined for PHPStan while the extension defined them.
#[test]
fn data_enum_stub_declares_from_json_like_the_runtime() {
    let stub = gen_enum_stub(
        &shape_enum(),
        &AHashSet::new(),
        "crate",
        &std::collections::HashSet::new(),
    );

    assert!(
        stub.contains("public static function from_json(string $json): self"),
        "{stub}"
    );
}

/// Positive control for the gate above: `rust_bindings.rs` routes a non-tagged enum to
/// `gen_enum_constants`, which emits class constants and NO `from_json`. Without this the fix could
/// have been "emit `from_json` for every enum" and still passed.
#[test]
fn unit_variant_enum_stub_declares_no_from_json() {
    let def = enum_def("Level", vec![variant("Low", vec![]), variant("High", vec![])]);

    let stub = gen_enum_stub(&def, &AHashSet::new(), "crate", &std::collections::HashSet::new());

    assert!(!stub.contains("from_json"), "{stub}");
}

fn snake_case_unit_enum() -> EnumDef {
    let mut def = enum_def(
        "BatchStatus",
        vec![variant("InProgress", vec![]), variant("Failed", vec![])],
    );
    def.serde_rename_all = Some("snake_case".to_string());
    def
}

/// The stub for a unit-variant enum must describe what the runtime actually registers: a
/// constants-only class (`gen_enum_constants`, `types/enums.rs`), not a native PHP 8.1 `enum`.
/// `BatchStatus::InProgress` (a native enum case) does not exist at runtime -- PHP class constants
/// are case-sensitive and an enum-case object is not a string -- so a stub declaring
/// `enum ... : string` describes an API the extension never provides, and a static analyser then
/// reports the *correct* call (`BatchStatus::INPROGRESS`) as an error and the *broken* one as fine.
///
/// Negative control folded in: a generator that emitted BOTH the class-with-constants shape AND
/// the native-enum shape (i.e. "emits everything") would still fail this test on the `enum` /
/// `case` absence assertions below, so the presence assertions alone cannot pass vacuously. ~keep
#[test]
fn unit_variant_enum_stub_declares_a_constants_class_not_a_native_enum() {
    let def = snake_case_unit_enum();

    let stub = gen_enum_stub(&def, &AHashSet::new(), "crate", &std::collections::HashSet::new());

    assert!(stub.contains("final class BatchStatus"), "{stub}");
    assert!(stub.contains("public const INPROGRESS = 'in_progress';"), "{stub}");
    assert!(stub.contains("public const FAILED = 'failed';"), "{stub}");
    assert!(!stub.contains("enum BatchStatus"), "{stub}");
    assert!(!stub.contains("case "), "{stub}");
}

/// The stub's constant names and values must be identical to what the runtime actually registers,
/// not merely "some constants" -- so this cross-checks directly against `gen_enum_constants`
/// (the runtime `#[php_impl]` generator) instead of duplicating expected strings that could drift
/// from the runtime independently of the stub.
#[test]
fn unit_variant_enum_stub_constants_match_gen_enum_constants_exactly() {
    use crate::backends::php::gen_bindings::types::gen_enum_constants;

    let def = snake_case_unit_enum();

    let stub = gen_enum_stub(&def, &AHashSet::new(), "crate", &std::collections::HashSet::new());
    let runtime = gen_enum_constants(&def, None, false, None);

    let runtime_consts: Vec<&str> = runtime
        .lines()
        .filter(|l| l.trim_start().starts_with("pub const"))
        .collect();
    assert_eq!(
        runtime_consts.len(),
        def.variants.len(),
        "apparatus check: one `pub const` line must be extracted per variant, or the loop below \
         asserts nothing. Extracted {runtime_consts:?} from:\n{runtime}"
    );

    for line in runtime_consts {
        // `pub const INPROGRESS: &str = "in_progress";` (runtime) -> `public const INPROGRESS =
        // 'in_progress';` (stub).
        let rest = line.trim_start().strip_prefix("pub const ").expect("pub const prefix");
        let (name, rest) = rest.split_once(':').expect("typed const declaration");
        let value = rest
            .split_once('"')
            .and_then(|(_, r)| r.split_once('"'))
            .expect("quoted value")
            .0;
        let expected = format!("public const {name} = '{value}';");
        assert!(
            stub.contains(&expected),
            "expected `{expected}` (derived from runtime line `{line}`) in stub:\n{stub}"
        );
    }
}

/// `escape_php_reserved_constant` (shared via `enum_constant_entries`) must apply to the stub's
/// constants exactly as it does to the runtime's -- a variant literally named `Default` or `Class`
/// would otherwise emit a PHP-reserved constant name and fail to parse.
#[test]
fn unit_variant_enum_stub_escapes_reserved_word_constant_names() {
    let def = enum_def("Mode", vec![variant("Default", vec![]), variant("Class", vec![])]);

    let stub = gen_enum_stub(&def, &AHashSet::new(), "crate", &std::collections::HashSet::new());

    assert!(stub.contains("public const DEFAULT_ = 'Default';"), "{stub}");
    assert!(stub.contains("public const CLASS_ = 'Class';"), "{stub}");
}

/// Pins the property that made the tagged-enum-declaration/constant-stub swap safe: the stub
/// constant's value must be the serde wire value, not the Rust variant ident, even when only a
/// per-variant `serde_rename` is set (no `serde_rename_all`) -- `enum_constant_entries` must feed
/// `variant.serde_rename` into `wire_variant_value`, not skip it. Without this, a caller comparing
/// against the extension's own published constant would silently never match, and passing the
/// constant back in would fall through to the default variant.
#[test]
fn unit_variant_enum_stub_constant_value_is_serde_rename_not_ident() {
    let mut renamed = variant("InProgress", vec![]);
    renamed.serde_rename = Some("in_progress_wire".to_string());
    let def = enum_def("Status", vec![renamed, variant("Done", vec![])]);

    let stub = gen_enum_stub(&def, &AHashSet::new(), "crate", &std::collections::HashSet::new());

    assert!(stub.contains("final class Status"), "{stub}");
    assert!(stub.contains("public const INPROGRESS = 'in_progress_wire';"), "{stub}");
    assert!(!stub.contains("public const INPROGRESS = 'InProgress';"), "{stub}");
    assert!(stub.contains("public const DONE = 'Done';"), "{stub}");
}

/// `gen_struct_constructor_stub_params` types a required field via the promoted-property shape;
/// a field whose type is a unit-variant enum must be typed `string`, matching what
/// `PhpMapper::named` actually lowers it to at the FFI boundary, not the enum's own class name.
#[test]
fn struct_constructor_param_of_unit_enum_type_is_typed_string() {
    let typ = TypeDef {
        name: "Batch".to_string(),
        has_serde: true,
        fields: vec![field("status", TypeRef::Named("BatchStatus".to_string()), false)],
        ..Default::default()
    };
    let enum_names: AHashSet<String> = ["BatchStatus".to_string()].into_iter().collect();

    let params = gen_struct_constructor_stub_params(&typ, &enum_names, &AHashSet::new(), &AHashSet::new(), true);
    let joined = params.join("\n");

    assert!(joined.contains("public readonly string $status"), "{joined}");
    assert!(!joined.contains("BatchStatus"), "{joined}");
}

/// Same fix, `Kwargs` shape: `gen_kwargs_constructor_stub_params` must also type a unit-enum field
/// as a nullable `string`, not the enum's class name.
#[test]
fn kwargs_constructor_param_of_unit_enum_type_is_typed_string() {
    let typ = TypeDef {
        name: "Batch".to_string(),
        has_default: true,
        fields: vec![field("status", TypeRef::Named("BatchStatus".to_string()), false)],
        ..Default::default()
    };
    let enum_names: AHashSet<String> = ["BatchStatus".to_string()].into_iter().collect();

    let params = gen_kwargs_constructor_stub_params(&typ, &enum_names);

    assert_eq!(params, vec!["        ?string $status = null".to_string()]);
}

/// Same fix, the `Kwargs` shape's separately-declared `#[php(prop)]` properties.
#[test]
fn kwargs_property_of_unit_enum_type_is_typed_string() {
    let typ = TypeDef {
        name: "Batch".to_string(),
        has_default: true,
        fields: vec![field("status", TypeRef::Named("BatchStatus".to_string()), false)],
        ..Default::default()
    };
    let enum_names: AHashSet<String> = ["BatchStatus".to_string()].into_iter().collect();

    let declarations = gen_kwargs_property_declarations(&typ, &enum_names, false);
    let joined = declarations.join("");

    assert!(joined.contains("public string $status;"), "{joined}");
    assert!(!joined.contains("BatchStatus"), "{joined}");
}

/// End-to-end regression through the real `generate_type_stubs` pipeline (not just the isolated
/// helper functions above): a struct field whose type is a unit-variant enum must be typed
/// `string` in BOTH the promoted-constructor property AND the getter's return type, and the enum
/// itself must get the constants-only class shape -- all three derived from the one `enum_names`
/// set `generate_type_stubs` builds, so they cannot disagree with each other.
#[test]
fn stub_types_unit_enum_valued_struct_field_and_getter_as_string() {
    use crate::core::config::resolved::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;

    let status_enum = snake_case_unit_enum();
    let batch = TypeDef {
        name: "Batch".to_string(),
        rust_path: "test_lib::Batch".to_string(),
        has_serde: true,
        fields: vec![field("status", TypeRef::Named("BatchStatus".to_string()), false)],
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "my-crate".to_string(),
        version: "1.0.0".to_string(),
        types: vec![batch],
        enums: vec![status_enum],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "my-crate".to_string(),
        ..ResolvedCrateConfig::default()
    };

    let files = super::generate_type_stubs(&api, &config).unwrap();
    let stub = &files[0].content;

    assert!(
        stub.contains("public readonly string $status"),
        "constructor-promoted `status` property must be typed `string`, not `BatchStatus`:\n{stub}"
    );
    assert!(
        stub.contains("function getStatus(): string"),
        "the getter's declared return type must also be `string`:\n{stub}"
    );
    assert!(!stub.contains("readonly BatchStatus"), "{stub}");
    assert!(stub.contains("final class BatchStatus"), "{stub}");
    assert!(stub.contains("public const INPROGRESS = 'in_progress';"), "{stub}");
    assert!(!stub.contains("enum BatchStatus"), "{stub}");
}

/// `#[php(getter)] pub fn get_<flat>(&self)` registers a read-only PHP PROPERTY named `<flat>`
/// (ext-php-rs strips the raw `get_` prefix, no case conversion) — not a `getFlat()` method. Every
/// flat field is `Option<T>` on the binding struct because only one variant is populated at a time,
/// so every property is nullable; the tag discriminator is the one non-nullable `string`.
#[test]
fn data_enum_stub_declares_a_readonly_property_per_flat_field() {
    let declarations = gen_data_enum_property_declarations(&shape_enum(), &AHashSet::new()).join("");

    assert!(
        declarations.contains("public readonly string $type_tag;"),
        "{declarations}"
    );
    assert!(
        declarations.contains("public readonly ?float $radius;"),
        "{declarations}"
    );
    assert!(declarations.contains("public readonly ?int $width;"), "{declarations}");
    assert!(declarations.contains("public readonly ?int $height;"), "{declarations}");
}

/// A tuple variant's flat field is named after the VARIANT (`flat_field_name`), not after the
/// positional `_0` ident, so two tuple variants do not collide on `_0`. The stub must reuse that
/// exact derivation — the runtime getter it mirrors is `get_text`, i.e. property `$text`.
#[test]
fn data_enum_property_uses_variant_derived_name_for_tuple_variants() {
    let mut text = variant("Text", vec![field("_0", TypeRef::String, false)]);
    text.is_tuple = true;
    let def = enum_def("Content", vec![text]);

    let declarations = gen_data_enum_property_declarations(&def, &AHashSet::new()).join("");

    assert!(
        declarations.contains("public readonly ?string $text;"),
        "{declarations}"
    );
    assert!(!declarations.contains("$_0"), "{declarations}");
}

/// `PhpMapper::named` lowers a unit-variant enum to `String` (ext-php-rs cannot carry a Rust enum),
/// so the runtime getter returns a plain PHP string. Typing the property as the `enum <Name>: string`
/// the stub declares elsewhere would promise a value the extension never returns.
#[test]
fn data_enum_property_types_a_string_enum_field_as_string() {
    let def = enum_def(
        "Part",
        vec![variant(
            "Image",
            vec![field("detail", TypeRef::Named("ImageDetail".to_string()), false)],
        )],
    );
    let enum_names: AHashSet<String> = ["ImageDetail".to_string()].into_iter().collect();

    let declarations = gen_data_enum_property_declarations(&def, &enum_names).join("");

    assert!(
        declarations.contains("public readonly ?string $detail;"),
        "{declarations}"
    );
    assert!(!declarations.contains("ImageDetail"), "{declarations}");
}

/// PHPStan at level max rejects a bare `array`, so map/vec properties carry a generic `@var`.
#[test]
fn data_enum_property_emits_generic_phpdoc_for_array_fields() {
    let def = enum_def(
        "CacheBackend",
        vec![variant(
            "OpenDal",
            vec![field(
                "config",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
            )],
        )],
    );

    let declarations = gen_data_enum_property_declarations(&def, &AHashSet::new()).join("");

    assert!(declarations.contains("@var ?array<string, string>"), "{declarations}");
    assert!(
        declarations.contains("public readonly ?array $config;"),
        "{declarations}"
    );
}

/// A struct with an explicit hand-written static `new` constructor keeps its own constructor
/// and must not additionally get a generated `from_json` stub.
#[test]
fn does_not_need_from_json_stub_when_explicit_static_new_exists() {
    let typ = TypeDef {
        name: "Custom".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        methods: vec![MethodDef {
            name: "new".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(!struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), true));
}

/// A `#[derive(Default)]` struct needs `from_json` even if every field is scalar, because the
/// extension's gate treats `has_default` as sufficient on its own (matching `structs.rs`).
#[test]
fn needs_from_json_stub_when_struct_has_default_impl() {
    let typ = TypeDef {
        name: "Config".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
        ..Default::default()
    };

    assert!(struct_needs_from_json_stub(&typ, &ahash::AHashSet::new(), true));
}

/// Regression for the `BudgetConfig` bug: the stub's constructor param list must sort
/// required fields before optional ones (stable, preserving relative order within each group)
/// regardless of raw field-declaration order — mirroring `BudgetConfig`'s core struct, where the
/// optional `global_limit: Option<f64>` is declared first, ahead of two required fields.
#[test]
fn required_fields_sort_before_optional_regardless_of_declaration_order() {
    let typ = TypeDef {
        name: "BudgetConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("global_limit", TypeRef::Primitive(PrimitiveType::F64), true),
            field("model_limits", TypeRef::String, false),
            field("enforcement", TypeRef::String, false),
        ],
        ..Default::default()
    };

    let params = gen_struct_constructor_stub_params(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true);
    let joined = params.join("\n");

    let model_limits_idx = joined.find("$modelLimits").expect("modelLimits param present");
    let enforcement_idx = joined.find("$enforcement").expect("enforcement param present");
    let global_limit_idx = joined.find("$globalLimit").expect("globalLimit param present");

    assert!(
        model_limits_idx < global_limit_idx && enforcement_idx < global_limit_idx,
        "required fields must precede the optional field despite declaration order: {joined}"
    );
    assert!(joined.contains("float $globalLimit = null"), "{joined}");
}

/// Regression for the `RateLimitConfig` bug: a `Duration` field on a type with a `Default` impl
/// is widened to an optional, nullable `int` param (the FFI boundary carries it as milliseconds),
/// even though the field itself is required in the IR. Previously the stub used the raw
/// `f.optional` (false) for both the type/nullability AND the sort key, so `window` rendered as
/// `public readonly float $window` (required, wrong type) and sorted ahead of the genuinely
/// optional fields — disagreeing with the runtime constructor on type, nullability, AND position.
///
/// Positive control for the widening: crate serde AND the type's own derives both present.
#[test]
fn duration_field_widened_by_default_impl_is_optional_int_and_sorts_last() {
    let typ = TypeDef {
        name: "RateLimitConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("rpm", TypeRef::Primitive(PrimitiveType::U32), true),
            field("tpm", TypeRef::Primitive(PrimitiveType::U32), true),
            field("window", TypeRef::Duration, false),
        ],
        ..Default::default()
    };

    let params = gen_struct_constructor_stub_params(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true);
    let joined = params.join("\n");

    assert!(
        joined.contains("?int $window = null"),
        "Duration field on a Default-impl type must be a nullable int, not float: {joined}"
    );
    let rpm_idx = joined.find("$rpm").expect("rpm param present");
    let window_idx = joined.find("$window").expect("window param present");
    assert!(rpm_idx < window_idx, "{joined}");
}

/// The same `Duration` widening keys on the CRATE probe, exactly as the runtime does
/// (`structs.rs`: `f.optional || (has_serde && typ.has_default && matches!(f.ty, Duration))`).
/// Without crate serde the mirror struct carries no `#[serde(default)]`/`skip_serializing_if`, the
/// runtime does not widen, and neither may the stub — the type takes the `Kwargs` shape here, so
/// the widening is observable on the separately-declared properties rather than on promoted params.
/// The core type's own derives (`has_serde: true` below) must not resurrect the widening.
#[test]
fn duration_field_is_not_widened_when_the_crate_has_no_serde() {
    let typ = TypeDef {
        name: "RateLimitConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("rpm", TypeRef::Primitive(PrimitiveType::U32), true),
            field("window", TypeRef::Duration, false),
        ],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), false),
        StubConstructorShape::Kwargs
    );

    let joined = gen_kwargs_property_declarations(&typ, &AHashSet::new(), false).join("");
    assert!(
        joined.contains("public int $window;"),
        "an un-widened Duration property stays non-nullable: {joined}"
    );
    assert!(joined.contains("public ?int $rpm;"), "{joined}");
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

/// Regression: `gen_static_method` (`functions/methods.rs`) falls back to `String::new()` — no
/// `#[php_impl]` method at all — for a static method whose params `has_unsupported_static_params`
/// flags. The PHPStan stub calls this exact function to decide whether to declare the method
/// (`type_stubs.rs`'s `non_excluded_methods` filter), so it must agree with the binding on every
/// param shape the binding can't cross, not just restate a copy of the same logic that can drift.
#[test]
fn map_param_is_unsupported_for_static_delegation() {
    let params = vec![param(
        "index",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    )];
    assert!(
        has_unsupported_static_params(&params, &AHashSet::new(), &AHashSet::new()),
        "a Map param is never delegatable — gen_static_method unconditionally bails on it"
    );
}

#[test]
fn non_opaque_non_enum_named_param_is_unsupported_for_static_delegation() {
    let params = vec![param("options", TypeRef::Named("ConversionOptions".to_string()))];
    assert!(
        has_unsupported_static_params(&params, &AHashSet::new(), &AHashSet::new()),
        "a Named param that is neither an opaque type nor a string enum can't cross the FFI \
         boundary gen_static_method builds"
    );
}

#[test]
fn opaque_or_string_enum_named_params_are_supported_for_static_delegation() {
    let opaque_types: AHashSet<String> = ["Client".to_string()].into_iter().collect();
    let string_enum_names: AHashSet<String> = ["Mode".to_string()].into_iter().collect();
    let params = vec![
        param("client", TypeRef::Named("Client".to_string())),
        param("mode", TypeRef::Named("Mode".to_string())),
    ];
    assert!(
        !has_unsupported_static_params(&params, &opaque_types, &string_enum_names),
        "opaque and string-enum Named params are exactly what gen_static_method can delegate"
    );
}

/// Regression: the PHPStan stub unconditionally emitted a full field-derived
/// `#[php(constructor)]` parameter list, even for the shapes where the real extension emits
/// something else entirely -- a field needing a named/complex param, with the type not
/// qualifying for `from_json`, gets a real, zero-param constructor that always throws
/// (`structs.rs`'s `has_named_params && !use_from_json` branch), not the field list.
///
/// "Without serde" here means the BINDING CRATE has none — the signal the runtime's `use_from_json`
/// actually reads. The fixture deliberately sets `has_serde: true` on the type: a core type that
/// derives `Serialize`/`Deserialize` still gets no `from_json` in a crate that cannot derive them on
/// the mirror struct, so the per-type flag must not rescue it out of this shape. ~keep
#[test]
fn stub_constructor_shape_is_throws_no_params_when_field_needs_named_param_without_serde() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), false),
        StubConstructorShape::ThrowsNoParams
    );
}

/// The inverse direction, and the reason this mismatch was worth fixing properly: a crate WITH
/// serde holding a type that derives none. The runtime's `use_from_json` is true (crate serde plus
/// `has_default`), every field is optional so no positional `#[php(constructor)]` is emitted, and
/// `from_json` is the extension's only constructor — `FromJsonOnly`. Keying on `TypeDef::has_serde`
/// instead produced `Kwargs`: a full list of nullable positional parameters for a `__construct` the
/// extension never defines. The wrong signal did not drop a method, it invented a constructor.
#[test]
fn stub_constructor_shape_is_from_json_only_when_only_the_crate_has_serde() {
    let typ = TypeDef {
        name: "Config".to_string(),
        has_serde: false,
        has_default: true,
        fields: vec![field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::FromJsonOnly
    );
}

/// Companion regression: `use_from_json` true (via `has_default`) with every field optional
/// means no field is a representable *required* one, so `structs.rs`'s
/// `has_representable_required` gate suppresses the positional `#[php(constructor)]`
/// entirely -- `from_json` is the extension's only constructor, and the stub must not
/// declare a positional one that doesn't exist.
#[test]
fn stub_constructor_shape_is_from_json_only_when_no_representable_required_field() {
    let typ = TypeDef {
        name: "Config".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::FromJsonOnly
    );
}

/// Same `use_from_json` gate as above, but with a representable required field present --
/// `structs.rs` emits both `from_json` AND the positional constructor, so the stub must
/// still declare the field-derived parameter list.
#[test]
fn stub_constructor_shape_is_positional_when_a_required_field_is_representable() {
    let typ = TypeDef {
        name: "Config".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("name", TypeRef::String, false),
            field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::Positional
    );
}

/// Positive control: the common case (crate serde and the type's own derives both present, no
/// named-param field, no `Default` impl) must stay `Positional` -- the shape most structs actually
/// use, and the one `gen_struct_constructor_stub_params`'s own tests already cover in detail.
#[test]
fn stub_constructor_shape_is_positional_for_the_plain_scalar_case() {
    let typ = TypeDef {
        name: "Point".to_string(),
        has_serde: true,
        fields: vec![
            field("x", TypeRef::Primitive(PrimitiveType::F64), false),
            field("y", TypeRef::Primitive(PrimitiveType::F64), false),
        ],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::Positional
    );
}

/// A `Default` type whose binding CRATE has no serde takes `structs.rs`'s final `else` branch and
/// is built by `config_gen::gen_php_kwargs_constructor` -- neither the positional
/// `#[php(constructor)]` nor `from_json`. The fixture's own `has_serde: true` must not divert it.
#[test]
fn stub_constructor_shape_is_kwargs_for_default_impl_without_serde() {
    let typ = kwargs_config_type();

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), false),
        StubConstructorShape::Kwargs
    );
}

/// A type with a hand-written static `new` gets NO generated constructor at all: `structs.rs`
/// gates its whole constructor block on `!has_explicit_static_new`. What PHP still sees is
/// ext-php-rs's own zero-arg `__construct`, which throws -- so the stub must declare the
/// zero-param throwing shape, not a field-derived parameter list for a `new(...)` that the
/// extension never defines.
#[test]
fn stub_constructor_shape_is_throws_no_params_when_type_has_explicit_static_new() {
    let typ = TypeDef {
        name: "Custom".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("name", TypeRef::String, false),
            field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        methods: vec![MethodDef {
            name: "new".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::ThrowsNoParams
    );
}

/// `RetryConfig`-shaped fixture for the `Kwargs` branch: a `#[derive(Default)]` type whose binding
/// crate has no serde (the shape's precondition, passed explicitly as `serde_available: false`).
/// `has_serde: true` on the type itself is deliberate — the per-type flag is not the signal, and a
/// fixture that set it to `false` would agree with the crate probe by accident and stop testing
/// anything. Deliberately mixes an optional field declared FIRST (so a required-before-optional
/// sort would move it), a required multi-word field (so a `to_php_name` rename would be visible),
/// and a `cfg`-gated field (which the positional shape filters out but
/// `gen_php_kwargs_constructor` keeps).
fn kwargs_config_type() -> TypeDef {
    TypeDef {
        name: "RetryConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("jitter", TypeRef::Primitive(PrimitiveType::Bool), true),
            field("max_retries", TypeRef::Primitive(PrimitiveType::U32), false),
            FieldDef {
                cfg: Some("feature = \"metrics\"".to_string()),
                ..field("metrics_label", TypeRef::String, false)
            },
        ],
        ..Default::default()
    }
}

/// The runtime constructor is `pub fn __construct(jitter: Option<bool>, max_retries: Option<u32>,
/// metrics_label: Option<String>) -> Self`, and ext-php-rs makes every `Option<T>` arg nullable
/// and omittable. The stub must reproduce that signature exactly: declaration order (NOT
/// required-first), the `cfg`-gated field included, every param `?T = null`, and the raw
/// snake_case parameter names ext-php-rs registers verbatim.
#[test]
fn kwargs_constructor_stub_matches_the_runtime_signature_exactly() {
    let params = gen_kwargs_constructor_stub_params(&kwargs_config_type(), &AHashSet::new());

    assert_eq!(
        params,
        vec![
            "        ?bool $jitter = null".to_string(),
            "        ?int $max_retries = null".to_string(),
            "        ?string $metrics_label = null".to_string(),
        ]
    );
}
