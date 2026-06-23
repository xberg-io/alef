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

// ---------------------------------------------------------------------------
// find_bridge_param / find_bridge_field
// ---------------------------------------------------------------------------

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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
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

// ---------------------------------------------------------------------------
// Shared native-object marshalling classification
// ---------------------------------------------------------------------------

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
fn native_marshalled_struct_params_allowlists_only_known_serde_structs() {
    // A neutral `Greeter` trait whose method takes one of each interesting param kind.
    let greet = MethodDef {
        name: "greet".to_string(),
        params: vec![
            make_param("opts", TypeRef::Named("Opts".to_string()), true), // struct, serde
            make_param("mood", TypeRef::Named("Mood".to_string()), true), // enum
            make_param("handle", TypeRef::Named("Handle".to_string()), true), // opaque
            make_param("hidden", TypeRef::Named("Hidden".to_string()), true), // excluded
            make_param("plain", TypeRef::Named("Plain".to_string()), true), // struct, NO serde
            make_param("name", TypeRef::String, true),                    // primitive-ish
            make_param("unknown", TypeRef::Named("Unknown".to_string()), true), // not in api.types
        ],
        return_type: TypeRef::Named("Doc".to_string()),
        receiver: Some(ReceiverKind::Ref),
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
            make_struct_def("Opts", true, false, false),   // qualifies
            make_struct_def("Handle", true, true, false),  // opaque → no
            make_struct_def("Hidden", true, false, true),  // excluded → no
            make_struct_def("Plain", false, false, false), // no serde → no
            make_struct_def("Doc", true, false, false),    // not a param, ignored
        ],
        // `Mood` lives in `enums`, never `types`, so it can never be a struct.
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

    // Direct classifier checks.
    assert!(is_native_marshalled_struct("Opts", &api));
    assert!(!is_native_marshalled_struct("Handle", &api));
    assert!(!is_native_marshalled_struct("Hidden", &api));
    assert!(!is_native_marshalled_struct("Plain", &api));
    assert!(!is_native_marshalled_struct("Unknown", &api));
}
