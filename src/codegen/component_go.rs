//! Go c-shared producers for synchronous native component contracts.

use crate::codegen::component::{ComponentMethodIr, ResolvedComponent, WireType, entry_point_symbol};
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::{Result, bail};
use heck::{ToSnakeCase, ToUpperCamelCase};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    component: &ResolvedComponent,
) -> Result<Vec<GeneratedFile>> {
    if !component.extra_dependencies.is_empty() {
        bail!(
            "Go component `{}` does not support Cargo extra_dependencies",
            component.component_name
        );
    }
    let contracts = prepare_contracts(component)?;
    let dir = PathBuf::from(format!(
        "crates/{}-{}-component",
        config.core_crate_dir(),
        component.component_name.replace('_', "-")
    ));
    let feature_hash = super::component_producer::feature_set_hash(&component.features, component.default_features);
    let ctx = crate::alef_context! {
        contracts => contracts, component => component.component_name, version => api.version,
        has_utf8 => component.provides.iter().any(|provided| provided.contract.methods.iter().any(|method| {
            method.result == WireType::Utf8 || method.params.iter().any(|param| param.ty == WireType::Utf8)
        })),
        feature_hash => feature_hash.iter().map(u8::to_string).collect::<Vec<_>>().join(", "),
        module => format!("alef.local/{}-{}-component", config.core_crate_dir(), component.component_name),
    };
    let mut files = Vec::new();
    for (path, template, generated) in [
        ("go.mod", "module.jinja", true),
        ("component.go", "bridge.go.jinja", true),
        ("component.c", "bridge.c.jinja", true),
        ("component.h", "bridge.h.jinja", true),
        ("implementation.go", "implementation.go.jinja", false),
    ] {
        files.push(GeneratedFile {
            path: dir.join(path),
            content: super::template_env::render(&format!("component_go/{template}"), ctx.clone()),
            generated_header: generated,
        });
    }
    for provided in &component.provides {
        let header = provided.contract.c_header();
        let header = header.replace(
            "typedef int32_t AlefComponentStatus;",
            "#ifndef ALEF_COMPONENT_ABI_TYPES\n#define ALEF_COMPONENT_ABI_TYPES\ntypedef int32_t AlefComponentStatus;",
        );
        let boundary = format!("typedef struct {}ApiV", provided.contract.name.to_upper_camel_case());
        let header = header.replacen(&boundary, &format!("#endif\n\n{boundary}"), 1);
        files.push(GeneratedFile {
            path: dir.join(format!("{}.h", provided.contract.name.to_snake_case())),
            content: header,
            generated_header: true,
        });
    }
    Ok(files)
}

fn prepare_contracts(component: &ResolvedComponent) -> Result<Vec<Value>> {
    component
        .provides
        .iter()
        .map(|provided| {
            let contract = &provided.contract;
            let constructor = &provided.implementation;
            if !go_identifier(constructor) {
                bail!(
                    "Go component `{}` contract `{}` constructor `{constructor}` must be a Go identifier",
                    component.component_name,
                    contract.name
                );
            }
            let methods = contract
                .methods
                .iter()
                .map(|method| prepare_method(component, &contract.name, method))
                .collect::<Result<Vec<_>>>()?;
            Ok(
                json!({"name": contract.name.to_upper_camel_case(), "snake": contract.name.to_snake_case(),
            "version": contract.interface_version, "constructor": constructor,
            "entry": entry_point_symbol(&contract.name), "methods": methods,
            "hash": provided.contract_hash.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")}),
            )
        })
        .collect()
}

fn go_identifier(name: &str) -> bool {
    if ["_", "init", "main"].contains(&name) {
        return false;
    }
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && ![
            "func",
            "type",
            "var",
            "const",
            "package",
            "import",
            "map",
            "range",
            "go",
            "defer",
            "return",
            "if",
            "else",
            "for",
            "select",
            "switch",
            "case",
            "default",
            "break",
            "continue",
            "fallthrough",
            "chan",
            "struct",
            "interface",
            "goto",
        ]
        .contains(&name)
}

fn prepare_method(component: &ResolvedComponent, contract: &str, method: &ComponentMethodIr) -> Result<Value> {
    let context = format!(
        "Go component `{}` contract `{contract}` method `{}`",
        component.component_name, method.name
    );
    if method.is_async {
        bail!("{context}: async methods are unsupported");
    }
    let output = wire_type(&method.result, true)
        .ok_or_else(|| anyhow::anyhow!("{context}: unsupported result {:?}", method.result))?;
    let params = method
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let ty = wire_type(&param.ty, false)
                .ok_or_else(|| anyhow::anyhow!("{context}: unsupported parameter `{}` {:?}", param.name, param.ty))?;
            Ok(json!({"name": format!("arg{index}"), "go": ty.0, "c": ty.1, "kind": ty.2}))
        })
        .collect::<Result<Vec<Value>>>()?;
    Ok(
        json!({"name": method.name.to_upper_camel_case(), "snake": method.name.to_snake_case(),
        "params": params, "go": output.0, "c": output.1, "kind": output.2, "fallible": method.fallible}),
    )
}

fn wire_type(ty: &WireType, output: bool) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match ty {
        WireType::Unit if output => ("", "", "unit"),
        WireType::Bool => ("bool", "uint8_t", "bool"),
        WireType::U8 => ("uint8", "uint8_t", "scalar"),
        WireType::U16 => ("uint16", "uint16_t", "scalar"),
        WireType::U32 => ("uint32", "uint32_t", "scalar"),
        WireType::U64 => ("uint64", "uint64_t", "scalar"),
        WireType::I8 => ("int8", "int8_t", "scalar"),
        WireType::I16 => ("int16", "int16_t", "scalar"),
        WireType::I32 => ("int32", "int32_t", "scalar"),
        WireType::I64 => ("int64", "int64_t", "scalar"),
        WireType::F32 => ("float32", "float", "scalar"),
        WireType::F64 => ("float64", "double", "scalar"),
        WireType::Utf8 if output => ("string", "AlefComponentOwnedBuffer", "string"),
        WireType::Bytes if output => ("[]byte", "AlefComponentOwnedBuffer", "bytes"),
        WireType::Utf8 => ("string", "AlefComponentStr", "string"),
        WireType::Bytes => ("[]byte", "AlefComponentSlice", "bytes"),
        _ => return None,
    })
}
