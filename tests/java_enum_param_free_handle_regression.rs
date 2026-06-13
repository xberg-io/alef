/// Regression test for Java type parameter _FREE handle emission
///
/// When a non-opaque type with serde is used as a function parameter (marshaled via _from_json),
/// the NativeLib must emit the matching _FREE handle. However, if the type is NOT used as
/// a return type, the builder_handles section won't be created (which skips the _FREE emission).
/// This test ensures that _FREE handles are emitted for ALL types that have a _FROM_JSON handle,
/// regardless of where they appear.
///
/// Reproducer: Type exported via FFI with _from_json and _free, used only as parameter.
use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_test_config(package: &str) -> ResolvedCrateConfig {
    resolved_one(&format!(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "{package}"
"#
    ))
}

#[test]
fn param_type_with_serde_emits_free_handle_in_native_lib() {
    // Create a non-opaque type with serde that is NOT marked as a return type.
    // This simulates RegionKind which is used as a parameter but never returned by functions,
    // so the return-type path in native_lib.rs (lines 274-323) doesn't emit its _FREE handle.
    let param_type = TypeDef {
        name: "MyMode".to_string(),
        rust_path: "test_lib::MyMode".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: true,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false, // KEY: not a return type, so the return-type path won't emit _FREE
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: "Synthetic type for testing".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let function = FunctionDef {
        name: "process".to_string(),
        rust_path: "test_lib::process".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "mode".to_string(),
            ty: TypeRef::Named("MyMode".to_string()),
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
            core_wrapper: CoreWrapper::None,
        }],
        return_type: TypeRef::Primitive(PrimitiveType::I32),
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![param_type],
        functions: vec![function],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config("com.example"))
        .unwrap();

    let native_lib = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("NativeLib.java"))
        .expect("NativeLib.java")
        .content
        .as_str();

    // The emitted wrapper code will use NativeLib.TEST_MY_MODE_FROM_JSON to marshal the param
    assert!(
        native_lib.contains("TEST_MY_MODE_FROM_JSON"),
        "NativeLib must declare TEST_MY_MODE_FROM_JSON handle for serde type param"
    );

    // The matching _FREE handle MUST be declared because the wrapper code invokes it
    // to clean up the allocated pointer returned by _from_json
    assert!(
        native_lib.contains("TEST_MY_MODE_FREE"),
        "NativeLib must declare TEST_MY_MODE_FREE when TEST_MY_MODE_FROM_JSON is declared\n\n{native_lib}"
    );

    // Verify the FFI wrapper actually uses both handles
    let wrapper = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("TestLibRs.java"))
        .expect("TestLibRs.java")
        .content
        .as_str();

    assert!(
        wrapper.contains("NativeLib.TEST_MY_MODE_FROM_JSON"),
        "Wrapper must use TEST_MY_MODE_FROM_JSON for param"
    );

    assert!(
        wrapper.contains("NativeLib.TEST_MY_MODE_FREE"),
        "Wrapper must use TEST_MY_MODE_FREE to clean up param\n\n{wrapper}"
    );
}
