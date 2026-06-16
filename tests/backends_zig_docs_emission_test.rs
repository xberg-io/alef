//! Coverage tests for Zig `///` doc-comment emission across every IR
//! doc-bearing site.
//!
//! iter-10 Stream A4 closed two gaps:
//! - struct field rustdoc → `///` immediately above `field: T,`
//! - enum-variant rustdoc → `///` immediately above the variant tag
//!
//! The pre-existing emitters already covered: type definition, opaque-handle
//! type, opaque-handle methods, and free functions. Tests below seed
//! `field.doc` / `variant.doc` and assert the rendered text appears between
//! the struct header and the field declaration (or above the tag) — proving
//! the doc lands at the right structural location, not just somewhere in the
//! file.

use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, FieldDef, PrimitiveType, TypeDef, TypeRef,
};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
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

fn render(api: ApiSurface) -> String {
    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let demo = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("demo.zig"))
        .expect("demo.zig generated");
    demo.content.clone()
}

#[test]
fn struct_fields_emit_zig_doc_comments_above_declaration() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".into(),
            rust_path: "demo::Config".into(),
            original_rust_path: String::new(),
            fields: vec![
                field_with_doc(
                    "threshold",
                    TypeRef::Primitive(PrimitiveType::I32),
                    "Activation threshold; values below this are rejected.",
                ),
                field_with_doc(
                    "label",
                    TypeRef::String,
                    "Human-readable label rendered on the diagnostic.",
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Configuration block for the demo subsystem.".into(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
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

    let content = render(api);
    let threshold_doc = "/// Activation threshold; values below this are rejected.";
    let label_doc = "/// Human-readable label rendered on the diagnostic.";
    assert!(
        content.contains(threshold_doc),
        "threshold field doc missing:\n{content}"
    );
    assert!(content.contains(label_doc), "label field doc missing:\n{content}");
    // Each doc must precede its field declaration.
    let threshold_doc_pos = content.find(threshold_doc).unwrap();
    let threshold_field_pos = content.find("threshold:").unwrap();
    assert!(
        threshold_doc_pos < threshold_field_pos,
        "threshold doc must precede field decl"
    );
    let label_doc_pos = content.find(label_doc).unwrap();
    let label_field_pos = content.find("label:").unwrap();
    assert!(label_doc_pos < label_field_pos, "label doc must precede field decl");
}

#[test]
fn unit_enum_variants_emit_zig_doc_comments_above_tag() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "LogLevel".into(),
            rust_path: "demo::LogLevel".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Trace".into(),
                    fields: vec![],
                    doc: "Most verbose level; intended for development only.".into(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Info".into(),
                    fields: vec![],
                    doc: "Default operational level.".into(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            doc: "Severity level for diagnostic events.".into(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = render(api);
    let trace_doc = "/// Most verbose level; intended for development only.";
    let info_doc = "/// Default operational level.";
    assert!(content.contains(trace_doc), "trace variant doc missing:\n{content}");
    assert!(content.contains(info_doc), "info variant doc missing:\n{content}");
}

#[test]
fn tagged_enum_variants_emit_zig_doc_comments() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Payload".into(),
            rust_path: "demo::Payload".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Empty".into(),
                    fields: vec![],
                    doc: "No payload present.".into(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Text".into(),
                    fields: vec![field_with_doc("0", TypeRef::String, "")],
                    is_tuple: true,
                    doc: "Textual payload carrying a UTF-8 message.".into(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            doc: "Sum-type carrying optional textual content.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("type".into()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = render(api);
    let empty_doc = "/// No payload present.";
    let text_doc = "/// Textual payload carrying a UTF-8 message.";
    assert!(content.contains(empty_doc), "Empty variant doc missing:\n{content}");
    assert!(content.contains(text_doc), "Text variant doc missing:\n{content}");
}
