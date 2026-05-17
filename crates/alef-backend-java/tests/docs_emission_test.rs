//! Coverage tests for Javadoc emission across every IR doc-bearing site.
//!
//! Each test seeds an `ApiSurface` with `.doc = "..."` on the field, variant,
//! or method being audited, then asserts the generated `.java` carries the
//! corresponding Javadoc paragraph at the expected position.
//!
//! These tests document the closed gaps from iter-10 Stream A4:
//! - record component (`@param` / inline field Javadoc)
//! - opaque-handle instance method (`/** ... */` above `public <ret> <name>(`)
//! - sealed-interface variant (already covered by inline `/** summary */`)
//! - plain enum constant (already covered by inline `/** summary */`)

use alef_backend_java::JavaBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef,
    TypeRef,
};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.example"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn field_with_doc(name: &str, ty: TypeRef, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: doc.to_string(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef_core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// Force the record into the multi-line emit path where per-field Javadoc is
/// legal. A single-line record (`record Foo(int x, int y)`) cannot legally
/// carry per-component Javadoc, so the emitter intentionally skips field docs
/// there; we exercise the multi-line path with a long type name + many fields.
#[test]
fn record_components_carry_field_javadoc_in_multi_line_emit() {
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "VeryLongRecordNameToForceMultiLineSplit".to_string(),
            rust_path: "demo::VeryLongRecordNameToForceMultiLineSplit".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                field_with_doc(
                    "alpha_channel_value",
                    TypeRef::String,
                    "Alpha-channel value used for alpha blending.",
                ),
                field_with_doc(
                    "beta_factor_for_scaling",
                    TypeRef::Primitive(PrimitiveType::I32),
                    "Beta is the second factor.",
                ),
                field_with_doc(
                    "gamma_exponent_for_tone",
                    TypeRef::Primitive(PrimitiveType::F64),
                    "Gamma exponent for tone mapping.",
                ),
                field_with_doc(
                    "delta_offset_in_pixels",
                    TypeRef::Primitive(PrimitiveType::I32),
                    "Delta offset applied last.",
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Three-channel record with documented components.".into(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let dto = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .contains("VeryLongRecordNameToForceMultiLineSplit.java")
        })
        .expect("dto file generated");
    let body = &dto.content;
    assert!(
        body.contains("Alpha-channel value used for alpha blending."),
        "alpha doc missing:\n{body}"
    );
    assert!(body.contains("Beta is the second factor."), "beta doc missing:\n{body}");
    assert!(
        body.contains("Gamma exponent for tone mapping."),
        "gamma doc missing:\n{body}"
    );
}

#[test]
fn opaque_handle_instance_method_emits_javadoc_above_signature() {
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Engine".into(),
            rust_path: "demo::Engine".into(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "run".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: Some("DemoError".into()),
                doc: "Run the engine with the given input and return the transcript.".into(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Stateful engine handle.".into(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let class = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Engine.java"))
        .expect("Engine.java generated");
    let body = &class.content;
    assert!(
        body.contains("Run the engine with the given input and return the transcript."),
        "method doc not propagated:\n{body}"
    );
    // The doc block must precede the method signature, not appear after it.
    let doc_pos = body.find("Run the engine").expect("doc present");
    let sig_pos = body.find("public String run(").expect("signature present");
    assert!(doc_pos < sig_pos, "Javadoc must precede method signature");
}

#[test]
fn plain_enum_variants_carry_summary_javadoc() {
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".into(),
            rust_path: "demo::Mode".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Prioritise throughput over fidelity.".into(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Accurate".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Prioritise fidelity over throughput.".into(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Operating mode for the engine.".into(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let mode = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Mode.java"))
        .expect("Mode.java generated");
    let body = &mode.content;
    assert!(
        body.contains("Prioritise throughput over fidelity."),
        "Fast doc missing:\n{body}"
    );
    assert!(
        body.contains("Prioritise fidelity over throughput."),
        "Accurate doc missing:\n{body}"
    );
}

#[test]
fn free_function_facade_emits_javadoc_above_static_method() {
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet".into(),
            rust_path: "demo::greet".into(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "subject".into(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("DemoError".into()),
            doc: "Build a localised greeting for the supplied subject.".into(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let combined = files.iter().map(|f| f.content.as_str()).collect::<String>();
    assert!(
        combined.contains("Build a localised greeting for the supplied subject."),
        "facade fn doc not propagated. Files emitted:\n{}",
        files
            .iter()
            .map(|f| format!("--- {} ---\n{}", f.path.display(), f.content))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
