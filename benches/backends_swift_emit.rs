use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,

        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
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
    }
}

fn make_enum(name: &str, variant_count: usize) -> EnumDef {
    let variants = (0..variant_count)
        .map(|i| EnumVariant {
            name: format!("Variant{}", i),
            fields: vec![],
            doc: String::new(),
            is_default: i == 0,
            serde_rename: None,
            is_tuple: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        })
        .collect();

    EnumDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        variants,
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,

        is_copy: false,
        has_serde: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: Vec::new(),
        version: Default::default(),
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("bench config must parse");
    cfg.resolve().expect("bench config must resolve").remove(0)
}

fn make_synthetic_api() -> ApiSurface {
    // 100 types, each with 5 fields of mixed types
    let mut types = Vec::new();
    for i in 1..=100 {
        let fields = vec![
            make_field(&format!("field_a{}", i), TypeRef::String, i % 3 == 0),
            make_field(&format!("field_b{}", i), TypeRef::Primitive(PrimitiveType::I32), false),
            make_field(
                &format!("field_c{}", i),
                TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
                false,
            ),
            make_field(
                &format!("field_d{}", i),
                TypeRef::Vec(Box::new(TypeRef::String)),
                i % 2 == 0,
            ),
            make_field(
                &format!("field_e{}", i),
                if i % 4 == 0 {
                    TypeRef::Named(format!("Type{:03}", (i - 1) % 100 + 1))
                } else {
                    TypeRef::Primitive(PrimitiveType::Bool)
                },
                false,
            ),
        ];
        types.push(make_type(&format!("Type{:03}", i), fields));
    }

    // 200 functions with varied signatures
    let mut functions = Vec::new();
    for i in 1..=200 {
        let params = vec![
            make_param(&format!("param_a{}", i), TypeRef::String),
            make_param(&format!("param_b{}", i), TypeRef::Primitive(PrimitiveType::I32)),
            if i % 2 == 0 {
                make_param(
                    &format!("param_c{}", i),
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                )
            } else {
                make_param(&format!("param_c{}", i), TypeRef::Vec(Box::new(TypeRef::String)))
            },
        ];

        let return_type = match i % 5 {
            0 => TypeRef::String,
            1 => TypeRef::Primitive(PrimitiveType::I32),
            2 => TypeRef::Vec(Box::new(TypeRef::String)),
            3 => TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
            _ => TypeRef::Named(format!("Type{:03}", i % 100 + 1)),
        };

        functions.push(FunctionDef {
            name: format!("fn_{:03}", i),
            rust_path: format!("demo::fn_{:03}", i),
            original_rust_path: String::new(),
            params,
            return_type,
            is_async: i % 7 == 0,
            error_type: if i % 10 == 0 { Some("Error".to_string()) } else { None },
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
        });
    }

    // 50 enums with mixed variants
    let mut enums = Vec::new();
    for i in 1..=50 {
        enums.push(make_enum(&format!("Enum{:03}", i), (i % 10) + 2));
    }

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types,
        functions,
        enums,
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn emit_synthetic_sample(c: &mut Criterion) {
    let api = black_box(make_synthetic_api());
    let config = black_box(make_config());

    c.bench_function("emit_synthetic_sample", |b| {
        b.iter(|| {
            let _ = SwiftBackend.generate_bindings(&api, &config);
        })
    });
}

criterion_group!(benches, emit_synthetic_sample);
criterion_main!(benches);
