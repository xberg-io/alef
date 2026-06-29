use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> alef::core::config::ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_field(name: &str, ty: TypeRef, optional: bool, default: Option<String>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default,
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

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true, // This is important for builder generation
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn test_java_serde_default_tuple_field_uses_nullable_type_and_null_default() {
    // Regression test for the KeywordConfig.ngram_range bug.
    // When a non-optional field has #[serde(default)], the Java builder should:
    // 1. Use a boxed/nullable type (not primitive)
    // 2. Initialize to `null` (not `List.of()` for Vec, not empty collection for tuples)
    // This allows Jackson to omit the field from JSON, letting Rust's serde apply its own default.
    let backend = JavaBackend;

    let tuple_field = make_field(
        "ngram_range",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        false,                                    // non-optional
        Some("/* serde(default) */".to_string()), // has #[serde(default)]
    );
    let typ = make_type("KeywordConfig", vec![tuple_field]);

    let api = ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![typ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.demo"
"#,
    );

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "generation failed: {:?}", result.err());
    let files = result.unwrap();

    // Find the generated type file
    let type_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("KeywordConfig.java"))
        .expect("KeywordConfig.java not generated");

    let content = &type_file.content;

    // The builder should declare the field as a nullable boxed type (List<Long>).
    // Java's default null value is enough, and it lets Rust's serde default apply.
    assert!(
        content.contains("@Nullable private List<Long> ngramRange;"),
        "Builder field should be nullable without an eager collection default, but got:\n{content}"
    );

    // The builder field should NOT be initialized to List.of()
    assert!(
        !content.contains("private List<Long> ngramRange = List.of();"),
        "Builder field should not initialize Vec fields to List.of() when they have #[serde(default)], but got:\n{content}"
    );

    // Verify withNgramRange setter exists and is present
    assert!(
        content.contains("withNgramRange"),
        "Builder should have withNgramRange setter, but got:\n{content}"
    );
}
