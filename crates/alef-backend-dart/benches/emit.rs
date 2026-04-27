use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType,
    TypeDef, TypeRef,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

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
        serde_rename_all: None,

        is_copy: false,
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo-crate".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: ::alef_core::config::FormatConfig::default(),
        format_overrides: ::std::collections::HashMap::new(),
    }
}

fn make_synthetic_api() -> ApiSurface {
    // 100 types, each with 5 fields of mixed types
    let mut types = Vec::new();
    for i in 1..=100 {
        let fields = vec![
            make_field(
                &format!("field_a{}", i),
                TypeRef::String,
                i % 3 == 0,
            ),
            make_field(
                &format!("field_b{}", i),
                TypeRef::Primitive(PrimitiveType::I32),
                false,
            ),
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
            make_param(
                &format!("param_a{}", i),
                TypeRef::String,
            ),
            make_param(
                &format!("param_b{}", i),
                TypeRef::Primitive(PrimitiveType::I32),
            ),
            if i % 2 == 0 {
                make_param(
                    &format!("param_c{}", i),
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                )
            } else {
                make_param(
                    &format!("param_c{}", i),
                    TypeRef::Vec(Box::new(TypeRef::String)),
                )
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
    }
}

fn emit_synthetic_kreuzberg(c: &mut Criterion) {
    let api = black_box(make_synthetic_api());
    let config = black_box(make_config());

    c.bench_function("emit_synthetic_kreuzberg", |b| {
        b.iter(|| {
            let _ = DartBackend.generate_bindings(&api, &config);
        })
    });
}

criterion_group!(benches, emit_synthetic_kreuzberg);
criterion_main!(benches);
