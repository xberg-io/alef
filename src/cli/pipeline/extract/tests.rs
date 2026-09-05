use super::external_types::merge_external_type_roots;
use super::filtering::{
    apply_exclude_fields, apply_filters, expand_include_list, is_type_excluded, redundant_generic_exclude_entries,
    unmatched_exclude_entries,
};
use super::sanitizer::{TypeSanitization, sanitize_type_ref, sanitize_unknown_types};
use super::validation::validate_extracted_api;
use crate::core::config::{ResolvedCrateConfig, SourceCrate};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;

/// sanitize_type_ref must resolve Map inner types (e.g. Named("str") → String)
/// without marking the Map as lossy. Lossy map inner changes are still reported
/// separately so validation can block them before codegen.
#[test]
fn sanitize_map_with_cow_key_preserves_map_structure_and_returns_lossless() {
    let known_types = AHashSet::default();
    let known_enums = AHashSet::default();

    let mut ty = TypeRef::Map(Box::new(TypeRef::Named("str".into())), Box::new(TypeRef::Json));

    let status = sanitize_type_ref(&mut ty, &known_types, &known_enums);

    assert!(
        matches!(&ty, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
        "expected Map(String, Json) but got {ty:?}"
    );

    assert_eq!(status, TypeSanitization::Lossless);

    let _ = known_types;
    let mut ty2 = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
    let sanitized2 = sanitize_type_ref(&mut ty2, &AHashSet::default(), &AHashSet::default());
    assert_eq!(sanitized2, TypeSanitization::Unchanged);
    assert!(
        matches!(&ty2, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
        "Map(String, Json) must not be mutated when already clean"
    );
}

#[test]
fn sanitize_map_with_bare_value_is_reported_as_sanitized() {
    let mut ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Value".to_string())));

    let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());

    assert!(
        sanitized.is_lossy(),
        "ambiguous bare Value inside Map must not be silently accepted"
    );
    assert!(
        matches!(&ty, TypeRef::Map(_, value) if matches!(value.as_ref(), TypeRef::Named(name) if name == "Value")),
        "ambiguous bare Value must remain visible for validation, got {ty:?}"
    );
}

/// Map(String, String) — the old case that was already handled correctly downstream —
/// must also return sanitized=false after this fix. Backends must handle it via the
/// normal (non-sanitized) Map conversion path.
#[test]
fn sanitize_map_with_both_string_types_returns_not_sanitized() {
    let mut ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
    assert_eq!(sanitized, TypeSanitization::Unchanged);
    assert!(matches!(
        &ty,
        TypeRef::Map(k, v)
            if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String)
    ));
}

#[test]
fn sanitize_map_with_unknown_value_type_returns_lossy() {
    let mut ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("ForeignPayload".into())),
    );

    let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());

    assert_eq!(sanitized, TypeSanitization::Lossy);
    assert!(
        matches!(&ty, TypeRef::Map(_, value) if matches!(value.as_ref(), TypeRef::String)),
        "unknown map value should be visibly sanitized for validation, got {ty:?}"
    );
}

/// Primitive field (not a Map) with unknown Named inner type still gets sanitized=true.
/// This ensures we didn't break non-Map sanitization.
#[test]
fn sanitize_named_unknown_type_returns_sanitized_true() {
    let mut ty = TypeRef::Named("UnknownForeignType".into());
    let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
    assert!(sanitized.is_lossy());
    assert!(matches!(ty, TypeRef::String));
}

#[test]
fn sanitize_field_type_path_rejects_same_name_from_different_crate() {
    let mut api = ApiSurface {
        types: vec![
            crate::core::ir::TypeDef {
                name: "UrlConfig".to_string(),
                rust_path: "facade::UrlConfig".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "crawl".to_string(),
                    ty: TypeRef::Named("CrawlConfig".to_string()),
                    type_rust_path: Some("external_core::CrawlConfig".to_string()),
                    ..crate::core::ir::FieldDef::default()
                }],
                ..crate::core::ir::TypeDef::default()
            },
            crate::core::ir::TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "facade::CrawlConfig".to_string(),
                ..crate::core::ir::TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    sanitize_unknown_types(&mut api);

    let field = &api.types[0].fields[0];
    assert!(field.sanitized);
    assert!(matches!(field.ty, TypeRef::String));
}

