use super::*;

#[test]
fn test_gen_bridge_wrapper_struct_contains_struct_name() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_wrapper_struct(&spec, &generator);
    assert!(
        result.contains("pub struct PyOcrBackendBridge"),
        "missing struct declaration in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_wrapper_struct_contains_inner_field() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_wrapper_struct(&spec, &generator);
    assert!(result.contains("inner: Py<PyAny>"), "missing inner field in:\n{result}");
}

#[test]
fn test_gen_bridge_wrapper_struct_contains_cached_name() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_wrapper_struct(&spec, &generator);
    assert!(
        result.contains("cached_name: String"),
        "missing cached_name field in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_plugin_impl_returns_none_when_no_super_trait() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    assert!(gen_bridge_plugin_impl(&spec, &generator).is_none());
}

#[test]
fn test_gen_bridge_plugin_impl_returns_some_when_super_trait_configured() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    assert!(gen_bridge_plugin_impl(&spec, &generator).is_some());
}

#[test]
fn test_gen_bridge_plugin_impl_uses_qualified_super_trait_path() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(
        result.contains("impl mylib::Plugin for PyOcrBackendBridge"),
        "missing qualified super-trait path in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_plugin_impl_uses_already_qualified_super_trait_path() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("other_crate::Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(
        result.contains("impl other_crate::Plugin for PyOcrBackendBridge"),
        "wrong super-trait path in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_plugin_impl_contains_name_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(
        result.contains("fn name(") && result.contains("cached_name"),
        "missing name() using cached_name in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_plugin_impl_contains_version_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(result.contains("fn version("), "missing version() in:\n{result}");
}

#[test]
fn test_gen_bridge_plugin_impl_contains_initialize_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(result.contains("fn initialize("), "missing initialize() in:\n{result}");
}

#[test]
fn test_gen_bridge_plugin_impl_contains_shutdown_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_plugin_impl(&spec, &generator).unwrap();
    assert!(result.contains("fn shutdown("), "missing shutdown() in:\n{result}");
}

