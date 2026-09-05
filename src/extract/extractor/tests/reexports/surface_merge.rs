use super::*;

#[test]
fn test_merge_surface_and_combines_own_cfg_with_module_cfg() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        types: vec![TypeDef {
            name: "Gated".into(),
            rust_path: "src::Gated".into(),
            cfg: Some(r#"feature = "own""#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, Some(r#"feature = "module""#.into()));

    let combined = dst.types[0]
        .cfg
        .as_deref()
        .expect("own cfg must survive the merge, combined");
    assert_eq!(combined, r#"all(feature = "module", feature = "own")"#);

    // Assert on the evaluation result, not just the string shape: feed the combined cfg to the
    // canonical evaluator so a normalisation regression that silently breaks the leaf match
    // would fail this test even if the string still "looked" combined. ~keep
    let satisfied = |enabled: &[&str]| {
        let set: ::std::collections::HashSet<&str> = enabled.iter().copied().collect();
        crate::core::ir::cfg_feature_satisfied(Some(combined), &set)
    };
    assert!(!satisfied(&[]), "neither gate enabled must not satisfy");
    assert!(!satisfied(&["module"]), "own cfg missing must not satisfy");
    assert!(!satisfied(&["own"]), "module cfg missing must not satisfy");
    assert!(satisfied(&["module", "own"]), "both gates enabled must satisfy");
}

/// Regression: a sibling crate commonly re-exports an item under the exact feature the item is
/// itself gated on (`#[cfg(feature = "x")] pub use other::thing;` where `thing` is declared
/// `#[cfg(feature = "x")]`). AND-combining unconditionally would grow that into `all(x, x)` —
/// same evaluation result, but a gate string that churns on every regen and reads as a generator
/// bug in the diff. The item's own gate already implies the reexport's, so it must pass through
/// unchanged, exactly as `combine_gates` already does for a method inheriting its impl block's
/// gate. ~keep
#[test]
fn test_merge_surface_collapses_reexport_cfg_identical_to_own_cfg() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        functions: vec![crate::core::ir::FunctionDef {
            name: "gated_fn".into(),
            rust_path: "src::gated_fn".into(),
            cfg: Some(r#"feature = "x""#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, Some(r#"feature = "x""#.into()));

    assert_eq!(
        dst.functions[0].cfg.as_deref(),
        Some(r#"feature = "x""#),
        "an identical reexport gate must not double-wrap the item's own gate"
    );
}

/// Same collapse, one level deeper: the item's own gate is already a conjunction that names the
/// reexport's feature (`all(x, y)` re-exported under `#[cfg(feature = "x")]`). The reexport gate
/// adds no information `all(x, y)` doesn't already require, so the combined gate must stay
/// `all(x, y)` rather than growing to the doubly redundant `all(x, all(x, y))`.
#[test]
fn test_merge_surface_collapses_reexport_cfg_already_implied_by_own_conjunction() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        functions: vec![crate::core::ir::FunctionDef {
            name: "gated_fn".into(),
            rust_path: "src::gated_fn".into(),
            cfg: Some(r#"all(feature = "x", feature = "y")"#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, Some(r#"feature = "x""#.into()));

    assert_eq!(
        dst.functions[0].cfg.as_deref(),
        Some(r#"all(feature = "x", feature = "y")"#),
        "a reexport gate already implied by the item's own conjunction must not be re-wrapped"
    );
}

#[test]
fn test_merge_surface_fills_module_cfg_when_type_has_no_own_cfg() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        types: vec![TypeDef {
            name: "Ungated".into(),
            rust_path: "src::Ungated".into(),
            cfg: None,
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, Some(r#"feature = "module""#.into()));

    assert_eq!(dst.types[0].cfg.as_deref(), Some(r#"feature = "module""#));
}

#[test]
fn test_merge_surface_leaves_own_cfg_unchanged_through_ungated_module() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        types: vec![TypeDef {
            name: "SelfGated".into(),
            rust_path: "src::SelfGated".into(),
            cfg: Some(r#"feature = "own""#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, None);

    assert_eq!(dst.types[0].cfg.as_deref(), Some(r#"feature = "own""#));
}

#[test]
fn test_merge_surface_filtered_and_combines_cfg_for_functions_and_enums() {
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        functions: vec![crate::core::ir::FunctionDef {
            name: "wanted_fn".into(),
            rust_path: "src::wanted_fn".into(),
            cfg: Some(r#"feature = "own""#.into()),
            ..Default::default()
        }],
        enums: vec![crate::core::ir::EnumDef {
            name: "WantedEnum".into(),
            rust_path: "src::WantedEnum".into(),
            cfg: Some(r#"feature = "own""#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let names = vec!["wanted_fn".to_string(), "WantedEnum".to_string()];
    merge_surface_filtered(&mut dst, src, &names, Some(r#"feature = "module""#.into()));

    assert_eq!(
        dst.functions[0].cfg.as_deref(),
        Some(r#"all(feature = "module", feature = "own")"#)
    );
    assert_eq!(
        dst.enums[0].cfg.as_deref(),
        Some(r#"all(feature = "module", feature = "own")"#)
    );
}

#[test]
fn test_merge_surface_extendr_registration_fails_closed_for_inherited_module_cfg() {
    // `extendr::gen_bindings::cfg_registration::always_registered` is `pub(super)`, scoped to
    // `crate::backends::extendr::gen_bindings`, so it cannot be called from this file — this
    // lane owns only `reexports.rs` and its tests, and backends are explicitly out of scope for
    // task #53. Read directly instead (not edited): after whitespace-stripping, it recognises
    // only the self-cancelling shape `any(X, not(X))` and returns `false` for everything else,
    // including any `all(...)` shape. `combine_cfg` in `reexports.rs` only ever produces
    // `all(...)` when AND-combining an inherited module gate, never `any(X, not(X))`, so a type
    // that gains an inherited module gate here always becomes ineligible for
    // `always_registered` and is omitted from the extendr `extendr_module!` block — a missing R
    // function, never a broken symbol. This pins that shape so the fail-closed consequence is a
    // documented, chosen outcome rather than a silent side effect. ~keep
    let mut dst = ApiSurface::default();
    let src = ApiSurface {
        types: vec![TypeDef {
            name: "RGated".into(),
            rust_path: "src::RGated".into(),
            cfg: Some(r#"feature = "own""#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    merge_surface(&mut dst, src, Some(r#"feature = "module""#.into()));

    let combined = dst.types[0].cfg.as_deref().expect("cfg should be combined");
    assert!(
        combined.starts_with("all("),
        "combined cfg must be an `all(...)` gate: {combined}"
    );
    assert!(
        !combined.starts_with("any("),
        "combined cfg must never take the shape `always_registered` treats as always-compiled: {combined}"
    );
}

#[test]
fn test_merge_surface_no_duplicates() {
    let mut dst = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Existing".into(),
            rust_path: "test::Existing".into(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let src = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Existing".into(),
                rust_path: "test::Existing".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
                name: "NewType".into(),
                rust_path: "test::NewType".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    merge_surface(&mut dst, src, None);
    assert_eq!(dst.types.len(), 2);
    assert_eq!(dst.types[0].name, "Existing");
    assert_eq!(dst.types[1].name, "NewType");
}

#[test]
fn test_merge_surface_filtered() {
    let mut dst = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let src = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Wanted".into(),
                rust_path: "test::Wanted".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
                name: "NotWanted".into(),
                rust_path: "test::NotWanted".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    merge_surface_filtered(&mut dst, src, &["Wanted".to_string()], None);
    assert_eq!(dst.types.len(), 1);
    assert_eq!(dst.types[0].name, "Wanted");
}

#[test]
fn test_merge_surface_includes_functions_and_enums() {
    let mut dst = ApiSurface {
        crate_name: "dst".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let src = ApiSurface {
        crate_name: "src".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![crate::core::ir::FunctionDef {
            name: "my_fn".into(),
            rust_path: "src::my_fn".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![crate::core::ir::EnumDef {
            name: "MyEnum".into(),
            rust_path: "src::MyEnum".into(),
            original_rust_path: String::new(),
            variants: vec![],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_content: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    super::reexports::merge_surface(&mut dst, src, None);
    assert_eq!(dst.functions.len(), 1);
    assert_eq!(dst.functions[0].name, "my_fn");
    assert_eq!(dst.enums.len(), 1);
    assert_eq!(dst.enums[0].name, "MyEnum");
}

#[test]
fn test_merge_surface_filtered_includes_functions_and_enums() {
    let mut dst = ApiSurface {
        crate_name: "dst".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let src = ApiSurface {
        crate_name: "src".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![
            crate::core::ir::FunctionDef {
                name: "wanted_fn".into(),
                rust_path: "src::wanted_fn".into(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Unit,
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
            },
            crate::core::ir::FunctionDef {
                name: "unwanted_fn".into(),
                rust_path: "src::unwanted_fn".into(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Unit,
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
            },
        ],
        enums: vec![
            crate::core::ir::EnumDef {
                name: "WantedEnum".into(),
                rust_path: "src::WantedEnum".into(),
                original_rust_path: String::new(),
                variants: vec![],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                has_default: false,
                serde_content: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                rename_all_fields: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
            },
            crate::core::ir::EnumDef {
                name: "UnwantedEnum".into(),
                rust_path: "src::UnwantedEnum".into(),
                original_rust_path: String::new(),
                variants: vec![],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                has_default: false,
                serde_content: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                rename_all_fields: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
            },
        ],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let names = vec!["wanted_fn".to_string(), "WantedEnum".to_string()];
    super::reexports::merge_surface_filtered(&mut dst, src, &names, None);
    assert_eq!(dst.functions.len(), 1);
    assert_eq!(dst.functions[0].name, "wanted_fn");
    assert_eq!(dst.enums.len(), 1);
    assert_eq!(dst.enums[0].name, "WantedEnum");
}
