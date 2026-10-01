//! External-type-roots merging and include-list-expansion tests, split out of
//! [`super`] (`tests.rs`), which the 1,000-line file-size cap no longer let hold
//! this concern inline. Uses `super`'s private fixture helpers (`make_typedef`,
//! `make_funcdef`, `surface_with`).

use super::*;

#[test]
fn merge_external_type_roots_imports_only_transitive_dtos() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.rs");
    std::fs::write(
        &source,
        r#"
pub struct ExternalConfig {
    pub nested: NestedConfig,
    #[cfg_attr(alef, alef(skip))]
    pub skipped: SkippedConfig,
}

impl ExternalConfig {
    pub fn method_only(&self) -> MethodOnlyConfig {
        unimplemented!()
    }
}

pub struct NestedConfig {
    pub mode: ExternalMode,
}

pub enum ExternalMode {
    Auto,
}

pub struct SkippedConfig {
    pub hidden: HiddenConfig,
}

pub struct HiddenConfig {
    pub value: String,
}

pub struct MethodOnlyConfig {
    pub value: String,
}

pub fn external_function() -> ExternalConfig {
    unimplemented!()
}
"#,
    )
    .unwrap();

    let mut surface = surface_with(vec![make_typedef("HostConfig")], vec![]);
    let config = ResolvedCrateConfig {
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![source],
            roots: vec!["ExternalConfig".to_string()],
            from_registry: false,
        }],
        ..Default::default()
    };

    merge_external_type_roots(&mut surface, &config).unwrap();

    let type_names: AHashSet<_> = surface.types.iter().map(|typ| typ.name.as_str()).collect();
    let enum_names: AHashSet<_> = surface.enums.iter().map(|enm| enm.name.as_str()).collect();

    assert!(type_names.contains("HostConfig"));
    assert!(type_names.contains("ExternalConfig"));
    assert!(type_names.contains("NestedConfig"));
    assert!(!type_names.contains("SkippedConfig"));
    assert!(!type_names.contains("HiddenConfig"));
    assert!(!type_names.contains("MethodOnlyConfig"));
    assert!(enum_names.contains("ExternalMode"));
    assert!(
        surface
            .types
            .iter()
            .find(|typ| typ.name == "ExternalConfig")
            .is_some_and(|typ| typ.methods.is_empty()),
        "external DTO methods must be stripped"
    );
    assert!(surface.functions.is_empty(), "external functions must not be merged");
}

#[test]
fn merge_external_type_roots_disambiguates_same_name_host_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.rs");
    std::fs::write(&source, "pub struct ExternalConfig { pub value: String }\n").unwrap();

    let mut surface = surface_with(vec![make_typedef("ExternalConfig")], vec![]);
    let config = ResolvedCrateConfig {
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![source],
            roots: vec!["ExternalConfig".to_string()],
            from_registry: false,
        }],
        ..Default::default()
    };

    merge_external_type_roots(&mut surface, &config).expect("qualified host and external types can coexist");

    let names: AHashSet<_> = surface.types.iter().map(|typ| typ.name.as_str()).collect();
    assert!(names.contains("ExternalConfig"));
    assert!(names.contains("ExternalExternalConfig"), "names: {names:?}");
}

