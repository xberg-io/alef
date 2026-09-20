use super::*;

#[test]
fn bridge_handle_path_uses_alias_typedef_rust_path() {
    let mut api = ApiSurface::default();
    api.types.push(make_type_def(
        "RendererHandle",
        "mylib::callbacks::RendererHandle",
        vec![],
    ));
    let bridge = make_bridge(
        Some("RendererHandle"),
        Some("renderer"),
        BridgeBinding::FunctionParam,
        None,
        None,
        None,
        None,
    );

    assert_eq!(
        bridge_handle_path(&api, &bridge, "mylib"),
        "mylib::callbacks::RendererHandle"
    );
}

#[test]
fn bridge_handle_path_uses_excluded_alias_path() {
    let mut api = ApiSurface::default();
    api.excluded_type_paths.insert(
        "RendererHandle".to_string(),
        "mylib::callbacks::RendererHandle".to_string(),
    );
    let bridge = make_bridge(
        Some("RendererHandle"),
        Some("renderer"),
        BridgeBinding::FunctionParam,
        None,
        None,
        None,
        None,
    );

    assert_eq!(
        bridge_handle_path(&api, &bridge, "mylib"),
        "mylib::callbacks::RendererHandle"
    );
}

#[test]
fn managed_function_lookup_matches_trait_bridge_lifecycle_names() {
    let bridge = TraitBridgeConfig {
        register_fn: Some("register_renderer".to_string()),
        unregister_fn: Some("unregister_renderer".to_string()),
        clear_fn: Some("clear_renderers".to_string()),
        ..TraitBridgeConfig::default()
    };
    let bridges = vec![bridge];

    assert!(is_trait_bridge_managed_fn("register_renderer", &bridges));
    assert!(is_trait_bridge_managed_fn("unregister_renderer", &bridges));
    assert!(is_trait_bridge_managed_fn("clear_renderers", &bridges));
    assert!(!is_trait_bridge_managed_fn("list_renderers", &bridges));
}

fn make_bridge(
    type_alias: Option<&str>,
    param_name: Option<&str>,
    bind_via: BridgeBinding,
    options_type: Option<&str>,
    options_field: Option<&str>,
    context_type: Option<&str>,
    result_type: Option<&str>,
) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: type_alias.map(str::to_string),
        param_name: param_name.map(str::to_string),
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via,
        options_type: options_type.map(str::to_string),
        options_field: options_field.map(str::to_string),
        context_type: context_type.map(str::to_string),
        result_type: result_type.map(str::to_string),
    }
}

#[test]
fn find_bridge_param_returns_first_param_match_in_function_param_mode() {
    let func = make_func(
        "convert",
        vec![
            make_param("html", TypeRef::String, true),
            make_param("visitor", TypeRef::Named("VisitorHandle".to_string()), false),
        ],
    );
    let bridges = vec![make_bridge(
        Some("VisitorHandle"),
        Some("visitor"),
        BridgeBinding::FunctionParam,
        None,
        None,
        None,
        None,
    )];
    let result = find_bridge_param(&func, &bridges).expect("bridge match");
    assert_eq!(result.0, 1);
}

#[test]
fn find_bridge_param_skips_options_field_bridges() {
    let func = make_func(
        "convert",
        vec![
            make_param("html", TypeRef::String, true),
            make_param("visitor", TypeRef::Named("VisitorHandle".to_string()), false),
        ],
    );
    let bridges = vec![make_bridge(
        Some("VisitorHandle"),
        Some("visitor"),
        BridgeBinding::OptionsField,
        Some("ConversionOptions"),
        Some("visitor"),
        None,
        None,
    )];
    assert!(
        find_bridge_param(&func, &bridges).is_none(),
        "bridges configured with bind_via=options_field must not be returned by find_bridge_param"
    );
}

