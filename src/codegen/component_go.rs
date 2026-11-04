//! Go c-shared producers for synchronous native component contracts.

use crate::codegen::component::{
    ComponentMethodIr, ResolvedComponent, ResolvedProvidedContract, WireType, entry_point_symbol,
};
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::{Result, bail};
use heck::{ToSnakeCase, ToUpperCamelCase};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

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
    let dir = crate::core::config::component::component_producer_dir(
        &config.core_crate_dir(),
        &component.component_name,
        component.producer_dir.as_deref(),
    );
    let ctx = template_context(api, config, component, contracts);
    let mut files = Vec::new();
    for (path, template, generated) in [
        ("go.mod", "module.jinja", false),
        ("component.go", "bridge.go.jinja", true),
        ("component.c", "bridge.c.jinja", true),
        ("alef_internal/bridge.h", "bridge.h.jinja", true),
        ("implementation.go", "implementation.go.jinja", false),
    ] {
        files.push(GeneratedFile {
            path: dir.join(path),
            content: super::template_env::render(&format!("component_go/{template}"), ctx.clone()),
            generated_header: generated,
        });
    }
    files.extend(
        component
            .provides
            .iter()
            .map(|provided| contract_header(&dir, provided)),
    );
    Ok(files)
}

fn template_context(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    component: &ResolvedComponent,
    contracts: Vec<Value>,
) -> minijinja::Value {
    let feature_hash = super::component_producer::feature_set_hash(&component.features, component.default_features);
    crate::alef_context! {
        contracts => contracts, component => component.component_name, version => api.version,
        go_version => crate::core::template_versions::toolchain::GO_VERSION,
        has_utf8 => component.provides.iter().any(|provided| provided.contract.methods.iter().any(|method| {
            method.result == WireType::Utf8 || method.params.iter().any(|param| param.ty == WireType::Utf8)
        })),
        feature_hash => feature_hash.iter().map(u8::to_string).collect::<Vec<_>>().join(", "),
        module => format!("alef.local/{}-{}-component", config.core_crate_dir(), component.component_name),
    }
}

fn contract_header(dir: &Path, provided: &ResolvedProvidedContract) -> GeneratedFile {
    let header = provided.contract.c_header().replace(
        "typedef int32_t AlefComponentStatus;",
        "#ifndef ALEF_COMPONENT_ABI_TYPES\n#define ALEF_COMPONENT_ABI_TYPES\ntypedef int32_t AlefComponentStatus;",
    );
    let boundary = format!("typedef struct {}ApiV", provided.contract.name.to_upper_camel_case());
    GeneratedFile {
        path: dir.join(format!("{}.h", provided.contract.name.to_snake_case())),
        content: header.replacen(&boundary, &format!("#endif\n\n{boundary}"), 1),
        generated_header: true,
    }
}

fn prepare_contracts(component: &ResolvedComponent) -> Result<Vec<Value>> {
    validate_package_identifiers(component)?;
    component
        .provides
        .iter()
        .map(|provided| {
            let contract = &provided.contract;
            let constructor = &provided.implementation;
            if !crate::core::config::component::is_valid_go_constructor_identifier(constructor) {
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

fn validate_package_identifiers(component: &ResolvedComponent) -> Result<()> {
    let mut identifiers = reserved_identifiers(component);
    for provided in &component.provides {
        let contract = &provided.contract;
        let name = contract.name.to_upper_camel_case();
        for identifier in [
            name.clone(),
            format!("{name}Implementation"),
            format!("AlefGo{name}_LifecycleCreate"),
            format!("AlefGo{name}_LifecycleDestroy"),
        ] {
            register_identifier(
                &mut identifiers,
                component,
                &contract.name,
                &identifier,
                "generated declaration",
            )?;
        }
        for method in &contract.methods {
            let identifier = format!("AlefGo{name}_Method{}", method.name.to_upper_camel_case());
            register_identifier(
                &mut identifiers,
                component,
                &contract.name,
                &identifier,
                "generated method declaration",
            )?;
        }
    }
    for provided in &component.provides {
        register_identifier(
            &mut identifiers,
            component,
            &provided.contract.name,
            &provided.implementation,
            "constructor",
        )?;
    }
    Ok(())
}

fn register_identifier(
    identifiers: &mut BTreeMap<String, String>,
    component: &ResolvedComponent,
    contract: &str,
    identifier: &str,
    role: &str,
) -> Result<()> {
    if let Some(previous) = identifiers.insert(identifier.into(), format!("{role} for contract `{contract}`")) {
        bail!(
            "Go component `{}` contract `{contract}` {role} `{identifier}` conflicts with {previous}",
            component.component_name
        );
    }
    Ok(())
}

fn reserved_identifiers(component: &ResolvedComponent) -> BTreeMap<String, String> {
    let mut identifiers = [
        "main",
        "alefBuffer",
        "alefRecover",
        "alefInput",
        "outInstance",
        "outError",
        "status",
        "C",
        "fmt",
        "cgo",
        "unsafe",
        "byte",
        "bool",
        "nil",
        "len",
        "append",
        "new",
        "recover",
        "int",
        "uint",
        "uint64",
        "false",
        "true",
    ]
    .into_iter()
    .map(|name| (name.into(), "generated bridge identifier".into()))
    .collect::<BTreeMap<_, _>>();
    for method in component
        .provides
        .iter()
        .flat_map(|provided| &provided.contract.methods)
    {
        if method.fallible {
            identifiers.insert("error".into(), "generated fallible method type".into());
        }
        for ty in std::iter::once(&method.result).chain(method.params.iter().map(|param| &param.ty)) {
            if let Some((go, _, _)) = wire_type(ty, true)
                && !go.is_empty()
            {
                identifiers.insert(go.trim_start_matches("[]").into(), "generated method type".into());
            }
            if *ty == WireType::Utf8 {
                identifiers.insert("utf8".into(), "generated UTF-8 import".into());
            }
        }
    }
    identifiers
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
