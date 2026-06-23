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
