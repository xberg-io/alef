/// Regression test for alef 0.25.34 bug:
/// Nullable fields with fully-qualified types in Builder emit invalid Java code.
///
/// The bug: Builder private field was emitted as:
///   @Nullable private java.nio.file.Path cacheDir;
///
/// The fix: For fully-qualified types, @Nullable must appear AT the simple-name segment:
///   @Nullable (unqualified type):   @Nullable String field;
///   @Nullable (qualified type):      java.nio.file.@Nullable Path field;
///
/// This matches Java's TYPE_USE annotation semantics and javac compilation rules.
use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_test_config(package: &str) -> ResolvedCrateConfig {
    resolved_one(&format!(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "{package}"

[crates.java.dto]
builder = "always"
"#
    ))
}

fn make_field(name: &str, ty: TypeRef, optional: bool, default: Option<&str>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: default.map(|s| s.to_string()),
        typed_default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

#[test]
fn builder_nullable_qualified_type_emits_correct_annotation_position() {
    let config = make_test_config("com.example");
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "CacheConfig".to_string(),
            rust_path: "lib::CacheConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                // Required Path field — no annotation
                make_field("base_dir", TypeRef::Path, false, None),
                // Optional Path field with #[serde(default)] — should be @Nullable with qualified type
                make_field("cache_dir", TypeRef::Path, false, Some("/* serde(default) */")),
                // Optional String field — should be @Nullable with simple type
                make_field("description", TypeRef::String, true, None),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Cache configuration with optional paths.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation failed: {:?}", result);
    let files = result.unwrap();

    // Find the CacheConfig.java file (the record + builder)
    let cache_config = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("CacheConfig.java"))
        .expect("CacheConfig.java should be generated");

    let content = &cache_config.content;

    // (1) Builder private field with @Nullable and qualified type should use correct annotation position:
    //     WRONG: @Nullable private java.nio.file.Path cacheDir;
    //     RIGHT: @Nullable private java.nio.file.@Nullable Path cacheDir;
    //     (note the @Nullable appears inside the qualified name)
    assert!(
        content.contains("java.nio.file.@Nullable Path cacheDir"),
        "Builder private field with qualified type should place @Nullable at simple-name segment. Got:\n{}",
        content
    );

    // (2) Verify that the WRONG form does NOT appear anywhere in the generated code
    assert!(
        !content.contains("@Nullable private java.nio.file.Path cacheDir"),
        "Builder private field should NOT have @Nullable before fully-qualified type. Got:\n{}",
        content
    );

    // (3) Builder setter parameter should also use correct position:
    //     WRONG: public Builder withCacheDir(final @Nullable java.nio.file.Path value)
    //     RIGHT: public Builder withCacheDir(final java.nio.file.@Nullable Path value)
    assert!(
        content.contains("java.nio.file.@Nullable Path value)"),
        "Builder setter parameter with qualified type should place @Nullable at simple-name segment. Got:\n{}",
        content
    );

    // (4) Simple type (String) should still use @Nullable at leading position:
    //     RIGHT: @Nullable private String description (for builder field)
    //     RIGHT: public Builder withDescription(final @Nullable String value) (for setter)
    assert!(
        content.contains("@Nullable private String description"),
        "Builder private field with simple type should use leading @Nullable. Got:\n{}",
        content
    );

    assert!(
        content.contains("public Builder withDescription(final @Nullable String value)"),
        "Builder setter with simple type should use leading @Nullable. Got:\n{}",
        content
    );

    // (5) Required Path field should NOT be nullable anywhere:
    assert!(
        content.contains("private java.nio.file.Path baseDir"),
        "Required Path field should not be nullable. Got:\n{}",
        content
    );
    assert!(
        !content.contains("private java.nio.file.@Nullable Path baseDir"),
        "Required Path field should not have @Nullable annotation. Got:\n{}",
        content
    );

    // (6) Verify @Nullable import is present
    assert!(
        content.contains("import org.jspecify.annotations.Nullable;"),
        "Should import @Nullable annotation. Got:\n{}",
        content
    );
}