#[test]
fn sanitize_field_type_path_accepts_same_crate_reexport_path() {
    let mut api = ApiSurface {
        types: vec![
            crate::core::ir::TypeDef {
                name: "Wrapper".to_string(),
                rust_path: "sample::Wrapper".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("CrawlConfig".to_string()),
                    type_rust_path: Some("sample::CrawlConfig".to_string()),
                    ..crate::core::ir::FieldDef::default()
                }],
                ..crate::core::ir::TypeDef::default()
            },
            crate::core::ir::TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "sample::types::config::CrawlConfig".to_string(),
                ..crate::core::ir::TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    sanitize_unknown_types(&mut api);

    let field = &api.types[0].fields[0];
    assert!(!field.sanitized);
    assert!(matches!(field.ty, TypeRef::Named(ref name) if name == "CrawlConfig"));
}

#[test]
fn sanitize_field_type_path_accepts_facade_reexport_path() {
    let mut api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![
            crate::core::ir::TypeDef {
                name: "Wrapper".to_string(),
                rust_path: "sample::Wrapper".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("CrawlConfig".to_string()),
                    type_rust_path: Some("sample::CrawlConfig".to_string()),
                    ..crate::core::ir::FieldDef::default()
                }],
                ..crate::core::ir::TypeDef::default()
            },
            crate::core::ir::TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "external_core::types::config::CrawlConfig".to_string(),
                ..crate::core::ir::TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    sanitize_unknown_types(&mut api);

    let field = &api.types[0].fields[0];
    assert!(!field.sanitized);
    assert!(matches!(field.ty, TypeRef::Named(ref name) if name == "CrawlConfig"));
}

/// Vec<Named("unknown")> should still return sanitized=true (inner Named replaced with String).
#[test]
fn sanitize_vec_with_unknown_named_returns_sanitized_true() {
    let mut ty = TypeRef::Vec(Box::new(TypeRef::Named("MyForeignStruct".into())));
    let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
    assert!(sanitized.is_lossy());
    assert!(matches!(
        &ty,
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)
    ));
}

