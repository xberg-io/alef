use ahash::AHashSet;
use alef::codegen::generators::enums::enum_has_data_variants;
use alef::codegen::generators::functions::{collect_explicit_core_imports, collect_trait_imports};
use alef::codegen::generators::structs::{
    can_generate_default_impl, gen_opaque_struct, gen_struct_default_impl, type_needs_mutex,
};
use alef::codegen::generators::trait_bridge::{
    TraitBridgeGenerator, TraitBridgeSpec, format_param_type, format_type_ref, gen_bridge_all, gen_bridge_trait_impl,
    gen_bridge_wrapper_struct,
};
use alef::codegen::generators::{
    AdapterBodies, AsyncPattern, RustBindingConfig, binding_helpers, gen_constructor, gen_enum, gen_function,
    gen_impl_block, gen_method, gen_opaque_impl_block, gen_static_method, gen_struct,
};
use alef::codegen::type_mapper::TypeMapper;
use alef::core::config::TraitBridgeConfig;
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType,
    ReceiverKind, TypeDef, TypeRef,
};
use std::borrow::Cow;
use std::collections::HashMap;

/// Minimal TypeMapper using plain Rust type names (no backend-specific overrides).
struct RustMapper;

impl TypeMapper for RustMapper {
    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}

fn default_cfg<'a>() -> RustBindingConfig<'a> {
    RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &["Clone", "Debug"],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[no_mangle]",
        enum_attrs: &[],
        enum_derives: &["Clone", "Debug", "PartialEq"],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "my_crate",
        async_pattern: AsyncPattern::None,
        has_serde: false,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
    }
}

fn assert_unimplemented_compile_error(result: &str, fn_name: &str) {
    assert!(
        result.contains("compile_error!"),
        "unsupported auto-delegation should emit a compile-time diagnostic"
    );
    assert!(
        result.contains(&format!("alef cannot auto-delegate `{fn_name}`")),
        "diagnostic should name the non-delegatable item"
    );
}

fn simple_type_def() -> TypeDef {
    TypeDef {
        name: "MyConfig".to_string(),
        rust_path: "my_crate::MyConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "The config name.".to_string(),
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "count".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: true,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "A minimal config type.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn simple_function_def() -> FunctionDef {
    FunctionDef {
        name: "process".to_string(),
        rust_path: "my_crate::process".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::String,
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
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        error_type: None,
        doc: "Process a string input.".to_string(),
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

fn simple_enum_def() -> EnumDef {
    EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "my_crate::OutputFormat".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Json".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Csv".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Plain".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: "Output format options.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}
#[path = "codegen_integration/async_unimplemented_lossy.rs"]
mod async_unimplemented_lossy;
#[path = "codegen_integration/basic_generation.rs"]
mod basic_generation;
#[path = "codegen_integration/binding_helper_call_args.rs"]
mod binding_helper_call_args;
#[path = "codegen_integration/binding_helper_lossy.rs"]
mod binding_helper_lossy;
#[path = "codegen_integration/binding_helper_serde_async.rs"]
mod binding_helper_serde_async;
#[path = "codegen_integration/binding_helpers.rs"]
mod binding_helpers_tests;
#[path = "codegen_integration/enums.rs"]
mod enums_tests;
#[path = "codegen_integration/fluent_builder.rs"]
mod fluent_builder;
#[path = "codegen_integration/functional_ref_mut.rs"]
mod functional_ref_mut;
#[path = "codegen_integration/functions.rs"]
mod functions_tests;
#[path = "codegen_integration/method_async_adapter.rs"]
mod method_async_adapter;
#[path = "codegen_integration/method_edge_cases.rs"]
mod method_edge_cases;
#[path = "codegen_integration/methods.rs"]
mod methods;
#[path = "codegen_integration/mutex_return_wrapping.rs"]
mod mutex_return_wrapping;
#[path = "codegen_integration/ref_param_borrowed_return_delegation.rs"]
mod ref_param_borrowed_return_delegation;
#[path = "codegen_integration/structs.rs"]
mod structs_tests;
#[path = "codegen_integration/trait_bridge.rs"]
mod trait_bridge_tests;
