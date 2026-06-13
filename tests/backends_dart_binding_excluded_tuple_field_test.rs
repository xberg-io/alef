use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, ErrorDef, ErrorVariant, FieldDef, MethodDef, TypeRef};

fn make_binding_excluded_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
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
        binding_excluded: true,
        binding_exclusion_reason: Some("type does not support bindings".to_string()),
        original_type: None,
    }
}

fn make_api_with_binding_excluded_error() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "UnitVariant".to_string(),
                    message_template: Some("unit variant".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Unit variant for testing.".to_string(),
                },
                ErrorVariant {
                    name: "TupleWithExcludedField".to_string(),
                    message_template: Some("tuple with excluded field".to_string()),
                    // A tuple variant where the single field is binding-excluded.
                    // The field type is a synthetic non-Default type like serde_json::Error.
                    fields: vec![make_binding_excluded_field(
                        "",
                        TypeRef::Named("serde_json::Error".to_string()),
                    )],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: true,
                    doc: "Tuple variant with a binding-excluded field.".to_string(),
                },
                ErrorVariant {
                    name: "StructWithExcludedField".to_string(),
                    message_template: Some("struct with excluded field".to_string()),
                    // A struct variant where all fields are binding-excluded.
                    fields: vec![make_binding_excluded_field(
                        "error",
                        TypeRef::Named("serde_json::Error".to_string()),
                    )],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: "Struct variant with binding-excluded fields.".to_string(),
                },
            ],
            doc: "Error enum with binding-excluded fields.".to_string(),
            // Add a method so the From impl gets emitted
            methods: vec![MethodDef {
                name: "message".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Get the error message.".to_string(),
                receiver: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
    ..Default::default()
}        ..Default::default()
    }
}

fn make_basic_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
version_from = "/nonexistent/Cargo.toml"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn snapshot_binding_excluded_tuple_field_generates_unreachable() {
    let api = make_api_with_binding_excluded_error();
    let config = make_basic_config();
    let files = DartBackend.generate_bindings(&api, &config).unwrap();

    // Find the Dart crate Rust file (should contain the mirror conversion impl)
    let rust_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("should have generated a lib.rs file");

    let content = &rust_file.content;

    // Verify that unreachable!() is emitted for the binding-excluded tuple variant,
    // not Default::default()
    assert!(
        content.contains("unreachable!(\"variant with binding-excluded fields cannot be constructed on dart side\")"),
        "Expected unreachable!() for tuple variant with binding-excluded field, but got:\n{}",
        content
    );

    // Verify that Default::default() is NOT emitted for the binding-excluded tuple variant
    let lines: Vec<&str> = content.lines().collect();
    let mut in_tuple_variant_arm = false;
    for (i, line) in lines.iter().enumerate() {
        if line.contains("TupleWithExcludedField") && line.contains("=>") {
            in_tuple_variant_arm = true;
        }
        if in_tuple_variant_arm && line.contains("Default::default()") {
            panic!(
                "Found Default::default() in TupleWithExcludedField arm at line {}: {}",
                i + 1,
                line
            );
        }
        if in_tuple_variant_arm && line.trim().ends_with(",") {
            in_tuple_variant_arm = false;
        }
    }

    // Similarly check the struct variant with binding-excluded field
    in_tuple_variant_arm = false;
    for (i, line) in lines.iter().enumerate() {
        if line.contains("StructWithExcludedField") && line.contains("=>") {
            in_tuple_variant_arm = true;
        }
        if in_tuple_variant_arm && line.contains("Default::default()") {
            panic!(
                "Found Default::default() in StructWithExcludedField arm at line {}: {}",
                i + 1,
                line
            );
        }
        if in_tuple_variant_arm && line.trim().ends_with(",") {
            in_tuple_variant_arm = false;
        }
    }
}
