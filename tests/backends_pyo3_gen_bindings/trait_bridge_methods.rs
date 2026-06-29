use super::*;

// ---------------------------------------------------------------------------
// gen_sync_method_body
// ---------------------------------------------------------------------------

#[test]
fn test_gen_sync_method_body_unit_return_no_error() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("tick", vec![], TypeRef::Unit, false, false, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(body.contains("Python::attach"), "sync body should use Python::attach");
    assert!(
        body.contains("getattr(\"tick\")") && body.contains("call_method1(\"run\""),
        "should resolve the host method by name and invoke it via the caller's contextvars context"
    );
    assert!(
        body.contains("unwrap_or(())"),
        "unit return without error should use unwrap_or(())"
    );
}

#[test]
fn test_gen_sync_method_body_string_return_no_error() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("name", vec![], TypeRef::String, false, false, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("getattr(\"name\")"),
        "should resolve the host method by name"
    );
    assert!(
        body.contains("call_method1(\"run\""),
        "should invoke the host method via the caller's contextvars context"
    );
    assert!(body.contains("extract::<String>()"), "should extract String return");
    assert!(
        body.contains("unwrap_or_default()"),
        "infallible string return should use unwrap_or_default"
    );
}

#[test]
fn test_gen_sync_method_body_with_params_runs_via_contextvars() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def(
        "process",
        vec![make_param_def("input", TypeRef::String, false)],
        TypeRef::String,
        false,
        false,
        false,
    );
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("getattr(\"process\")"),
        "single-param method should resolve the host method by name"
    );
    assert!(
        body.contains("call_method1(\"run\", (bound_method, input"),
        "single-param method should forward args through the caller's contextvars context"
    );
}

#[test]
fn test_gen_sync_method_body_with_error_uses_map_err() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("run", vec![], TypeRef::Unit, false, true, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("map_err"),
        "fallible method should have map_err for error conversion"
    );
    assert!(
        body.contains("Error::from("),
        "error path should call the configured error_constructor"
    );
}

// ---------------------------------------------------------------------------
// gen_async_method_body
// ---------------------------------------------------------------------------

#[test]
fn test_gen_async_method_body_uses_spawn_blocking() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("fetch", vec![], TypeRef::String, true, true, false);
    let body = generator.gen_async_method_body(&method, &spec);

    assert!(
        body.contains("spawn_blocking"),
        "async method should use spawn_blocking for Python dispatch"
    );
    assert!(
        body.contains("Python::attach"),
        "async body should re-enter Python GIL inside spawn_blocking"
    );
    assert!(
        body.contains(".await"),
        "async body should await the spawn_blocking result"
    );
}

#[test]
fn test_gen_async_method_body_clones_ref_params() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def(
        "transform",
        vec![make_param_def("data", TypeRef::String, false)],
        TypeRef::String,
        true,
        true,
        false,
    );
    let body = generator.gen_async_method_body(&method, &spec);

    // owned params must be cloned before the blocking closure captures them
    assert!(
        body.contains("let data = data.clone()"),
        "owned params should be cloned before spawn_blocking capture"
    );
}

#[test]
fn test_gen_async_method_body_unit_return() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("shutdown", vec![], TypeRef::Unit, true, true, false);
    let body = generator.gen_async_method_body(&method, &spec);

    assert!(
        body.contains("map(|_| ())"),
        "async unit return should map result to ()"
    );
    assert!(
        body.contains("Error::from("),
        "async unit return error path should call the configured error_constructor"
    );
}

// ---------------------------------------------------------------------------
// Native-object marshalling of struct callback params
//
// A trait-callback param that is a known serde struct must be handed to the host as the
// binding's NATIVE Python object — constructed via the same `From<core::T>` conversion the
// binding uses for return values — NOT serialized to a JSON string. Enum / opaque / unknown
// params keep their prior representation. The positive allowlist is computed by the SHARED
// classifier (`native_marshalled_struct_params`) and carried on the generator's
// `struct_param_types`.
// ---------------------------------------------------------------------------

