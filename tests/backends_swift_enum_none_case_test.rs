//! A Rust enum variant named `None` must not become the Swift case `none`.
//!
//! On a field of type `Effort?`, `.none` means "no value" (`Optional.none`), not the enum case.
//! A caller who writes `effort: .none` sends nothing, and Swift reports only a warning. The Swift
//! case gets a different name; the wire value stays `"none"`.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn unit_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        ..Default::default()
    }
}

fn request_type() -> TypeDef {
    let effort = FieldDef {
        version: Default::default(),
        name: "effort".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("Effort".to_string()))),
        optional: true,
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
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    TypeDef {
        name: "Request".to_string(),
        rust_path: "demo::Request".to_string(),
        original_rust_path: String::new(),
        fields: vec![effort],
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
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn api_with_enum(effort: EnumDef) -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![request_type()],
        functions: vec![],
        enums: vec![effort],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn swift_source(effort: EnumDef) -> String {
    let files = SwiftBackend
        .generate_bindings(&api_with_enum(effort), &make_config())
        .expect("generate must succeed");
    files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(".swift"))
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every line of `src` that declares or selects the Swift case `none` by its bare name.
fn bare_none_lines(src: &str) -> Vec<&str> {
    src.lines()
        .filter(|line| {
            let t = line.trim();
            t == "case none"
                || t.starts_with("case none =")
                || t.starts_with("case none:")
                || t.contains("= .none")
                || t.contains("case .none")
        })
        .collect()
}

fn effort_unit_enum() -> EnumDef {
    EnumDef {
        name: "Effort".to_string(),
        rust_path: "demo::Effort".to_string(),
        variants: vec![unit_variant("Low"), unit_variant("High"), unit_variant("None")],
        has_serde: true,
        serde_rename_all: Some("lowercase".to_string()),
        ..Default::default()
    }
}

#[test]
fn unit_enum_variant_none_gets_a_swift_case_other_than_none() {
    let src = swift_source(effort_unit_enum());

    assert!(
        src.contains("public enum Effort: String"),
        "sanity check: the Effort enum must be emitted:\n{src}"
    );
    assert_eq!(
        bare_none_lines(&src),
        Vec::<&str>::new(),
        "no generated line may declare or select the bare case `none`:\n{src}"
    );
    assert!(
        src.contains("case none_ = \"none\""),
        "the renamed case must keep the wire value \"none\":\n{src}"
    );
}

#[test]
fn data_enum_variant_none_keeps_its_wire_tag_under_a_new_case_name() {
    let payload = EnumVariant {
        name: "Budget".to_string(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::String,
            ..request_type().fields.remove(0)
        }],
        ..Default::default()
    };
    let en = EnumDef {
        name: "Effort".to_string(),
        rust_path: "demo::Effort".to_string(),
        variants: vec![unit_variant("Low"), unit_variant("None"), payload],
        has_serde: true,
        serde_rename_all: Some("lowercase".to_string()),
        ..Default::default()
    };
    let src = swift_source(en);

    assert_eq!(
        bare_none_lines(&src),
        Vec::<&str>::new(),
        "no generated line may declare or select the bare case `none`:\n{src}"
    );
    assert!(
        src.contains("case \"none\":") && src.contains("self = .none_"),
        "the Codable body must decode the wire tag \"none\" into the renamed case:\n{src}"
    );
}