#[test]
fn merge_external_type_roots_preserves_qualified_enum_references() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.rs");
    std::fs::write(
        &source,
        r#"
pub struct ConversionOptions {
    pub format: OutputFormat,
}

pub enum OutputFormat {
    Plain,
}
"#,
    )
    .unwrap();

    let mut surface = surface_with(vec![], vec![]);
    surface.enums.push(crate::core::ir::EnumDef {
        name: "OutputFormat".into(),
        rust_path: "host_core::OutputFormat".into(),
        ..Default::default()
    });
    let config = ResolvedCrateConfig {
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![source],
            roots: vec!["ConversionOptions".to_string()],
            from_registry: false,
        }],
        ..Default::default()
    };

    merge_external_type_roots(&mut surface, &config).expect("qualified enums can coexist");

    assert!(surface.enums.iter().any(|enm| enm.name == "OutputFormat"));
    assert!(
        surface.enums.iter().any(|enm| enm.name == "ExternalOutputFormat"),
        "enums: {:?}",
        surface.enums
    );
    let format_field = &surface
        .types
        .iter()
        .find(|typ| typ.name == "ConversionOptions")
        .expect("external root is merged")
        .fields[0];
    assert_eq!(
        format_field.ty,
        TypeRef::Named("ExternalOutputFormat".into()),
        "field: {format_field:?}"
    );
    assert_eq!(
        format_field.type_rust_path.as_deref(),
        Some("external_core::external::OutputFormat")
    );
}

#[test]
fn qualified_exclude_field_matches_only_its_exact_type_path() {
    let mut host = make_typedef("ConversionOptions");
    host.fields.push(crate::core::ir::FieldDef {
        name: "format".into(),
        ty: TypeRef::String,
        ..Default::default()
    });
    let mut external = make_typedef("ConversionOptions");
    external.rust_path = "external_core::options::ConversionOptions".into();
    external.fields.push(crate::core::ir::FieldDef {
        name: "format".into(),
        ty: TypeRef::String,
        ..Default::default()
    });
    let mut surface = surface_with(vec![host, external], vec![]);

    apply_exclude_fields(&mut surface, &["external_core::ConversionOptions.format".into()]);

    assert!(!surface.types[0].fields[0].binding_excluded);
    assert!(surface.types[1].fields[0].binding_excluded);
}

#[test]
fn merge_external_type_roots_excluded_field_prunes_colliding_foreign_type() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.rs");
    std::fs::write(
        &source,
        r#"
pub struct ExternalConfig {
    pub kept: KeptType,
    pub dropped: CollidingType,
}

pub struct KeptType {
    pub value: String,
}

pub struct CollidingType {
    pub value: String,
}
"#,
    )
    .unwrap();

    let mut surface = surface_with(vec![make_typedef("CollidingType")], vec![]);
    let config = ResolvedCrateConfig {
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![source],
            roots: vec!["ExternalConfig".to_string()],
            from_registry: false,
        }],
        exclude: crate::core::config::ExcludeConfig {
            fields: vec!["ExternalConfig.dropped".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };

    merge_external_type_roots(&mut surface, &config)
        .expect("excluded field must prune the colliding foreign type instead of rejecting");

    let type_names: AHashSet<_> = surface.types.iter().map(|typ| typ.name.as_str()).collect();
    assert!(type_names.contains("ExternalConfig"));
    assert!(
        type_names.contains("KeptType"),
        "non-excluded field type must still be merged"
    );
    let colliding: Vec<_> = surface.types.iter().filter(|typ| typ.name == "CollidingType").collect();
    assert_eq!(colliding.len(), 1, "no duplicate/foreign CollidingType");
    assert_eq!(
        colliding[0].rust_path, "my_crate::CollidingType",
        "host CollidingType is untouched"
    );
}

#[test]
fn merge_external_type_roots_validates_qualified_roots_by_rust_path() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("external.rs");
    std::fs::write(&source, "pub struct ExternalConfig { pub value: String }\n").unwrap();

    let mut surface = surface_with(vec![], vec![]);
    let config = ResolvedCrateConfig {
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![source],
            roots: vec!["other_core::ExternalConfig".to_string()],
            from_registry: false,
        }],
        ..Default::default()
    };

    let err = merge_external_type_roots(&mut surface, &config).unwrap_err();

    assert!(
        err.to_string()
            .contains("external type root `other_core::ExternalConfig` was not found"),
        "expected qualified root mismatch error, got: {err:#}"
    );
}

