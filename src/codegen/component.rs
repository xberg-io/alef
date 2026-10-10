//! Stable component-contract IR and C header generation.

pub use alef_component_runtime::ComponentDeliveryMode;

use crate::core::config::component::SUPPORTED_COMPONENT_TARGETS;
use crate::core::config::{ComponentConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use anyhow::{Context as _, Result, bail};
use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A canonical, hashable component contract derived from a Rust trait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentContractIr {
    pub name: String,
    pub interface_version: u32,
    pub trait_path: String,
    pub methods: Vec<ComponentMethodIr>,
    pub records: Vec<ComponentRecordIr>,
    pub enums: Vec<ComponentEnumIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentMethodIr {
    pub name: String,
    pub is_async: bool,
    pub params: Vec<ComponentParamIr>,
    pub result: WireType,
    pub fallible: bool,
    /// The trait method's declared error type name (e.g. `"Error"`), when `fallible`.
    ///
    /// The producer only needs `fallible` to decide whether to emit an error slot; a
    /// host-side proxy additionally needs this name to construct a typed error from the
    /// message a component returns on failure.
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentParamIr {
    pub name: String,
    pub ty: WireType,
    /// Whether the Rust contract borrows this value for the duration of the call.
    pub borrowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentRecordIr {
    pub name: String,
    pub fields: Vec<ComponentParamIr>,
}

/// A data-free enum referenced by a contract's methods or records.
///
/// Variant *names* (and their declaration order) are hashed alongside the
/// contract: the wire representation is just the discriminant, so adding,
/// removing, or reordering a variant changes what a peer that skipped the
/// rebuild would decode, even though [`WireType::Enum`] itself only carries
/// the enum's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentEnumIr {
    pub name: String,
    pub variants: Vec<String>,
}

/// Layout-independent values allowed across a component boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WireType {
    Unit,
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Utf8,
    Char,
    Path,
    Bytes,
    Optional(Box<WireType>),
    Slice(Box<WireType>),
    Record(String),
    Enum(String),
    Opaque(String),
}

/// One contract a resolved component provides: its canonical IR, the hash
/// derived from it, and the Rust type implementing it.
#[derive(Debug, Clone)]
pub struct ResolvedProvidedContract {
    pub implementation: String,
    pub contract: ComponentContractIr,
    pub contract_hash: [u8; 32],
}

/// A configured component after feature-gated extraction and ABI-drift checks.
#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    pub component_name: String,
    pub features: Vec<String>,
    pub default_features: bool,
    /// Targets this component is downloaded for; excludes anything in `bundled_on`.
    pub targets: Vec<String>,
    pub bundled_on: Vec<String>,
    pub provides: Vec<ResolvedProvidedContract>,
}

impl ResolvedComponent {
    /// Whether `target` downloads this component or must link it into the binding.
    ///
    /// No backend switches on this yet -- it exists so the future host-linking
    /// work has a typed hook to read instead of re-deriving `bundled_on` membership.
    #[must_use]
    pub fn delivery_mode(&self, target: &str) -> ComponentDeliveryMode {
        if self.bundled_on.iter().any(|bundled| bundled == target) {
            ComponentDeliveryMode::Bundled
        } else {
            ComponentDeliveryMode::Download
        }
    }
}

/// The concrete download targets for a component: its explicit `targets` list, or
/// every [`SUPPORTED_COMPONENT_TARGETS`] target enabled by the crate's `[targets]`
/// table and not listed in `bundled_on`.
#[must_use]
pub fn resolve_component_targets(config: &ResolvedCrateConfig, component: &ComponentConfig) -> Vec<String> {
    match &component.targets {
        Some(explicit) => explicit.clone(),
        None => SUPPORTED_COMPONENT_TARGETS
            .iter()
            .copied()
            .filter(|triple| crate::publish::platform::target_triple_enabled(&config.targets, triple))
            .filter(|triple| !component.bundled_on.iter().any(|bundled| bundled == triple))
            .map(str::to_string)
            .collect(),
    }
}

/// Resolve every configured component and reject feature-dependent ABI drift.
pub fn resolve_components(api: &ApiSurface, config: &ResolvedCrateConfig) -> Result<Vec<ResolvedComponent>> {
    let contracts = config
        .component_contracts
        .iter()
        .map(|contract| (contract.name.as_str(), contract))
        .collect::<BTreeMap<_, _>>();
    let mut expected_hashes = BTreeMap::<&str, [u8; 32]>::new();
    let mut resolved = Vec::with_capacity(config.components.len());

    for component in &config.components {
        let enabled = component
            .features
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let component_api = api.with_cfg_filtered_deep(&enabled);
        let mut provides = Vec::with_capacity(component.provides.len());
        for entry in &component.provides {
            let contract_config = contracts.get(entry.contract.as_str()).with_context(|| {
                format!(
                    "component `{}` references a missing contract `{}`",
                    component.name, entry.contract
                )
            })?;
            let contract = ComponentContractIr::from_trait(
                &component_api,
                &contract_config.name,
                &contract_config.trait_path,
                contract_config.interface_version,
            )?;
            let hash = contract.hash()?;
            if let Some(expected) = expected_hashes.get(entry.contract.as_str()) {
                if expected != &hash {
                    bail!(
                        "component `{}` changes contract `{}` under its feature set; component features may change implementations, not ABI signatures",
                        component.name,
                        entry.contract
                    );
                }
            } else {
                expected_hashes.insert(entry.contract.as_str(), hash);
            }
            provides.push(ResolvedProvidedContract {
                implementation: entry.implementation.clone(),
                contract,
                contract_hash: hash,
            });
        }
        resolved.push(ResolvedComponent {
            component_name: component.name.clone(),
            features: component.features.clone(),
            default_features: component.default_features,
            targets: resolve_component_targets(config, component),
            bundled_on: component.bundled_on.clone(),
            provides,
        });
    }

    Ok(resolved)
}

impl ComponentContractIr {
    /// Build a contract from an extracted trait path or short trait name.
    pub fn from_trait(api: &ApiSurface, name: &str, trait_path: &str, interface_version: u32) -> Result<Self> {
        let trait_def = find_trait(api, trait_path)?;
        let mut records = BTreeMap::new();
        let mut enums = BTreeMap::new();
        let methods = trait_def
            .methods
            .iter()
            .map(|method| map_method(api, method, &mut records, &mut enums))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            name: name.to_string(),
            interface_version,
            trait_path: trait_def.rust_path.clone(),
            methods,
            records: records.into_values().collect(),
            enums: enums.into_values().collect(),
        })
    }

    /// BLAKE3 hash of the canonical JSON representation.
    pub fn hash(&self) -> Result<[u8; 32]> {
        let encoded = serde_json::to_vec(self).context("serializing canonical component contract")?;
        Ok(*blake3::hash(&encoded).as_bytes())
    }

    pub fn hash_hex(&self) -> Result<String> {
        Ok(blake3::Hash::from_bytes(self.hash()?).to_hex().to_string())
    }

    /// Emit the public C declarations for this contract's typed function table.
    pub fn c_header(&self) -> String {
        let guard = format!("ALEF_COMPONENT_{}_H", self.name.to_shouty_snake_case());
        let contract = self.name.to_upper_camel_case();
        let mut out = String::new();
        let _ = writeln!(out, "#ifndef {guard}");
        let _ = writeln!(out, "#define {guard}\n");
        out.push_str("#include <stddef.h>\n#include <stdint.h>\n\n");
        out.push_str("typedef int32_t AlefComponentStatus;\n");
        out.push_str("typedef struct { const char *ptr; size_t len; } AlefComponentStr;\n");
        out.push_str("typedef struct { const uint8_t *ptr; size_t len; } AlefComponentSlice;\n");
        out.push_str("typedef void (*AlefComponentBufferFree)(void *, uint8_t *, size_t, size_t);\n");
        out.push_str("typedef struct { uint8_t *ptr; size_t len; size_t capacity; void *context; AlefComponentBufferFree free; } AlefComponentOwnedBuffer;\n");
        out.push_str("typedef struct { size_t struct_size; uint32_t abi_major; uint32_t abi_minor; void *context; void (*log)(void *, uint32_t, AlefComponentStr); } AlefComponentHostApiV1;\n");
        out.push_str("typedef void (*AlefComponentTaskCallback)(void *, AlefComponentStatus, AlefComponentOwnedBuffer, AlefComponentOwnedBuffer);\n");
        out.push_str(
            "typedef AlefComponentStatus (*AlefComponentTaskStart)(void *, AlefComponentTaskCallback, void *);\n",
        );
        out.push_str("typedef struct AlefComponentTaskV1 { size_t struct_size; void *context; AlefComponentTaskStart start; AlefComponentStatus (*cancel)(void *); void (*drop)(void *); } AlefComponentTaskV1;\n");
        out.push_str("typedef struct { size_t struct_size; uint32_t abi_major; uint32_t abi_minor; AlefComponentStr component_id; AlefComponentStr component_version; uint8_t contract_hash[32]; uint8_t feature_set_hash[32]; const void *contract; size_t contract_size; AlefComponentStatus (*create)(const AlefComponentHostApiV1 *, void **, AlefComponentOwnedBuffer *); void (*destroy)(void *); } AlefComponentV1;\n\n");

        let mut declarations = BTreeSet::new();
        for record in &self.records {
            let _ = writeln!(out, "typedef struct {} {{", record.name);
            for field in &record.fields {
                emit_auxiliary_types(&field.ty, &mut declarations);
                let _ = writeln!(out, "    {} {};", c_type(&field.ty), field.name.to_snake_case());
            }
            let _ = writeln!(out, "}} {};\n", record.name);
        }
        for method in &self.methods {
            for param in &method.params {
                emit_auxiliary_types(&param.ty, &mut declarations);
            }
            emit_auxiliary_types(&method.result, &mut declarations);
        }
        for declaration in declarations {
            let _ = writeln!(out, "{declaration}");
        }
        if !out.ends_with("\n\n") {
            out.push('\n');
        }

        for method in self.methods.iter().filter(|method| method.is_async) {
            let callback = format!("{}{}Completion", contract, method.name.to_upper_camel_case());
            let result_param = result_parameter(&method.result);
            let _ = writeln!(
                out,
                "typedef void (*{callback})(void *context, AlefComponentStatus status{result_param}, AlefComponentOwnedBuffer error);"
            );
        }
        if self.methods.iter().any(|method| method.is_async) {
            out.push('\n');
        }

        let _ = writeln!(out, "typedef struct {contract}ApiV{} {{", self.interface_version);
        out.push_str("    size_t struct_size;\n");
        for method in &self.methods {
            let _ = writeln!(out, "    {};", method_pointer(&contract, method));
        }
        let _ = writeln!(out, "}} {contract}ApiV{};\n", self.interface_version);
        let _ = writeln!(
            out,
            "AlefComponentStatus {}(uint32_t, const AlefComponentHostApiV1 *, const AlefComponentV1 **);\n",
            entry_point_symbol(&self.name)
        );
        let _ = writeln!(out, "#endif /* {guard} */");
        out
    }
}