#[test]
fn find_bridge_field_detects_field_via_alias() {
    let opts_type = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "mylib::ConversionOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("debug", TypeRef::Primitive(PrimitiveType::Bool)),
            make_field(
                "visitor",
                TypeRef::Optional(Box::new(TypeRef::Named("VisitorHandle".to_string()))),
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
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
    };
    let func = make_func(
        "convert",
        vec![
            make_param("html", TypeRef::String, true),
            make_param(
                "options",
                TypeRef::Optional(Box::new(TypeRef::Named("ConversionOptions".to_string()))),
                false,
            ),
        ],
    );
    let bridges = vec![make_bridge(
        Some("VisitorHandle"),
        Some("visitor"),
        BridgeBinding::OptionsField,
        Some("ConversionOptions"),
        None,
        None,
        None,
    )];
    let m = find_bridge_field(&func, std::slice::from_ref(&opts_type), &bridges).expect("bridge field match");
    assert_eq!(m.param_index, 1);
    assert_eq!(m.param_name, "options");
    assert_eq!(m.options_type, "ConversionOptions");
    assert!(m.param_is_optional);
    assert_eq!(m.field_name, "visitor");
}

#[test]
fn bridge_type_wins_over_shared_parameter_name_in_either_order() {
    for optional in [false, true] {
        for reverse in [false, true] {
            let named = TypeRef::Named("WriterHandle".into());
            let ty = if optional {
                TypeRef::Optional(Box::new(named))
            } else {
                named
            };
            let func = make_func("use_writer", vec![make_param("host", ty, optional)]);
            let mut bridges = vec![
                lookup_bridge(Some("ReaderHandle"), Some("host")),
                lookup_bridge(Some("WriterHandle"), Some("host")),
            ];
            if reverse {
                bridges.reverse();
            }
            let (index, bridge) = find_bridge_param(&func, &bridges).expect("typed bridge");
            assert_eq!(index, 0);
            assert_eq!(bridge.type_alias.as_deref(), Some("WriterHandle"));
        }
    }
}

#[test]
fn bridge_lookup_retains_name_fallback_and_first_parameter_precedence() {
    let bridges = vec![
        lookup_bridge(None, Some("host")),
        lookup_bridge(Some("WriterHandle"), None),
    ];
    let func = make_func(
        "use_hosts",
        vec![
            make_param("host", TypeRef::Named("OtherHandle".into()), false),
            make_param("writer", TypeRef::Named("WriterHandle".into()), false),
        ],
    );
    let (index, bridge) = find_bridge_param(&func, &bridges).expect("name fallback");
    assert_eq!(index, 0);
    assert!(std::ptr::eq(bridge, &bridges[0]));
    let unmatched = make_func("other", vec![make_param("other", TypeRef::String, false)]);
    assert!(find_bridge_param(&unmatched, &bridges).is_none());
}

#[test]
fn bridge_lookup_ignores_options_field_type_matches() {
    let func = make_func(
        "use_writer",
        vec![make_param("host", TypeRef::Named("WriterHandle".into()), false)],
    );
    let mut options = lookup_bridge(Some("WriterHandle"), Some("host"));
    options.bind_via = BridgeBinding::OptionsField;
    let bridges = vec![options, lookup_bridge(None, Some("host"))];
    let (_, bridge) = find_bridge_param(&func, &bridges).expect("function bridge");
    assert!(std::ptr::eq(bridge, &bridges[1]));
}

fn lookup_bridge(type_alias: Option<&str>, param_name: Option<&str>) -> TraitBridgeConfig {
    make_bridge(
        type_alias,
        param_name,
        BridgeBinding::FunctionParam,
        None,
        None,
        None,
        None,
    )
}

#[test]
fn find_bridge_field_returns_none_for_function_param_bridge() {
    let opts_type = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "mylib::ConversionOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field(
            "visitor",
            TypeRef::Optional(Box::new(TypeRef::Named("VisitorHandle".to_string()))),
        )],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
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
    };
    let func = make_func(
        "convert",
        vec![make_param(
            "options",
            TypeRef::Named("ConversionOptions".to_string()),
            false,
        )],
    );
    let bridges = vec![make_bridge(
        Some("VisitorHandle"),
        Some("visitor"),
        BridgeBinding::FunctionParam,
        None,
        None,
        None,
        None,
    )];
    assert!(find_bridge_field(&func, std::slice::from_ref(&opts_type), &bridges).is_none());
}

