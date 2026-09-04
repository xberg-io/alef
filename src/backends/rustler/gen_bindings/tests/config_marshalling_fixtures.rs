use super::test_api;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn config_param() -> ParamDef {
    ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        ..Default::default()
    }
}

fn configs_param() -> ParamDef {
    ParamDef {
        name: "configs".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Config".to_string()))),
        ..Default::default()
    }
}

fn config_method(
    name: &str,
    parameter: ParamDef,
    return_type: TypeRef,
    receiver: ReceiverKind,
    is_async: bool,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![parameter],
        return_type,
        is_async,
        error_type: error_type.map(str::to_string),
        receiver: Some(receiver),
        cfg: None,
        ..Default::default()
    }
}

fn config_builder_methods() -> Vec<MethodDef> {
    vec![
        config_method(
            "configure",
            config_param(),
            TypeRef::Unit,
            ReceiverKind::Ref,
            false,
            None,
        ),
        config_method(
            "with_config",
            config_param(),
            TypeRef::Named("Builder".to_string()),
            ReceiverKind::RefMut,
            false,
            None,
        ),
        config_method(
            "with_config_later",
            config_param(),
            TypeRef::Named("Builder".to_string()),
            ReceiverKind::RefMut,
            true,
            None,
        ),
        config_method(
            "try_with_config",
            config_param(),
            TypeRef::Named("Builder".to_string()),
            ReceiverKind::RefMut,
            false,
            Some("BuildError"),
        ),
    ]
}

fn config_builder_type() -> TypeDef {
    let mut methods = config_builder_methods();
    methods.extend([
        config_method(
            "configure_later",
            config_param(),
            TypeRef::Unit,
            ReceiverKind::Ref,
            true,
            None,
        ),
        config_method(
            "configure_many",
            configs_param(),
            TypeRef::Unit,
            ReceiverKind::Ref,
            false,
            None,
        ),
        config_method(
            "configure_many_later",
            configs_param(),
            TypeRef::Unit,
            ReceiverKind::Ref,
            true,
            None,
        ),
    ]);
    TypeDef {
        name: "Builder".to_string(),
        rust_path: "my_lib::Builder".to_string(),
        original_rust_path: "my_lib::Builder".to_string(),
        methods,
        is_opaque: true,
        is_clone: true,
        ..Default::default()
    }
}

fn config_registry_type() -> TypeDef {
    TypeDef {
        name: "Registry".to_string(),
        rust_path: "my_lib::Registry".to_string(),
        original_rust_path: "my_lib::Registry".to_string(),
        methods: vec![
            config_method(
                "configure_many",
                configs_param(),
                TypeRef::Unit,
                ReceiverKind::Ref,
                false,
                None,
            ),
            config_method(
                "configure_many_later",
                configs_param(),
                TypeRef::Unit,
                ReceiverKind::Ref,
                true,
                None,
            ),
        ],
        is_clone: true,
        ..Default::default()
    }
}

fn free_function(name: &str, parameter: ParamDef, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: format!("my_lib::{name}"),
        params: vec![parameter],
        return_type: TypeRef::Unit,
        is_async,
        ..Default::default()
    }
}

fn merge_function() -> FunctionDef {
    let mut first = config_param();
    first.name = "first".to_string();
    let mut second = config_param();
    second.name = "second".to_string();
    FunctionDef {
        name: "merge".to_string(),
        rust_path: "my_lib::merge".to_string(),
        original_rust_path: "my_lib::merge".to_string(),
        params: vec![first, second],
        return_type: TypeRef::Unit,
        ..Default::default()
    }
}

/// A fallible, parameterless method on `Config` itself (`Config::validate(&self)`), matching
/// the real-world shape of `ExtractionConfig::validate`. `Config` is a non-opaque type with
/// `has_default: true`, so its receiver — not just its params — must go through the same
/// JSON marshalling boundary as every other default-typed value at the NIF edge.
fn config_validate_method() -> MethodDef {
    MethodDef {
        name: "validate".to_string(),
        return_type: TypeRef::Unit,
        error_type: Some("ValidationError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

pub(super) fn config_marshalling_api_surface() -> ApiSurface {
    let config_type = TypeDef {
        name: "Config".to_string(),
        rust_path: "my_lib::Config".to_string(),
        original_rust_path: "my_lib::Config".to_string(),
        has_default: true,
        has_serde: true,
        methods: vec![config_validate_method()],
        ..Default::default()
    };
    let mut mutable_configs = configs_param();
    mutable_configs.is_ref = true;
    mutable_configs.is_mut = true;
    let mut api = test_api();
    api.types = vec![config_type, config_builder_type(), config_registry_type()];
    api.functions = vec![
        free_function("build", config_param(), false),
        free_function("build_async", config_param(), true),
        free_function("mutate_configs", mutable_configs.clone(), false),
        free_function("mutate_configs_async", mutable_configs, true),
        merge_function(),
    ];
    api
}

fn json_param(sanitized: bool) -> ParamDef {
    ParamDef {
        name: "metadata".to_string(),
        ty: TypeRef::Json,
        sanitized,
        ..Default::default()
    }
}

fn json_method(name: &str, is_async: bool, sanitized: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![json_param(false)],
        return_type: TypeRef::Unit,
        is_async,
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized,
        ..Default::default()
    }
}

fn json_builder_type() -> TypeDef {
    TypeDef {
        name: "Builder".to_string(),
        rust_path: "my_lib::Builder".to_string(),
        original_rust_path: "my_lib::Builder".to_string(),
        methods: vec![
            json_method("set_metadata", false, false),
            json_method("set_metadata_later", true, false),
            json_method("sanitized_metadata", false, true),
            json_method("sanitized_metadata_later", true, true),
        ],
        is_opaque: true,
        is_clone: true,
        ..Default::default()
    }
}

pub(super) fn json_marshalling_api_surface() -> ApiSurface {
    let mut api = test_api();
    api.types = vec![json_builder_type()];
    api.functions = vec![
        free_function("render", json_param(false), false),
        free_function("render_async", json_param(false), true),
        free_function("nondelegated_json", json_param(true), false),
        free_function("nondelegated_json_async", json_param(true), true),
    ];
    api
}