#[test]
fn extract_with_external_type_roots_keeps_host_sources_and_field_type() {
    let dir = tempfile::tempdir().unwrap();
    // `extract()` writes an IR cache under a CWD-RELATIVE `.alef/` (pipeline/extract.rs:94 ->
    // `cache::write_ir_cache` -> `ir_cache_dir()`). Without this guard the write lands in
    // whichever tempdir a concurrent `CwdGuard` holder installed, and fails with ENOENT or
    // EEXIST when that tempdir is removed mid-write -- roughly 1 full-suite run in 3. ~keep
    let _cwd = crate::test_support::CwdGuard::enter(dir.path());
    let manifest = dir.path().join("Cargo.toml");
    let host = dir.path().join("host.rs");
    let external = dir.path().join("external.rs");
    std::fs::write(&manifest, "[package]\nname = \"host\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(
        &host,
        r#"
pub struct HostConfig {
    pub external: external_core::ExternalConfig,
}
"#,
    )
    .unwrap();
    std::fs::write(
        &external,
        r#"
pub struct ExternalConfig {
    pub nested: NestedConfig,
}

pub struct NestedConfig {
    pub enabled: bool,
}
"#,
    )
    .unwrap();

    let config = ResolvedCrateConfig {
        name: "host".to_string(),
        sources: vec![host],
        source_crates: vec![SourceCrate {
            name: "external-core".to_string(),
            sources: vec![external],
            roots: vec!["external_core::ExternalConfig".to_string()],
            from_registry: false,
        }],
        version_from: manifest.to_string_lossy().into_owned(),
        include: crate::core::config::IncludeConfig {
            types: vec!["HostConfig".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };

    let api = super::super::extract(&config, &dir.path().join("alef.toml"), true).unwrap();

    let host_config = api
        .types
        .iter()
        .find(|typ| typ.name == "HostConfig")
        .expect("host type should survive extraction");
    let external_field = host_config
        .fields
        .iter()
        .find(|field| field.name == "external")
        .expect("host field should survive extraction");

    assert!(
        matches!(&external_field.ty, TypeRef::Named(name) if name == "ExternalConfig"),
        "external field should remain typed, got {:?}",
        external_field.ty
    );
    assert!(api.types.iter().any(|typ| typ.name == "ExternalConfig"));
    assert!(api.types.iter().any(|typ| typ.name == "NestedConfig"));
}

/// Regression for a batch-result include bug: a function listed in
/// `[crates.include].functions` returns a wrapper struct that is NOT in
/// `[crates.include].types`. Before the fix, the include filter dropped the
/// wrapper struct (it was unreachable from the included types), and the later
/// `sanitize_unknown_types` pass collapsed the function's `return_type` to
/// `String`, breaking every binding facade.
///
/// After the fix, `expand_include_list` seeds itself from included functions'
/// signatures so the wrapper is retained.
#[test]
fn expand_include_list_seeds_from_included_function_signatures() {
    let surface = surface_with(
        vec![
            make_typedef("BatchScrapeResult"),
            make_typedef("BatchScrapeResults"),
            make_typedef("UnusedType"),
        ],
        vec![make_funcdef(
            "batch_scrape",
            TypeRef::Named("BatchScrapeResults".into()),
            vec![TypeRef::Vec(Box::new(TypeRef::String))],
        )],
    );

    let include_types = vec!["BatchScrapeResult".to_string()];
    let include_functions = vec!["batch_scrape".to_string()];

    let expanded = expand_include_list(&surface, &include_types, &include_functions);

    assert!(
        expanded.contains("BatchScrapeResult"),
        "per-element type explicitly listed must be present; got: {expanded:?}"
    );
    assert!(
        expanded.contains("BatchScrapeResults"),
        "wrapper return type of included function must be auto-included; got: {expanded:?}"
    );
    assert!(
        !expanded.contains("UnusedType"),
        "unrelated type must not be pulled in; got: {expanded:?}"
    );
}

/// Function parameter types must also be retained — a function listed in
/// `include.functions` that accepts a custom config struct must keep that
/// struct in the surface even if the user forgot to list it under
/// `include.types`.
#[test]
fn expand_include_list_seeds_from_included_function_param_types() {
    let surface = surface_with(
        vec![make_typedef("CrawlConfig"), make_typedef("EngineHandle")],
        vec![make_funcdef(
            "create_engine",
            TypeRef::Named("EngineHandle".into()),
            vec![TypeRef::Optional(Box::new(TypeRef::Named("CrawlConfig".into())))],
        )],
    );

    let include_types = vec!["EngineHandle".to_string()];
    let include_functions = vec!["create_engine".to_string()];

    let expanded = expand_include_list(&surface, &include_types, &include_functions);

    assert!(
        expanded.contains("CrawlConfig"),
        "param type referenced through Optional must be retained; got: {expanded:?}"
    );
}

/// When no functions are in the include list, behaviour is unchanged —
/// expansion stays anchored to `include_types` only.
#[test]
fn expand_include_list_with_empty_functions_matches_legacy_behaviour() {
    let surface = surface_with(
        vec![make_typedef("Kept"), make_typedef("Dropped")],
        vec![make_funcdef("do_thing", TypeRef::Named("Dropped".into()), vec![])],
    );

    let include_types = vec!["Kept".to_string()];
    let include_functions: Vec<String> = vec![];

    let expanded = expand_include_list(&surface, &include_types, &include_functions);
    assert!(expanded.contains("Kept"));
    assert!(
        !expanded.contains("Dropped"),
        "function not in include.functions must not pull in its return type; got: {expanded:?}"
    );
}

#[test]
fn expand_include_list_does_not_follow_binding_excluded_fields() {
    let surface = surface_with(
        vec![
            crate::core::ir::TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "external_core::CrawlConfig".to_string(),
                fields: vec![
                    crate::core::ir::FieldDef {
                        name: "content".to_string(),
                        ty: TypeRef::Named("ContentConfig".to_string()),
                        ..crate::core::ir::FieldDef::default()
                    },
                    crate::core::ir::FieldDef {
                        name: "dispatch".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named("DispatchProfile".to_string()))),
                        binding_excluded: true,
                        ..crate::core::ir::FieldDef::default()
                    },
                ],
                ..make_typedef("CrawlConfig")
            },
            make_typedef("ContentConfig"),
            crate::core::ir::TypeDef {
                name: "DispatchProfile".to_string(),
                rust_path: "external_core::DispatchProfile".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "bypass".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("DynBypassProvider".to_string()))),
                    ..crate::core::ir::FieldDef::default()
                }],
                ..make_typedef("DispatchProfile")
            },
            make_typedef("DynBypassProvider"),
        ],
        vec![],
    );

    let expanded = expand_include_list(&surface, &["CrawlConfig".to_string()], &[]);

    assert!(expanded.contains("CrawlConfig"));
    assert!(expanded.contains("ContentConfig"));
    assert!(
        !expanded.contains("DispatchProfile"),
        "binding-excluded fields must not pull skipped internals into the public graph: {expanded:?}"
    );
    assert!(
        !expanded.contains("DynBypassProvider"),
        "nested internals behind binding-excluded fields must stay out: {expanded:?}"
    );
}