/// The C symbol a contract's producer entry point is exported under.
///
/// A component that provides more than one contract exports one such symbol
/// per contract from the same producer cdylib, since the shared `AlefComponentV1`
/// descriptor carries exactly one contract's function table.
#[must_use]
pub fn entry_point_symbol(contract_name: &str) -> String {
    format!("alef_component_entry_v1_{}", contract_name.to_snake_case())
}

fn find_trait<'a>(api: &'a ApiSurface, trait_path: &str) -> Result<&'a TypeDef> {
    let short_name = trait_path.rsplit("::").next().unwrap_or(trait_path);
    let matches = api
        .types
        .iter()
        .filter(|typ| typ.is_trait && (typ.rust_path == trait_path || typ.name == short_name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [trait_def] => Ok(trait_def),
        [] => bail!("component contract trait `{trait_path}` was not found in the extracted API"),
        _ => bail!("component contract trait `{trait_path}` is ambiguous; use its full Rust path"),
    }
}

fn map_method(
    api: &ApiSurface,
    method: &MethodDef,
    records: &mut BTreeMap<String, ComponentRecordIr>,
    enums: &mut BTreeMap<String, ComponentEnumIr>,
) -> Result<ComponentMethodIr> {
    if method.is_static || method.receiver != Some(ReceiverKind::Ref) {
        bail!(
            "component method `{}` must use an immutable `&self` receiver",
            method.name
        );
    }
    if method.sanitized {
        bail!("component method `{}` contains a sanitized type", method.name);
    }
    let params = method
        .params
        .iter()
        .map(|param| {
            if param.sanitized || param.is_mut {
                bail!("component parameter `{}::{}` is not ABI-safe", method.name, param.name);
            }
            Ok(ComponentParamIr {
                name: param.name.clone(),
                ty: map_type(api, &param.ty, records, enums)?,
                borrowed: param.is_ref,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ComponentMethodIr {
        name: method.name.clone(),
        is_async: method.is_async,
        params,
        result: map_type(api, &method.return_type, records, enums)?,
        fallible: method.error_type.is_some(),
        error_type: method.error_type.clone(),
    })
}

fn map_type(
    api: &ApiSurface,
    ty: &TypeRef,
    records: &mut BTreeMap<String, ComponentRecordIr>,
    enums: &mut BTreeMap<String, ComponentEnumIr>,
) -> Result<WireType> {
    let mapped = match ty {
        TypeRef::Unit => WireType::Unit,
        TypeRef::Primitive(primitive) => map_primitive(primitive)?,
        TypeRef::String => WireType::Utf8,
        TypeRef::Char => WireType::Char,
        TypeRef::Path => WireType::Path,
        TypeRef::Bytes => WireType::Bytes,
        TypeRef::Duration => WireType::U64,
        TypeRef::Optional(inner) => WireType::Optional(Box::new(map_type(api, inner, records, enums)?)),
        TypeRef::Vec(inner) => WireType::Slice(Box::new(map_type(api, inner, records, enums)?)),
        TypeRef::Map(_, _) => bail!("maps require an explicit component wire adapter"),
        TypeRef::Json => bail!("JSON requires an explicit component wire adapter"),
        TypeRef::Named(name) => map_named(api, name, records, enums)?,
    };
    Ok(mapped)
}

fn map_primitive(primitive: &PrimitiveType) -> Result<WireType> {
    Ok(match primitive {
        PrimitiveType::Bool => WireType::Bool,
        PrimitiveType::U8 => WireType::U8,
        PrimitiveType::U16 => WireType::U16,
        PrimitiveType::U32 => WireType::U32,
        PrimitiveType::U64 => WireType::U64,
        PrimitiveType::I8 => WireType::I8,
        PrimitiveType::I16 => WireType::I16,
        PrimitiveType::I32 => WireType::I32,
        PrimitiveType::I64 => WireType::I64,
        PrimitiveType::F32 => WireType::F32,
        PrimitiveType::F64 => WireType::F64,
        PrimitiveType::Usize | PrimitiveType::Isize => {
            bail!("usize/isize are target-dependent and cannot appear in component contracts")
        }
    })
}

fn map_named(
    api: &ApiSurface,
    name: &str,
    records: &mut BTreeMap<String, ComponentRecordIr>,
    enums: &mut BTreeMap<String, ComponentEnumIr>,
) -> Result<WireType> {
    if let Some(enum_def) = api.enums.iter().find(|candidate| candidate.name == name) {
        if enum_def.variants.iter().any(|variant| !variant.fields.is_empty()) {
            bail!("data enum `{name}` requires an explicit component wire adapter");
        }
        enums.entry(name.to_string()).or_insert_with(|| ComponentEnumIr {
            name: name.to_string(),
            variants: enum_def.variants.iter().map(|variant| variant.name.clone()).collect(),
        });
        return Ok(WireType::Enum(name.to_string()));
    }
    let Some(typ) = api.types.iter().find(|candidate| candidate.name == name) else {
        bail!("named component type `{name}` was not found in the extracted API");
    };
    if typ.is_opaque || typ.is_trait {
        return Ok(WireType::Opaque(name.to_string()));
    }
    if !records.contains_key(name) {
        records.insert(
            name.to_string(),
            ComponentRecordIr {
                name: name.to_string(),
                fields: Vec::new(),
            },
        );
        let fields = typ
            .fields
            .iter()
            .map(|field| {
                if field.sanitized {
                    bail!("record field `{name}::{}` is sanitized", field.name);
                }
                Ok(ComponentParamIr {
                    name: field.name.clone(),
                    ty: map_type(api, &field.ty, records, enums)?,
                    borrowed: false,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        records.get_mut(name).expect("record placeholder must exist").fields = fields;
    }
    Ok(WireType::Record(name.to_string()))
}

fn c_type(ty: &WireType) -> String {
    match ty {
        WireType::Unit => "void".to_string(),
        WireType::Bool | WireType::U8 => "uint8_t".to_string(),
        WireType::U16 => "uint16_t".to_string(),
        WireType::U32 => "uint32_t".to_string(),
        WireType::U64 => "uint64_t".to_string(),
        WireType::I8 => "int8_t".to_string(),
        WireType::I16 => "int16_t".to_string(),
        WireType::I32 | WireType::Enum(_) => "int32_t".to_string(),
        WireType::I64 => "int64_t".to_string(),
        WireType::F32 => "float".to_string(),
        WireType::F64 => "double".to_string(),
        WireType::Utf8 | WireType::Char | WireType::Path => "AlefComponentStr".to_string(),
        WireType::Bytes => "AlefComponentSlice".to_string(),
        WireType::Optional(inner) => format!("AlefOptional{}", wire_name(inner)),
        WireType::Slice(inner) => format!("AlefSlice{}", wire_name(inner)),
        WireType::Record(name) => name.clone(),
        WireType::Opaque(name) => format!("{name}Handle"),
    }
}

fn wire_name(ty: &WireType) -> String {
    match ty {
        WireType::Utf8 => "Utf8".to_string(),
        WireType::Char => "Char".to_string(),
        WireType::Path => "Path".to_string(),
        WireType::Bytes => "Bytes".to_string(),
        WireType::Record(name) | WireType::Enum(name) | WireType::Opaque(name) => name.to_upper_camel_case(),
        WireType::Optional(inner) => format!("Optional{}", wire_name(inner)),
        WireType::Slice(inner) => format!("Slice{}", wire_name(inner)),
        other => c_type(other).replace("_t", "").to_upper_camel_case(),
    }
}

fn emit_auxiliary_types(ty: &WireType, declarations: &mut BTreeSet<String>) {
    match ty {
        WireType::Optional(inner) => {
            emit_auxiliary_types(inner, declarations);
            declarations.insert(format!(
                "typedef struct {{ uint8_t is_some; {} value; }} {};",
                c_type(inner),
                c_type(ty)
            ));
        }
        WireType::Slice(inner) => {
            emit_auxiliary_types(inner, declarations);
            declarations.insert(format!(
                "typedef struct {{ const {} *ptr; size_t len; }} {};",
                c_type(inner),
                c_type(ty)
            ));
        }
        WireType::Opaque(name) => {
            declarations.insert(format!("typedef void *{name}Handle;"));
        }
        _ => {}
    }
}

fn result_parameter(result: &WireType) -> String {
    if result == &WireType::Unit {
        String::new()
    } else {
        format!(", {} result", c_output_type(result))
    }
}

fn method_pointer(contract: &str, method: &ComponentMethodIr) -> String {
    let mut params = vec!["void *instance".to_string()];
    params.extend(
        method
            .params
            .iter()
            .map(|param| format!("{} {}", c_type(&param.ty), param.name.to_snake_case())),
    );
    if method.is_async {
        params.push(format!(
            "{}{}Completion completion",
            contract,
            method.name.to_upper_camel_case()
        ));
        params.push("void *completion_context".to_string());
        params.push("AlefComponentTaskV1 *out_task".to_string());
    } else {
        if method.result != WireType::Unit {
            params.push(format!("{} *out_result", c_output_type(&method.result)));
        }
        params.push("AlefComponentOwnedBuffer *out_error".to_string());
    }
    format!(
        "AlefComponentStatus (*{})({})",
        method.name.to_snake_case(),
        params.join(", ")
    )
}

fn c_output_type(ty: &WireType) -> String {
    match ty {
        WireType::Utf8 | WireType::Char | WireType::Path | WireType::Bytes => "AlefComponentOwnedBuffer".to_string(),
        other => c_type(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, ParamDef};

    fn sample_api() -> ApiSurface {
        ApiSurface {
            types: vec![
                TypeDef {
                    name: "Request".into(),
                    rust_path: "demo::Request".into(),
                    fields: vec![FieldDef {
                        name: "text".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    }],
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "Extractor".into(),
                    rust_path: "demo::Extractor".into(),
                    is_trait: true,
                    is_opaque: true,
                    methods: vec![
                        MethodDef {
                            name: "extract".into(),
                            params: vec![ParamDef {
                                name: "request".into(),
                                ty: TypeRef::Named("Request".into()),
                                is_ref: true,
                                ..ParamDef::default()
                            }],
                            return_type: TypeRef::Bytes,
                            receiver: Some(ReceiverKind::Ref),
                            error_type: Some("Error".into()),
                            ..MethodDef::default()
                        },
                        MethodDef {
                            name: "warm".into(),
                            return_type: TypeRef::Unit,
                            receiver: Some(ReceiverKind::Ref),
                            is_async: true,
                            ..MethodDef::default()
                        },
                    ],
                    ..TypeDef::default()
                },
            ],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn canonical_contract_hash_is_deterministic() {
        let first = ComponentContractIr::from_trait(&sample_api(), "extractor", "demo::Extractor", 1).unwrap();
        let second = ComponentContractIr::from_trait(&sample_api(), "extractor", "Extractor", 1).unwrap();
        assert_eq!(first.hash().unwrap(), second.hash().unwrap());
        assert_eq!(first.records[0].name, "Request");
    }

    #[test]
    fn header_contains_sync_and_async_function_slots() {
        let contract = ComponentContractIr::from_trait(&sample_api(), "extractor", "demo::Extractor", 1).unwrap();
        let header = contract.c_header();
        assert!(header.contains("typedef struct ExtractorApiV1"), "{header}");
        assert!(header.contains("AlefComponentStatus (*extract)"), "{header}");
        assert!(header.contains("ExtractorWarmCompletion completion"), "{header}");
        assert!(header.contains("typedef struct Request"), "{header}");
    }

    #[test]
    fn rejects_target_sized_integer() {
        let mut api = sample_api();
        api.types[1].methods[0].params[0].ty = TypeRef::Primitive(PrimitiveType::Usize);
        let error = ComponentContractIr::from_trait(&api, "extractor", "Extractor", 1).unwrap_err();
        assert!(error.to_string().contains("target-dependent"));
    }

    fn sample_enum_api(variants: &[&str]) -> ApiSurface {
        ApiSurface {
            types: vec![TypeDef {
                name: "Coder".into(),
                rust_path: "demo::Coder".into(),
                is_trait: true,
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "encode".into(),
                    params: vec![ParamDef {
                        name: "format".into(),
                        ty: TypeRef::Named("Format".into()),
                        ..ParamDef::default()
                    }],
                    return_type: TypeRef::Unit,
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            enums: vec![EnumDef {
                name: "Format".into(),
                rust_path: "demo::Format".into(),
                variants: variants
                    .iter()
                    .map(|name| EnumVariant {
                        name: (*name).to_string(),
                        ..EnumVariant::default()
                    })
                    .collect(),
                ..EnumDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn adding_an_enum_variant_changes_the_contract_hash() {
        let before =
            ComponentContractIr::from_trait(&sample_enum_api(&["json", "yaml"]), "coder", "demo::Coder", 1).unwrap();
        let after =
            ComponentContractIr::from_trait(&sample_enum_api(&["json", "yaml", "toml"]), "coder", "demo::Coder", 1)
                .unwrap();
        assert_eq!(before.enums[0].variants, vec!["json", "yaml"]);
        assert_ne!(before.hash().unwrap(), after.hash().unwrap());
    }

    #[test]
    fn resolves_components_against_one_contract_hash() {
        let mut config = ResolvedCrateConfig {
            component_contracts: vec![ComponentContractConfig {
                name: "extractor".into(),
                trait_path: "demo::Extractor".into(),
                interface_version: 1,
            }],
            ..ResolvedCrateConfig::default()
        };
        for (name, feature) in [("pdf", "pdf"), ("office", "office")] {
            config.components.push(ComponentConfig {
                name: name.into(),
                provides: vec![ComponentProvidesConfig {
                    contract: "extractor".into(),
                    implementation: format!("demo::{}Extractor", name.to_upper_camel_case()),
                }],
                features: vec![feature.into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
            });
        }
        let components = resolve_components(&sample_api(), &config).unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(
            components[0].provides[0].contract_hash,
            components[1].provides[0].contract_hash
        );
    }

    #[test]
    fn component_can_provide_more_than_one_contract() {
        let mut config = ResolvedCrateConfig {
            component_contracts: vec![
                ComponentContractConfig {
                    name: "extractor".into(),
                    trait_path: "demo::Extractor".into(),
                    interface_version: 1,
                },
                ComponentContractConfig {
                    name: "coder".into(),
                    trait_path: "demo::Coder".into(),
                    interface_version: 1,
                },
            ],
            ..ResolvedCrateConfig::default()
        };
        let mut api = sample_api();
        api.types.push(TypeDef {
            name: "Coder".into(),
            rust_path: "demo::Coder".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "encode".into(),
                return_type: TypeRef::Bytes,
                receiver: Some(ReceiverKind::Ref),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        });
        config.components.push(ComponentConfig {
            name: "bundle".into(),
            provides: vec![
                ComponentProvidesConfig {
                    contract: "extractor".into(),
                    implementation: "demo::BundleExtractor".into(),
                },
                ComponentProvidesConfig {
                    contract: "coder".into(),
                    implementation: "demo::BundleCoder".into(),
                },
            ],
            features: vec!["bundle".into()],
            default_features: false,
            targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
            bundled_on: vec!["wasm32-unknown-unknown".into()],
        });

        let components = resolve_components(&api, &config).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].provides.len(), 2);
        assert_eq!(
            components[0].delivery_mode("wasm32-unknown-unknown"),
            ComponentDeliveryMode::Bundled
        );
        assert_eq!(
            components[0].delivery_mode("x86_64-unknown-linux-gnu"),
            ComponentDeliveryMode::Download
        );
    }

    #[test]
    fn default_targets_come_from_supported_targets_minus_bundled_on() {
        let config = ResolvedCrateConfig::default();
        let component = ComponentConfig {
            name: "fast".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "extractor".into(),
                implementation: "demo::FastExtractor".into(),
            }],
            features: vec!["fast".into()],
            default_features: false,
            targets: None,
            bundled_on: vec!["aarch64-apple-darwin".into()],
        };
        let targets = resolve_component_targets(&config, &component);
        assert_eq!(targets.len(), SUPPORTED_COMPONENT_TARGETS.len() - 1);
        assert!(!targets.iter().any(|target| target == "aarch64-apple-darwin"));
    }
}