fn make_struct_aware_generator(core_import: &str, struct_params: &[&str]) -> Pyo3BridgeGenerator {
    Pyo3BridgeGenerator {
        core_import: core_import.to_string(),
        type_paths: HashMap::new(),
        error_type: "Error".to_string(),
        struct_param_types: struct_params.iter().map(|s| s.to_string()).collect(),
        struct_return_types: std::collections::HashSet::new(),
    }
}

fn struct_param_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
    TraitBridgeSpec {
        trait_def,
        bridge_config: bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    }
}

#[test]
fn test_sync_struct_param_passed_as_native_object_not_json_string() {
    // `Greeter::greet(&self, opts: &Opts) -> Doc` — `Opts` is a known serde struct.
    let generator = make_struct_aware_generator("my_lib", &["Opts"]);
    let trait_def = make_trait_def("Greeter", "my_lib::Greeter", vec![]);
    let bridge_cfg = make_bridge_cfg("Greeter");
    let spec = struct_param_spec(&trait_def, &bridge_cfg);

    let method = make_method_def(
        "greet",
        vec![make_param_def("opts", TypeRef::Named("Opts".to_string()), true)],
        TypeRef::Named("Doc".to_string()),
        false,
        true,
        false,
    );
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("Opts::from((*opts).clone())"),
        "struct param must be constructed as the binding's native Python object via From<core>:\n{body}"
    );
    assert!(
        !body.contains("serde_json::to_string(opts)"),
        "struct param must NOT be serialized to a JSON string:\n{body}"
    );
}

#[test]
fn test_async_struct_param_passed_as_native_object_not_json_string() {
    let generator = make_struct_aware_generator("my_lib", &["Opts"]);
    let trait_def = make_trait_def("Greeter", "my_lib::Greeter", vec![]);
    let bridge_cfg = make_bridge_cfg("Greeter");
    let spec = struct_param_spec(&trait_def, &bridge_cfg);

    let method = make_method_def(
        "greet",
        vec![make_param_def("opts", TypeRef::Named("Opts".to_string()), true)],
        TypeRef::Named("Doc".to_string()),
        true,
        true,
        false,
    );
    let body = generator.gen_async_method_body(&method, &spec);

    // The cloning preamble owns the core value, and the call site builds the native object.
    assert!(
        body.contains("let opts_owned = opts.clone();"),
        "async preamble must clone the core struct value for native construction:\n{body}"
    );
    assert!(
        body.contains("Opts::from(opts_owned)"),
        "async struct param must be constructed as the binding's native Python object:\n{body}"
    );
    assert!(
        !body.contains("serde_json::to_string(opts)") && !body.contains("opts_json"),
        "async struct param must NOT be serialized to a JSON string:\n{body}"
    );
}

#[test]
fn test_enum_and_unknown_params_keep_json_string_representation() {
    // `Mood` (enum) and `Widget` (unknown) are NOT in the struct allowlist, so they keep the
    // prior JSON-string representation.
    let generator = make_struct_aware_generator("my_lib", &["Opts"]);
    let trait_def = make_trait_def("Greeter", "my_lib::Greeter", vec![]);
    let bridge_cfg = make_bridge_cfg("Greeter");
    let spec = struct_param_spec(&trait_def, &bridge_cfg);

    let method = make_method_def(
        "greet",
        vec![
            make_param_def("mood", TypeRef::Named("Mood".to_string()), true),
            make_param_def("widget", TypeRef::Named("Widget".to_string()), true),
        ],
        TypeRef::Unit,
        false,
        false,
        false,
    );
    let sync_body = generator.gen_sync_method_body(&method, &spec);
    assert!(
        sync_body.contains("serde_json::to_string(mood)") && sync_body.contains("serde_json::to_string(widget)"),
        "non-struct Named params must keep the JSON-string representation:\n{sync_body}"
    );
    assert!(
        !sync_body.contains("Mood::from(") && !sync_body.contains("Widget::from("),
        "non-struct Named params must NOT be constructed as native objects:\n{sync_body}"
    );
}