/// Plain data-struct TypeDef (not a trait, not opaque) with configurable serde / exclusion.
fn make_struct_def(name: &str, has_serde: bool, is_opaque: bool, binding_excluded: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        is_trait: false,
        is_opaque,
        has_serde,
        binding_excluded,
        ..TypeDef::default()
    }
}

#[test]
fn opaque_params_require_clone_read_only_exported_handle() {
    let handle = TypeDef {
        name: "Control".into(),
        is_opaque: true,
        is_clone: true,
        ..Default::default()
    };
    let api = ApiSurface {
        types: vec![
            handle.clone(),
            TypeDef {
                name: "NoClone".into(),
                is_clone: false,
                ..handle.clone()
            },
            TypeDef {
                name: "Hidden".into(),
                binding_excluded: true,
                ..handle.clone()
            },
            TypeDef {
                name: "Trait".into(),
                is_trait: true,
                ..handle.clone()
            },
            TypeDef {
                name: "Lifetime".into(),
                has_lifetime_params: true,
                ..handle.clone()
            },
            TypeDef {
                name: "Value".into(),
                is_opaque: false,
                ..handle.clone()
            },
            TypeDef {
                name: "Mutable".into(),
                methods: vec![MethodDef {
                    receiver: Some(ReceiverKind::RefMut),
                    ..Default::default()
                }],
                ..handle
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        native_marshalled_opaque_params(&api),
        std::collections::HashSet::from(["Control".into()])
    );
}

#[test]
fn native_marshalled_struct_params_allowlists_only_known_serde_structs() {
    let greet = MethodDef {
        name: "greet".to_string(),
        params: vec![
            make_param("opts", TypeRef::Named("Opts".to_string()), true),
            make_param("mood", TypeRef::Named("Mood".to_string()), true),
            make_param("handle", TypeRef::Named("Handle".to_string()), true),
            make_param("hidden", TypeRef::Named("Hidden".to_string()), true),
            make_param("plain", TypeRef::Named("Plain".to_string()), true),
            make_param("name", TypeRef::String, true),
            make_param("unknown", TypeRef::Named("Unknown".to_string()), true),
        ],
        return_type: TypeRef::Named("Doc".to_string()),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..MethodDef::default()
    };
    let trait_def = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "mylib::Greeter".to_string(),
        is_trait: true,
        methods: vec![greet],
        ..TypeDef::default()
    };

    let api = ApiSurface {
        types: vec![
            make_struct_def("Opts", true, false, false),
            make_struct_def("Handle", true, true, false),
            make_struct_def("Hidden", true, false, true),
            make_struct_def("Plain", false, false, false),
            make_struct_def("Doc", true, false, false),
        ],
        enums: vec![EnumDef {
            name: "Mood".to_string(),
            ..EnumDef::default()
        }],
        ..ApiSurface::default()
    };

    let structs = native_marshalled_struct_params(&trait_def, &api);
    assert!(
        structs.contains("Opts"),
        "known serde struct must be allowlisted: {structs:?}"
    );
    assert_eq!(structs.len(), 1, "only `Opts` qualifies, got {structs:?}");
    for excluded in ["Mood", "Handle", "Hidden", "Plain", "Unknown", "Doc"] {
        assert!(
            !structs.contains(excluded),
            "{excluded} must NOT be native-marshalled: {structs:?}"
        );
    }

    assert!(is_native_marshalled_struct("Opts", &api));
    assert!(!is_native_marshalled_struct("Handle", &api));
    assert!(!is_native_marshalled_struct("Hidden", &api));
    assert!(!is_native_marshalled_struct("Plain", &api));
    assert!(!is_native_marshalled_struct("Unknown", &api));
}

#[test]
fn find_trait_def_locates_bridge_trait_by_name() {
    let trait_def = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "mylib::Greeter".to_string(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "process".to_string(),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let api = ApiSurface {
        types: vec![
            TypeDef {
                name: "Greeter".to_string(),
                is_trait: false,
                ..TypeDef::default()
            },
            trait_def,
            TypeDef {
                name: "Other".to_string(),
                is_trait: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };
    let bridge = TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..TraitBridgeConfig::default()
    };

    let found = find_trait_def(&bridge, &api).expect("trait def must be found");
    assert!(
        found.is_trait,
        "must resolve the trait, not the struct of the same name"
    );
    assert_eq!(found.methods.len(), 1);
    assert_eq!(found.methods[0].name, "process");

    let missing = TraitBridgeConfig {
        trait_name: "Absent".to_string(),
        ..TraitBridgeConfig::default()
    };
    assert!(find_trait_def(&missing, &api).is_none());
}

fn plugin_trait_api() -> ApiSurface {
    ApiSurface {
        types: vec![TypeDef {
            name: "SamplePlugin".to_string(),
            rust_path: "sample_core::SamplePlugin".to_string(),
            is_trait: true,
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn plugin_bridge_cfg() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "SamplePlugin".to_string(),
        registry_getter: Some("sample_core::registry::get_sample_plugin_registry".to_string()),
        register_fn: Some("install_sample_plugin".to_string()),
        ..TraitBridgeConfig::default()
    }
}

#[test]
fn bridge_targets_language_is_true_when_no_spelling_is_excluded() {
    assert!(bridge_targets_language(&plugin_bridge_cfg(), &["node", "napi"]));
}

#[test]
fn bridge_targets_language_is_false_when_any_spelling_is_excluded() {
    for excluded in ["node", "napi"] {
        let bridge = TraitBridgeConfig {
            exclude_languages: vec![excluded.to_string()],
            ..plugin_bridge_cfg()
        };

        assert!(
            !bridge_targets_language(&bridge, &["node", "napi"]),
            "`exclude_languages = [\"{excluded}\"]` must suppress the bridge under both spellings"
        );
    }
}

#[test]
fn bridge_register_symbol_is_none_without_a_registry_getter() {
    let bridge = TraitBridgeConfig {
        registry_getter: None,
        ..plugin_bridge_cfg()
    };

    assert_eq!(bridge_register_symbol(&bridge), None);
}

#[test]
fn bridge_register_symbol_is_the_configured_name_with_a_registry_getter() {
    assert_eq!(
        bridge_register_symbol(&plugin_bridge_cfg()),
        Some("install_sample_plugin")
    );
}

#[test]
fn active_bridge_trait_def_resolves_the_trait_for_an_unexcluded_target() {
    let api = plugin_trait_api();

    let found = active_bridge_trait_def(&plugin_bridge_cfg(), &api, &["node", "napi"])
        .expect("an unexcluded bridge whose trait is present must resolve");

    assert_eq!(found.name, "SamplePlugin");
}

#[test]
fn active_bridge_trait_def_is_none_when_the_target_is_excluded_under_either_spelling() {
    let api = plugin_trait_api();

    for excluded in ["node", "napi"] {
        let bridge = TraitBridgeConfig {
            exclude_languages: vec![excluded.to_string()],
            ..plugin_bridge_cfg()
        };

        assert_eq!(
            active_bridge_trait_def(&bridge, &api, &["node", "napi"]).map(|typ| typ.name.as_str()),
            None,
            "`exclude_languages = [\"{excluded}\"]` must gate the bridge out"
        );
    }
}

#[test]
fn active_bridge_trait_def_is_none_when_the_trait_is_absent_from_the_api_surface() {
    let bridge = TraitBridgeConfig {
        trait_name: "AbsentPlugin".to_string(),
        ..plugin_bridge_cfg()
    };

    assert_eq!(
        active_bridge_trait_def(&bridge, &plugin_trait_api(), &["node", "napi"]).map(|typ| typ.name.as_str()),
        None
    );
}
