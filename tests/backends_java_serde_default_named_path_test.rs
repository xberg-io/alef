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
        has_serde: true, // required for builder generation
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn generate(typ: TypeDef) -> Vec<alef::core::backend::GeneratedFile> {
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
    JavaBackend
        .generate_bindings(&api, &config)
        .expect("java generation failed")
}

/// Regression test for the SsrfPolicy.deny_private bug: a non-optional field carrying
/// the named `#[serde(default = "path")]` form (stored verbatim as
/// `serde(default = "...")` in the IR) must be treated exactly like the bare
/// `#[serde(default)]` placeholder. Before the fix the Java builder emitted
/// `private boolean denyPrivate = serde(default = "default_deny_private");`, which is
/// uncompilable and crashed the Maven checkstyle parser.
#[test]
fn test_java_named_serde_default_does_not_leak_raw_attribute() {
    let field = make_field(
        "deny_private",
        TypeRef::Primitive(PrimitiveType::Bool),
        false, // non-optional
        Some("serde(default = \"default_deny_private\")".to_string()),
    );
    let files = generate(make_type("SsrfPolicy", vec![field]));

    let type_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SsrfPolicy.java"))
        .expect("SsrfPolicy.java not generated");
    let content = &type_file.content;

    // The raw serde attribute must never reach the generated Java source.
    assert!(
        !content.contains("serde(default"),
        "named serde-default attribute leaked into generated Java:\n{content}"
    );

    // The field is boxed/nullable so `null` represents "not set", letting Rust's
    // serde apply its own default (matches the bare-placeholder behaviour).
    assert!(
        content.contains("@Nullable private Boolean denyPrivate;"),
        "builder field should be a nullable boxed Boolean, but got:\n{content}"
    );

    // It must NOT be emitted as a primitive with a raw initializer.
    assert!(
        !content.contains("private boolean denyPrivate ="),
        "builder field should not be a primitive boolean with an initializer, but got:\n{content}"
    );
}
