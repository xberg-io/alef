//! Issue #380: `fn tag_record(record: &mut Record)` must not silently drop the mutation.
//!
//! These tests exercise the real `JavaBackend::generate_bindings` path end to end -- not just
//! the shared `codegen::mut_writeback` policy module in isolation -- so they prove the backend
//! is actually wired to the policy, not merely that the policy itself is correct.

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "krz-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "krz"

[crates.java]
package = "io.test.krz"
"#,
    )
}

fn record_type() -> TypeDef {
    TypeDef {
        name: "Record".to_string(),
        rust_path: "krz_lib::Record".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: "score".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: "A record that gets mutated in place.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn mut_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: true,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: CoreWrapper::None,
    }
}

fn base_api(functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "krz-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![record_type()],
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn generate_bindings_writes_back_the_mutated_record_end_to_end() {
    let api = base_api(vec![FunctionDef {
        name: "tag_record".to_string(),
        params: vec![mut_param("record", "Record")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }]);

    let files = JavaBackend
        .generate_bindings(&api, &make_config())
        .expect("generation must succeed for the supported single-&mut-DTO-param shape");
    let main_class = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".java") && f.content.contains("tagRecord"))
        .expect("the main FFI class file declaring tagRecord must be generated")
        .content;

    assert!(
        main_class.contains("public static Record tagRecord(final Record record) throws"),
        "must return the updated value instead of void, got:\n{main_class}"
    );
    assert!(
        main_class.contains("NativeLib.KRZ_TAG_RECORD.invoke(crecord);"),
        "must still call the FFI mutator, got:\n{main_class}"
    );
    assert!(
        main_class.contains("NativeLib.KRZ_RECORD_TO_JSON.invoke(crecord)"),
        "must read the mutated handle back out, got:\n{main_class}"
    );
    assert!(
        !main_class.contains("public static void tagRecord"),
        "must not regress to the lossy void-return shape, got:\n{main_class}"
    );
}

#[test]
fn generate_bindings_rejects_a_mut_dto_param_combined_with_a_non_unit_return() {
    let api = base_api(vec![FunctionDef {
        name: "tag_and_count".to_string(),
        params: vec![mut_param("record", "Record")],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        error_type: None,
        ..Default::default()
    }]);

    let err = JavaBackend
        .generate_bindings(&api, &make_config())
        .expect_err("a &mut DTO param combined with a non-unit return must fail generation");
    let message = err.to_string();
    assert!(
        message.contains("tag_and_count"),
        "the diagnostic must name the offending function, got: {message}"
    );
}