#[test]
fn test_gen_bridge_trait_impl_includes_impl_header() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(
        result.contains("impl mylib::OcrBackend for PyOcrBackendBridge"),
        "missing impl header in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_trait_impl_includes_method_signatures() {
    let methods = vec![make_method(
        "process",
        vec![],
        TypeRef::String,
        false,
        false,
        None,
        None,
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(result.contains("fn process("), "missing method signature in:\n{result}");
}

#[test]
fn test_gen_bridge_trait_impl_includes_method_body_from_generator() {
    let methods = vec![make_method(
        "process",
        vec![],
        TypeRef::String,
        false,
        false,
        None,
        None,
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(
        result.contains("// sync body for process"),
        "missing sync method body in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_trait_impl_async_method_uses_async_body() {
    let methods = vec![make_method(
        "process_async",
        vec![],
        TypeRef::String,
        true,
        false,
        None,
        None,
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(
        result.contains("// async body for process_async"),
        "missing async method body in:\n{result}"
    );
    assert!(
        result.contains("async fn process_async("),
        "missing async keyword in method signature in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_trait_impl_filters_trait_source_methods() {
    let methods = vec![
        make_method("own_method", vec![], TypeRef::String, false, false, None, None),
        make_method(
            "inherited_method",
            vec![],
            TypeRef::String,
            false,
            false,
            Some("other_crate::OtherTrait"),
            None,
        ),
    ];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(
        result.contains("fn own_method("),
        "own method should be present in:\n{result}"
    );
    assert!(
        !result.contains("fn inherited_method("),
        "inherited method should be filtered out in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_trait_impl_method_with_params() {
    let params = vec![
        make_param("input", TypeRef::String, true),
        make_param("count", TypeRef::Primitive(PrimitiveType::U32), false),
    ];
    let methods = vec![make_method(
        "process",
        params,
        TypeRef::String,
        false,
        false,
        None,
        None,
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(result.contains("input: &str"), "missing &str param in:\n{result}");
    assert!(result.contains("count: u32"), "missing u32 param in:\n{result}");
}

#[test]
fn test_gen_bridge_trait_impl_return_type_with_error() {
    let methods = vec![make_method(
        "process",
        vec![],
        TypeRef::String,
        false,
        false,
        None,
        Some("MyError"),
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_trait_impl(&spec, &generator);
    assert!(
        result.contains("-> std::result::Result<String, mylib::MyError>"),
        "missing std::result::Result return type in:\n{result}"
    );
}

#[test]
fn test_gen_bridge_registration_fn_returns_none_without_register_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    assert!(gen_bridge_registration_fn(&spec, &generator).is_none());
}

#[test]
fn test_gen_bridge_registration_fn_returns_some_with_register_fn() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let result = gen_bridge_registration_fn(&spec, &generator);
    assert!(result.is_some());
    let code = result.unwrap();
    assert!(
        code.contains("register_ocr_backend"),
        "missing register fn name in:\n{code}"
    );
}

#[test]
fn test_gen_bridge_all_includes_imports() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(output.imports.contains(&"pyo3::prelude::*".to_string()));
    assert!(output.imports.contains(&"pyo3::types::PyString".to_string()));
}

#[test]
fn test_gen_bridge_all_includes_wrapper_struct() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        output.code.contains("pub struct PyOcrBackendBridge"),
        "missing struct in:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_includes_constructor() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        output.code.contains("pub fn new("),
        "missing constructor in:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_includes_trait_impl() {
    let methods = vec![make_method(
        "process",
        vec![],
        TypeRef::String,
        false,
        false,
        None,
        None,
    )];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        output.code.contains("impl mylib::OcrBackend for PyOcrBackendBridge"),
        "missing trait impl in:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_includes_plugin_impl_when_super_trait_set() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(Some("Plugin"), None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        output.code.contains("impl mylib::Plugin for PyOcrBackendBridge"),
        "missing plugin impl in:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_no_plugin_impl_when_no_super_trait() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        !output.code.contains("fn name(") || !output.code.contains("cached_name"),
        "unexpected plugin impl present without super_trait"
    );
}

#[test]
fn test_gen_bridge_all_includes_registration_fn_when_configured() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        output.code.contains("register_ocr_backend"),
        "missing registration fn in:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_no_registration_fn_when_absent() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    assert!(
        !output.code.contains("register_ocr_backend"),
        "unexpected registration fn present:\n{}",
        output.code
    );
}

#[test]
fn test_gen_bridge_all_ordering_struct_before_trait_impl() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let generator = MockBridgeGenerator;
    let output = gen_bridge_all(&spec, &generator);
    let struct_pos = output.code.find("pub struct PyOcrBackendBridge").unwrap();
    let impl_pos = output
        .code
        .find("impl mylib::OcrBackend for PyOcrBackendBridge")
        .unwrap();
    assert!(struct_pos < impl_pos, "struct should appear before trait impl");
}

/// Mock generator that opts in to defaulted-method forwarding.
struct MockForwardingGenerator;

impl TraitBridgeGenerator for MockForwardingGenerator {
    fn foreign_object_type(&self) -> &str {
        MockBridgeGenerator.foreign_object_type()
    }

    fn bridge_imports(&self) -> Vec<String> {
        MockBridgeGenerator.bridge_imports()
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_sync_method_body(method, spec)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_async_method_body(method, spec)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_constructor(spec)
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_registration_fn(spec)
    }

    fn gen_method_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!("self.host_has(\"{}\")", method.name))
    }
}

fn ocr_like_trait() -> TypeDef {
    make_type_def(
        "OcrBackend",
        "mylib::OcrBackend",
        vec![
            make_method(
                "process_image",
                vec![make_param("image_bytes", TypeRef::Bytes, true)],
                TypeRef::Named("ExtractionResult".to_string()),
                true,
                false,
                None,
                Some("MyError"),
            ),
            make_method(
                "supports_language",
                vec![],
                TypeRef::Primitive(PrimitiveType::Bool),
                false,
                false,
                None,
                None,
            ),
            make_method(
                "supports_table_detection",
                vec![],
                TypeRef::Primitive(PrimitiveType::Bool),
                false,
                true,
                None,
                None,
            ),
            make_method(
                "process_document",
                vec![make_param("path", TypeRef::Path, true)],
                TypeRef::Named("ExtractionResult".to_string()),
                true,
                true,
                None,
                Some("MyError"),
            ),
        ],
    )
}

#[test]
fn trait_impl_without_hook_still_skips_defaulted_methods() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let result = gen_bridge_trait_impl(&spec, &MockBridgeGenerator);
    assert!(
        !result.contains("fn supports_table_detection"),
        "defaulted method must stay omitted without the presence hook:\n{result}"
    );
}

#[test]
fn trait_impl_with_hook_forwards_defaulted_methods_with_guard() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let result = gen_bridge_trait_impl(&spec, &MockForwardingGenerator);
    assert!(
        result.contains("fn supports_table_detection"),
        "defaulted method must be emitted when the generator opts in:\n{result}"
    );
    assert!(
        result.contains("self.host_has(\"supports_table_detection\")"),
        "presence guard must use the generator's check expression:\n{result}"
    );
    assert!(
        result.contains("PyOcrBackendBridgeDefaultSupportsTableDetection(self).supports_table_detection()"),
        "fallback must delegate to the trait's default body via the per-method delegate:\n{result}"
    );
    assert!(
        result.contains("PyOcrBackendBridgeDefaultProcessDocument(self).process_document(path).await"),
        "async fallback must await the delegate call:\n{result}"
    );
}

#[test]
fn default_delegates_route_other_methods_back_through_bridge() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let output = gen_bridge_all(&spec, &MockForwardingGenerator);
    let code = &output.code;
    assert!(
        code.contains("struct PyOcrBackendBridgeDefaultSupportsTableDetection<'a>(&'a PyOcrBackendBridge);"),
        "delegate struct missing:\n{code}"
    );
    let delegate_impl_start = code
        .find("impl mylib::OcrBackend for PyOcrBackendBridgeDefaultSupportsTableDetection<'_>")
        .expect("delegate trait impl missing");
    let delegate_impl = &code[delegate_impl_start..];
    let delegate_impl_end = delegate_impl
        .find("\n}\n")
        .map(|i| &delegate_impl[..i])
        .unwrap_or(delegate_impl);
    assert!(
        delegate_impl_end.contains("fn process_image"),
        "delegate must forward required methods:\n{delegate_impl_end}"
    );
    assert!(
        delegate_impl_end.contains("fn process_document"),
        "delegate must forward the other defaulted method back through the bridge:\n{delegate_impl_end}"
    );
    assert!(
        !delegate_impl_end.contains("fn supports_table_detection"),
        "delegate must not override its own method:\n{delegate_impl_end}"
    );
    assert!(
        code.contains("impl mylib::Plugin for PyOcrBackendBridgeDefaultSupportsTableDetection<'_>"),
        "delegate must forward the Plugin super-trait:\n{code}"
    );
}

#[test]
fn no_delegates_emitted_without_hook() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let output = gen_bridge_all(&spec, &MockBridgeGenerator);
    assert!(
        !output.code.contains("DefaultSupportsTableDetection"),
        "delegates must not be emitted when the generator does not opt in:\n{}",
        output.code
    );
}

/// Mock generator that also opts lifecycle methods in to no-op tolerance.
struct MockLifecycleGenerator;

impl TraitBridgeGenerator for MockLifecycleGenerator {
    fn foreign_object_type(&self) -> &str {
        MockBridgeGenerator.foreign_object_type()
    }

    fn bridge_imports(&self) -> Vec<String> {
        MockBridgeGenerator.bridge_imports()
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_sync_method_body(method, spec)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_async_method_body(method, spec)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_constructor(spec)
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        MockBridgeGenerator.gen_registration_fn(spec)
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!("self.host_has(\"{}\")", method.name))
    }
}

#[test]
fn lifecycle_methods_noop_when_host_lacks_them() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let result = gen_bridge_plugin_impl(&spec, &MockLifecycleGenerator).expect("plugin impl");
    assert!(
        result.contains("if !(self.host_has(\"initialize\")) {"),
        "initialize must be guarded by the lifecycle presence check:\n{result}"
    );
    assert!(
        result.contains("if !(self.host_has(\"shutdown\")) {"),
        "shutdown must be guarded by the lifecycle presence check:\n{result}"
    );
}

#[test]
fn lifecycle_methods_unguarded_without_hook() {
    let trait_def = ocr_like_trait();
    let config = make_trait_bridge_config(Some("Plugin"), Some("register_ocr_backend"));
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let result = gen_bridge_plugin_impl(&spec, &MockBridgeGenerator).expect("plugin impl");
    assert!(
        !result.contains("host_has"),
        "no guard without the lifecycle hook:\n{result}"
    );
}