#[test]
fn normalize_field_type_paths_preserves_explicit_reexport_path() {
    let mut surface = surface_with(
        vec![
            crate::core::ir::TypeDef {
                name: "UrlConfig".to_string(),
                rust_path: "facade::UrlConfig".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "crawl".to_string(),
                    ty: TypeRef::Named("CrawlConfig".to_string()),
                    type_rust_path: Some("external_core::CrawlConfig".to_string()),
                    ..crate::core::ir::FieldDef::default()
                }],
                ..make_typedef("UrlConfig")
            },
            crate::core::ir::TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "external_core::types::config::CrawlConfig".to_string(),
                ..make_typedef("CrawlConfig")
            },
        ],
        vec![],
    );

    super::super::type_helpers::normalize_field_type_paths(&mut surface);

    let field = &surface.types[0].fields[0];
    assert_eq!(field.type_rust_path.as_deref(), Some("external_core::CrawlConfig"));
}

#[test]
fn qualified_field_path_restores_surviving_disambiguated_type_name() {
    let mut owner = make_typedef("PipelineStage");
    owner.fields.push(crate::core::ir::FieldDef {
        name: "engine_config".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("EngineConfig".to_string()))),
        type_rust_path: Some("host_core::types::EngineConfig".to_string()),
        ..Default::default()
    });
    let mut public_config = make_typedef("FormatsEngineConfig");
    public_config.rust_path = "host_core::types::formats::EngineConfig".to_string();
    let mut surface = surface_with(vec![owner, public_config], vec![]);

    super::super::type_helpers::resolve_qualified_field_type_names(&mut surface);
    sanitize_unknown_types(&mut surface);

    let field = &surface.types[0].fields[0];
    assert_eq!(
        field.ty,
        TypeRef::Optional(Box::new(TypeRef::Named("FormatsEngineConfig".to_string())))
    );
    assert!(!field.sanitized);
}

