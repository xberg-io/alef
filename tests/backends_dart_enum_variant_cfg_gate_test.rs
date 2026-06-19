/// Test that enum variants carrying a `#[cfg(feature = "...")]` attribute cause the Dart
/// Rust crate generator to keep the mirror enum complete while guarding upstream-referencing
/// conversion arms.
///
/// This covers a regression where a generated Rust crate unconditionally referenced a
/// feature-gated upstream enum variant, causing `cargo check` failures when that feature was
/// inactive.
use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
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

/// Build an `ImageOutputFormat`-shaped enum with:
///   - `Native`  — no cfg (always present)
///   - `Jpeg { quality: u8 }` — no cfg (always present)
///   - `Heif { quality: u8 }` — gated behind `feature = "heic"`
fn make_image_output_format_enum() -> EnumDef {
    EnumDef {
        name: "ImageOutputFormat".to_string(),
        rust_path: "demo::ImageOutputFormat".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Native".to_string(),
                fields: vec![],
                doc: "Keep the original image format.".to_string(),
                is_default: true,
                serde_rename: None,
                is_tuple: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Jpeg".to_string(),
                fields: vec![make_field("quality", TypeRef::Primitive(PrimitiveType::U8), false)],
                doc: "JPEG output.".to_string(),
                is_default: false,
                serde_rename: None,
                is_tuple: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Heif".to_string(),
                fields: vec![make_field("quality", TypeRef::Primitive(PrimitiveType::U8), false)],
                doc: "HEIF/HEIC output. Requires the `heic` feature.".to_string(),
                is_default: false,
                serde_rename: None,
                is_tuple: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                // This is the key: the upstream variant carries `#[cfg(feature = "heic")]`.
                cfg: Some(r#"feature = "heic""#.to_string()),
                version: Default::default(),
            },
        ],
        excluded_variants: vec![],
        methods: vec![],
        doc: "Output image format for extraction.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
        has_default: false,
    }
}

fn generate_lib_rs(enum_def: EnumDef) -> String {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![enum_def],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_basic_config();
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content
        .clone()
}

fn mirror_enum_section(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("pub enum ImageOutputFormat {")
        .expect("mirror enum section must exist");
    let end = lib_rs[start..]
        .find("\n}\n\n// From<SourceT>")
        .map(|offset| start + offset + "\n}\n".len())
        .expect("mirror enum section must end before conversion impls");
    &lib_rs[start..end]
}

fn from_core_section(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("impl From<demo::ImageOutputFormat> for ImageOutputFormat")
        .expect("From<CoreType> impl must exist");
    &lib_rs[start..]
}

#[test]
fn cfg_gated_variant_keeps_mirror_variant_unconditional() {
    let lib_rs = generate_lib_rs(make_image_output_format_enum());
    let mirror_enum = mirror_enum_section(&lib_rs);

    assert!(
        mirror_enum.contains("Heif {"),
        "mirror enum must include the feature-gated variant unconditionally:\n{mirror_enum}"
    );
    assert!(
        !mirror_enum.contains(r#"#[cfg(feature = "heic")]"#),
        "mirror enum must not propagate upstream cfg guards because FRB generates \
         unconditional references to mirror variants:\n{mirror_enum}"
    );
}

#[test]
fn cfg_gated_variant_cfg_precedes_variant_in_from_core_arm() {
    let lib_rs = generate_lib_rs(make_image_output_format_enum());
    let from_core = from_core_section(&lib_rs);

    let cfg_line = from_core
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(r#"#[cfg(feature = "heic")]"#))
        .map(|(i, _)| i)
        .expect("#[cfg(feature = \"heic\")] line not found");

    let heif_arm_line = from_core
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("demo::ImageOutputFormat::Heif"))
        .map(|(i, _)| i)
        .expect("Heif match arm line not found");

    assert!(
        cfg_line < heif_arm_line,
        "#[cfg] attribute (line {cfg_line}) must precede the Heif match arm (line {heif_arm_line})",
    );
}

#[test]
fn non_cfg_variants_have_no_cfg_attribute() {
    let lib_rs = generate_lib_rs(make_image_output_format_enum());

    // The mirror enum remains unconditional. This output-only enum only needs one cfg guard:
    // the `From<CoreType>` match arm for `Heif`.
    let cfg_count = lib_rs
        .lines()
        .filter(|l| l.contains(r#"#[cfg(feature = "heic")]"#))
        .count();
    assert_eq!(
        cfg_count, 1,
        "Expected exactly 1 occurrence of #[cfg(feature = \"heic\")]: \
         the From<Core> arm that references the upstream variant. \
         Found {cfg_count}:\n{lib_rs}",
    );
}

#[test]
fn ungated_variants_are_present_without_cfg() {
    let lib_rs = generate_lib_rs(make_image_output_format_enum());

    // Native and Jpeg must be present.
    assert!(lib_rs.contains("Native"), "Native variant missing from lib.rs");
    assert!(lib_rs.contains("Jpeg"), "Jpeg variant missing from lib.rs");
}