#[test]
fn validate_extracted_api_does_not_suppress_critical_codes() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![crate::core::ir::FunctionDef {
            name: "render".to_string(),
            rust_path: "sample_lib::render".to_string(),
            original_rust_path: String::new(),
            params: vec![crate::core::ir::ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("MissingPayload".to_string()),
                ..crate::core::ir::ParamDef::default()
            }],
            return_type: TypeRef::String,
            error_type: None,
            doc: String::new(),
            is_async: false,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..ApiSurface::default()
    };

    let config = ResolvedCrateConfig::default();
    let err = validate_extracted_api(&api, &config).expect_err("must stay fatal");

    assert!(
        err.to_string().contains("unknown_named_type"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_extracted_api_honors_configured_surface_suppressions() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![crate::core::ir::FunctionDef {
            name: "render".to_string(),
            rust_path: "sample_lib::render".to_string(),
            original_rust_path: String::new(),
            params: vec![crate::core::ir::ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("MissingPayload".to_string()),
                ..crate::core::ir::ParamDef::default()
            }],
            return_type: TypeRef::String,
            ..crate::core::ir::FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        suppress_validation_codes: vec!["unknown_named_type".to_string()],
        ..ResolvedCrateConfig::default()
    };

    validate_extracted_api(&api, &config).expect("configured diagnostic should be suppressed");
}

fn make_typedef(name: &str) -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: name.to_string(),
        rust_path: format!("my_crate::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        doc: String::new(),
        cfg: None,
        serde_rename_all: None,
        has_serde: false,
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

fn make_funcdef(name: &str, return_type: TypeRef, param_types: Vec<TypeRef>) -> crate::core::ir::FunctionDef {
    crate::core::ir::FunctionDef {
        name: name.to_string(),
        rust_path: format!("my_crate::{name}"),
        original_rust_path: String::new(),
        params: param_types
            .into_iter()
            .enumerate()
            .map(|(i, ty)| crate::core::ir::ParamDef {
                name: format!("arg{i}"),
                ty,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            })
            .collect(),
        return_type,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn surface_with(types: Vec<crate::core::ir::TypeDef>, functions: Vec<crate::core::ir::FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "my_crate".into(),
        version: "0.1.0".into(),
        types,
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

mod config_entry_matching;
mod external_type_roots;
mod fixed_size_arrays;
mod ir_cache_version_salt;
mod param_provenance;
mod redundant_exclude_entries;

fn make_unsupported_method(type_name: &str, method_name: &str) -> crate::core::ir::UnsupportedPublicItem {
    crate::core::ir::UnsupportedPublicItem {
        item_kind: "method".to_string(),
        item_path: format!("my_crate::module::{type_name}.{method_name}"),
        reason: "public generic trait methods cannot be represented without explicit monomorphization metadata"
            .to_string(),
        suggested_fix: "exclude the method".to_string(),
    }
}

fn make_unsupported_function(fn_name: &str) -> crate::core::ir::UnsupportedPublicItem {
    crate::core::ir::UnsupportedPublicItem {
        item_kind: "function".to_string(),
        item_path: format!("my_crate::{fn_name}"),
        reason: "generic function".to_string(),
        suggested_fix: "exclude the function".to_string(),
    }
}

/// A method item whose `TypeName.method_name` tail appears in `exclude.methods`
/// must be removed from `unsupported_public_items`.
#[test]
fn apply_filters_removes_unsupported_method_when_excluded_by_methods_list() {
    let mut surface = surface_with(vec![], vec![]);
    surface
        .unsupported_public_items
        .push(make_unsupported_method("NodeContext", "serialize"));

    let mut config = ResolvedCrateConfig::default();
    config.exclude.methods = vec!["NodeContext.serialize".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    assert!(
        result.unsupported_public_items.is_empty(),
        "method listed in exclude.methods must be removed from unsupported_public_items; \
             remaining: {:?}",
        result.unsupported_public_items
    );
}

/// A method item whose tail is NOT in `exclude.methods` must be retained so the
/// diagnostic still surfaces as a fatal error.
#[test]
fn apply_filters_retains_unsupported_method_when_not_in_exclude_list() {
    let mut surface = surface_with(vec![], vec![]);
    surface
        .unsupported_public_items
        .push(make_unsupported_method("NodeContext", "serialize"));

    let mut config = ResolvedCrateConfig::default();
    config.exclude.methods = vec!["NodeContext.other_method".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    assert_eq!(
        result.unsupported_public_items.len(),
        1,
        "method NOT in exclude.methods must remain in unsupported_public_items"
    );
}

/// Non-method items (kind == "function") must be unaffected by `exclude.methods` —
/// they are only suppressed by `exclude.functions`.
#[test]
fn apply_filters_exclude_methods_does_not_affect_unsupported_function_items() {
    let mut surface = surface_with(vec![], vec![]);
    surface
        .unsupported_public_items
        .push(make_unsupported_function("generic_helper"));

    let mut config = ResolvedCrateConfig::default();
    config.exclude.methods = vec!["generic_helper".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    assert_eq!(
        result.unsupported_public_items.len(),
        1,
        "exclude.methods must not suppress items with item_kind == 'function'"
    );
}

#[test]
fn apply_filters_retains_unsupported_function_when_included_by_function_list() {
    let mut surface = surface_with(vec![], vec![]);
    surface
        .unsupported_public_items
        .push(make_unsupported_function("generic_helper"));
    surface
        .unsupported_public_items
        .push(make_unsupported_function("unused_generic"));

    let mut config = ResolvedCrateConfig::default();
    config.include.functions = vec!["generic_helper".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    assert_eq!(
        result
            .unsupported_public_items
            .iter()
            .map(|item| item.item_path.as_str())
            .collect::<Vec<_>>(),
        vec!["my_crate::generic_helper"],
        "include.functions must retain diagnostics only for included generic functions"
    );
}

#[test]
fn apply_filters_retains_unsupported_method_when_parent_type_is_included() {
    let mut surface = surface_with(vec![make_typedef("NodeContext"), make_typedef("OtherType")], vec![]);
    surface
        .unsupported_public_items
        .push(make_unsupported_method("NodeContext", "serialize"));
    surface
        .unsupported_public_items
        .push(make_unsupported_method("OtherType", "serialize"));

    let mut config = ResolvedCrateConfig::default();
    config.include.types = vec!["NodeContext".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    assert_eq!(
        result
            .unsupported_public_items
            .iter()
            .map(|item| item.item_path.as_str())
            .collect::<Vec<_>>(),
        vec!["my_crate::module::NodeContext.serialize"],
        "include.types must retain diagnostics only for methods owned by included public types"
    );
}

/// A method declared in `[[crates.services]].configurators` must remain in
/// `service.configurators` even when the same `OwnerType.method_name` key also
/// appears in `[crates.exclude].methods`.
///
/// Background: the exclude list is intended to suppress the *generic struct-level*
/// method emission (preventing non-delegatable stubs in binding codegen). It must
/// not remove the method from the *service IR*, where its presence drives dedicated
/// C/host-language configurator entrypoints. Both intents are independent.
///
/// The fixture uses a purely synthetic owner type so no consumer-library names
/// appear in the test.
#[test]
fn configurator_survives_exclude_methods_post_service_pass() {
    use crate::core::config::service::{EntrypointSpec, ServiceConfig};
    use crate::core::ir::{MethodDef, ReceiverKind, ServiceDef, TypeRef};

    let configurator_method = MethodDef {
        name: "setup".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Foo".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let constructor_method = MethodDef {
        name: "new".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Foo".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let service = ServiceDef {
        name: "Foo".to_string(),
        rust_path: "test_crate::Foo".to_string(),
        constructor: constructor_method,
        configurators: vec![configurator_method],
        registrations: vec![],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let mut config = ResolvedCrateConfig {
        name: "test_crate".to_string(),
        services: vec![ServiceConfig {
            owner_type: "Foo".to_string(),
            constructor: Some("new".to_string()),
            configurators: vec!["setup".to_string()],
            registrations: vec![],
            entrypoints: vec![EntrypointSpec {
                method: "run".to_string(),
                kind: "run".to_string(),
            }],
            skip_languages: vec![],
            host_app_inner_accessor: None,
        }],
        ..Default::default()
    };
    config.exclude.methods = vec!["Foo.setup".to_string()];

    let mut api = ApiSurface {
        crate_name: "test_crate".to_string(),
        services: vec![service],
        ..ApiSurface::default()
    };

    if !config.exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !config.exclude.methods.contains(&key)
            });
        }
    }

    assert_eq!(api.services.len(), 1, "service must be present after the exclude pass");
    assert_eq!(
        api.services[0].configurators.len(),
        1,
        "configurator `setup` must survive the exclude-methods post-service pass; got {:?}",
        api.services[0]
            .configurators
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        api.services[0].configurators[0].name, "setup",
        "configurator name must be `setup`"
    );
}

/// Regression: the function-dedup pass in `dedup_api_surface` must key on
/// `(name, cfg)`, not `name` alone. The pub-use-clears-skip extractor pass
/// synthesises a paired entry under a disjoint cfg for `#[cfg(X)] pub use mod::fn`
/// patterns whose source is generic/skipped (e.g. `embed_texts_async`: a real
/// `#[cfg(feature="embeddings")]` clone plus a `#[cfg(not(feature="embeddings"))]`
/// stub). Both must survive dedup because exactly one compiles under any feature
/// combination; collapsing by name alone dropped one and made the symbol vanish
/// from every binding whenever the surviving entry's cfg was inactive.
#[test]
fn dedup_keeps_same_named_functions_with_disjoint_cfgs() {
    let mut real = make_funcdef(
        "embed_texts_async",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        vec![],
    );
    real.cfg = Some("all (feature = \"embeddings\" , feature = \"tokio-runtime\")".to_string());
    let mut stub = make_funcdef(
        "embed_texts_async",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        vec![],
    );
    stub.cfg = Some(
        "all (feature = \"embedding-presets\" , not (feature = \"embeddings\") , feature = \"tokio-runtime\")"
            .to_string(),
    );

    let mut surface = surface_with(vec![], vec![real, stub]);
    super::type_helpers::dedup_api_surface(&mut surface);

    let entries: Vec<_> = surface
        .functions
        .iter()
        .filter(|f| f.name == "embed_texts_async")
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "both cfg-gated alternatives must survive dedup; got {entries:?}"
    );
    let cfgs: Vec<&str> = entries.iter().filter_map(|f| f.cfg.as_deref()).collect();
    assert!(cfgs.iter().any(|c| c.contains("\"embeddings\"") && !c.contains("not")));
    assert!(cfgs.iter().any(|c| c.contains("not") && c.contains("\"embeddings\"")));
}

/// Regression: when a name resolves to a real re-export and a crate-root stub under
/// disjoint cfgs (e.g. `LlmBackend`: real `pub use` gated `all(feature="ner-llm",
/// not(android-x86_64))` plus a crate-root stub gated `any(not(feature="ner-llm"),
/// android-x86_64)`), the surviving type entry must carry the OR-merge of both cfgs.
/// Otherwise the emitted dart wrapper struct is gated out on android-x86_64 while
/// flutter_rust_bridge's `frb_generated.rs` references `crate::LlmBackend`
/// unconditionally → `cannot find type LlmBackend in crate`.
#[test]
fn dedup_or_merges_cfgs_for_same_named_type_with_disjoint_cfgs() {
    let mut real = make_typedef("LlmBackend");
    real.is_opaque = true;
    real.cfg = Some(
        "all (feature = \"ner-llm\" , not (target_arch = \"wasm32\") , not (all (target_os = \"android\" , target_arch = \"x86_64\")))"
            .to_string(),
    );
    let mut stub = make_typedef("LlmBackend");
    stub.is_opaque = true;
    stub.cfg = Some(
        "any (not (feature = \"ner-llm\") , all (target_os = \"android\" , target_arch = \"x86_64\"))".to_string(),
    );

    let mut surface = surface_with(vec![real, stub], vec![]);
    super::type_helpers::dedup_api_surface(&mut surface);

    let entries: Vec<_> = surface.types.iter().filter(|t| t.name == "LlmBackend").collect();
    assert_eq!(entries.len(), 1, "same-named type collision must collapse to one entry");
    let cfg = entries[0].cfg.as_deref().expect("merged cfg must be present");
    assert!(
        cfg.starts_with("any("),
        "disjoint cfgs must be OR-merged into an any(...) gate; got `{cfg}`"
    );
    assert!(
        cfg.contains("not (all (target_os = \"android\""),
        "the real re-export's cfg must remain in the merge; got `{cfg}`"
    );
    assert!(
        cfg.contains("all (target_os = \"android\" , target_arch = \"x86_64\")"),
        "the android-x86_64 stub cfg must be included so the wrapper survives there; got `{cfg}`"
    );
}

/// A same-named type collision where every member carries the SAME (single) cfg is a
/// genuine duplicate (e.g. one feature, two module paths) — dedup must collapse to the
/// shortest rust_path without wrapping the cfg in a spurious `any(...)`.
#[test]
fn dedup_collapses_same_named_type_with_identical_cfg() {
    let mut near = make_typedef("Table");
    near.rust_path = "my_crate::Table".to_string();
    near.cfg = Some("feature = \"office\"".to_string());
    let mut far = make_typedef("Table");
    far.rust_path = "my_crate::extraction::docx::Table".to_string();
    far.cfg = Some("feature = \"office\"".to_string());

    let mut surface = surface_with(vec![far, near], vec![]);
    super::type_helpers::dedup_api_surface(&mut surface);

    let entries: Vec<_> = surface.types.iter().filter(|t| t.name == "Table").collect();
    assert_eq!(entries.len(), 1, "identical-cfg duplicates must collapse to one");
    assert_eq!(entries[0].rust_path, "my_crate::Table", "shortest rust_path wins");
    assert_eq!(
        entries[0].cfg.as_deref(),
        Some("feature = \"office\""),
        "identical cfg must pass through unchanged, not be wrapped in any(...)"
    );
}

/// Same-named functions sharing an identical cfg (or both `None`) at different
/// rust_paths are genuine duplicates — dedup must still collapse them to the
/// entry with the shortest rust_path (closest to crate root).
#[test]
fn dedup_collapses_same_named_functions_with_identical_cfg() {
    let mut near = make_funcdef(
        "clean_text",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        vec![],
    );
    near.rust_path = "my_crate::clean_text".to_string();
    let mut far = make_funcdef(
        "clean_text",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        vec![],
    );
    far.rust_path = "my_crate::text::quality::clean_text".to_string();

    let mut surface = surface_with(vec![], vec![far, near]);
    super::type_helpers::dedup_api_surface(&mut surface);

    let entries: Vec<_> = surface.functions.iter().filter(|f| f.name == "clean_text").collect();
    assert_eq!(entries.len(), 1, "identical-cfg duplicates must collapse to one");
    assert_eq!(entries[0].rust_path, "my_crate::clean_text", "shortest rust_path wins");
}

/// A field listed in `[crates.exclude].fields` as `"TypeName.field_name"` must be marked
/// `binding_excluded` on the matching struct field — the same central IR flag that
/// `#[cfg_attr(alef, alef(skip))]` sets — so it disappears from every binding uniformly
/// without any backend-specific handling. Sibling fields on the same type must be
/// unaffected.
#[test]
fn apply_filters_marks_field_binding_excluded_when_listed_in_exclude_fields() {
    let mut foo = make_typedef("Foo");
    foo.fields = vec![
        crate::core::ir::FieldDef {
            name: "bar".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            ..crate::core::ir::FieldDef::default()
        },
        crate::core::ir::FieldDef {
            name: "baz".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            ..crate::core::ir::FieldDef::default()
        },
    ];
    let surface = surface_with(vec![foo], vec![]);

    let mut config = ResolvedCrateConfig::default();
    config.exclude.fields = vec!["Foo.bar".to_string()];

    let result = apply_filters(surface, &config).expect("filters must resolve every configured entry");

    let typ = result
        .types
        .iter()
        .find(|t| t.name == "Foo")
        .expect("Foo must survive filtering");
    let bar = typ
        .fields
        .iter()
        .find(|f| f.name == "bar")
        .expect("bar field must survive filtering");
    assert!(
        bar.binding_excluded,
        "Foo.bar listed in exclude.fields must be binding_excluded"
    );
    assert!(
        bar.binding_exclusion_reason.is_some(),
        "binding_excluded field must carry a diagnostic reason"
    );

    let baz = typ
        .fields
        .iter()
        .find(|f| f.name == "baz")
        .expect("baz field must survive filtering");
    assert!(
        !baz.binding_excluded,
        "Foo.baz not listed in exclude.fields must remain included"
    );
}

/// A field whose named type is not part of the binding surface is sanitized to `String`, which
/// erases the only record of what the Rust source actually declared. `type_rust_path` does not
/// cover it: `extract_field_type_rust_path` returns `None` for a single-segment path, which is
/// exactly what an imported type looks like at the use site. `original_type` is the documented
/// carrier for the pre-sanitization type, so it must be populated here too -- otherwise the Rust
/// reference page has nothing left to render but the placeholder. ~keep
#[test]
fn sanitize_records_the_pre_sanitization_type_of_an_unknown_named_field() {
    let mut api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "TextOptions".to_string(),
            rust_path: "sample::TextOptions".to_string(),
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "pool".to_string(),
                    ty: TypeRef::Named("PoolSettings".to_string()),
                    optional: true,
                    ..crate::core::ir::FieldDef::default()
                },
                crate::core::ir::FieldDef {
                    name: "stages".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("StageSpec".to_string()))),
                    ..crate::core::ir::FieldDef::default()
                },
            ],
            ..crate::core::ir::TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    sanitize_unknown_types(&mut api);

    let pool = &api.types[0].fields[0];
    assert!(pool.sanitized);
    assert_eq!(pool.original_type.as_deref(), Some("PoolSettings"));
    let stages = &api.types[0].fields[1];
    assert!(stages.sanitized);
    assert_eq!(stages.original_type.as_deref(), Some("Vec<StageSpec>"));
}