#[test]
fn public_reexport_field_path_wins_over_a_same_crate_short_name_collision() {
    let mut owner = make_typedef("OcrConfig");
    owner.fields.push(crate::core::ir::FieldDef {
        name: "tesseract_config".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("OcrTesseractConfig".to_string()))),
        type_rust_path: Some("xberg::types::TesseractConfig".to_string()),
        ..Default::default()
    });

    let mut public_config = make_typedef("TesseractConfig");
    public_config.rust_path = "xberg::TesseractConfig".to_string();

    let mut excluded_collision = make_typedef("OcrTesseractConfig");
    excluded_collision.rust_path = "xberg::ocr::TesseractConfig".to_string();
    excluded_collision.binding_excluded = true;

    let mut surface = surface_with(vec![owner, public_config, excluded_collision], vec![]);

    super::super::type_helpers::resolve_qualified_field_type_names(&mut surface);
    sanitize_unknown_types(&mut surface);

    let field = &surface.types[0].fields[0];
    assert_eq!(
        field.ty,
        TypeRef::Optional(Box::new(TypeRef::Named("TesseractConfig".to_string())))
    );
    assert!(!field.sanitized, "an exact public path must not degrade to a string");
}

#[test]
fn exact_excluded_field_path_stays_rust_only_in_the_binding_projection() {
    let mut owner = make_typedef("InternalOcrOptions");
    owner.fields.push(crate::core::ir::FieldDef {
        name: "tesseract_config".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("TesseractConfig".to_string()))),
        type_rust_path: Some("xberg::ocr::TesseractConfig".to_string()),
        ..Default::default()
    });

    let mut public_config = make_typedef("TesseractConfig");
    public_config.rust_path = "xberg::TesseractConfig".to_string();

    let mut excluded_collision = make_typedef("OcrTesseractConfig");
    excluded_collision.rust_path = "xberg::ocr::TesseractConfig".to_string();
    excluded_collision.binding_excluded = true;

    let mut surface = surface_with(vec![owner, public_config, excluded_collision], vec![]);
    super::super::type_helpers::resolve_qualified_field_type_names(&mut surface);

    let source_field = &surface.types[0].fields[0];
    assert_eq!(
        source_field.ty,
        TypeRef::Optional(Box::new(TypeRef::Named("OcrTesseractConfig".to_string())))
    );

    let projected = crate::codegen::binding_projection::project_owned(surface);
    assert!(projected.types.iter().all(|typ| typ.name != "OcrTesseractConfig"));
    let projected_field = &projected.types[0].fields[0];
    assert_eq!(projected_field.ty, TypeRef::Optional(Box::new(TypeRef::String)));
    assert!(
        projected_field.sanitized,
        "the excluded internal type must not enter bindings"
    );
}
