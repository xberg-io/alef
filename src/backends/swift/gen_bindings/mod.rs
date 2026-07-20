use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use crate::core::config::{
    AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir,
};
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::backends::swift::gen_rust_crate;
use crate::backends::swift::type_map::SwiftMapper;

mod boxes;
pub(crate) mod bridge_artifacts;
mod client;
pub(crate) mod dto;
mod enums;
mod errors;
mod forwarders;
pub(crate) mod opaque_handles;
mod overloads;
pub mod plugin_marshal;
pub mod service_api;
mod streaming;
pub mod trait_bridge;

pub struct SwiftBackend;

fn effective_exclude_types(config: &ResolvedCrateConfig, api: &ApiSurface) -> std::collections::HashSet<String> {
    let mut exclude_types: std::collections::HashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(swift) = &config.swift {
        exclude_types.extend(swift.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(api.enums.iter().filter(|e| e.binding_excluded).map(|e| e.name.clone()));
    exclude_types.extend(api.excluded_type_paths.keys().cloned());
    exclude_types
}

fn emit_sendable_conformance(out: &mut String, type_name: &str, mark: Option<&str>, comments: &[&str]) {
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_sendable_conformance.swift.jinja",
        minijinja::context! {
            type_name => type_name,
            mark => mark,
            comments => comments,
        },
    ));
}

impl Backend for SwiftBackend {
    fn name(&self) -> &str {
        "swift"
    }

    fn language(&self) -> Language {
        Language::Swift
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_name = config.swift_module();
        let mapper = SwiftMapper;

        let original_api = api;

        let core_crate_dir = config.core_crate_for_language(crate::core::config::extras::Language::Swift);
        let effective_features =
            gen_rust_crate::feature_gate::effective_swift_codegen_features(original_api, config, &core_crate_dir);
        let configured_features: std::collections::HashSet<&str> =
            effective_features.iter().map(String::as_str).collect();

        // Drop any type/enum/function whose `#[cfg(feature = "...")]` gate is not satisfied
        let cfg_filtered_api = original_api.with_cfg_filtered(&configured_features);
        let deduped_api = cfg_filtered_api.with_deduped_functions();
        let api = &deduped_api;

        let exclude_types = effective_exclude_types(config, api);
        // fields appear in the `#[swift_bridge(init)] fn new(...)` constructor extern.
        let exclude_fields: std::collections::HashSet<String> = config
            .swift
            .as_ref()
            .map(|c| c.exclude_fields.iter().cloned().collect())
            .unwrap_or_default();

        let mut imports: BTreeSet<String> = BTreeSet::new();
        imports.insert("import Foundation".to_string());
        if !api.types.is_empty() || !api.enums.is_empty() || !api.errors.is_empty() {
            imports.insert("import RustBridge".to_string());
        }

        if let Some(swift_config) = &config.swift {
            for capsule_cfg in swift_config.capsule_types.values() {
                if let Some(module_name) = capsule_cfg.host_type.split('.').next() {
                    if !module_name.is_empty() && !module_name.contains(['*', '?', '&']) {
                        imports.insert(format!("import {module_name}"));
                    }
                }
            }
        }

        let mut body = String::new();

        let capsule_present = config
            .swift
            .as_ref()
            .map(|c| !c.capsule_types.is_empty())
            .unwrap_or(false);
        if capsule_present {
            let mut aliases: Vec<String> = api
                .types
                .iter()
                .filter(|t| t.is_opaque && !t.methods.is_empty() && !exclude_types.contains(&t.name))
                .map(|t| format!("public typealias {0} = RustBridge.{0}", t.name))
                .collect();
            aliases.sort();
            aliases.dedup();
            if !aliases.is_empty() {
                body.push_str(&aliases.join("\n"));
                body.push_str("\n\n");
            }
        }

        let unit_serde_enum_names: std::collections::HashSet<String> = api
            .enums
            .iter()
            .filter(|e| !exclude_types.contains(&e.name))
            .filter(|e| e.has_serde && e.variants.iter().all(|v| v.fields.is_empty()))
            .map(|e| e.name.clone())
            .collect();

        // either form — untagged enums (`#[serde(untagged)]`) are emitted directly as
        // JSON, and tagged enums (default external tagging or `#[serde(tag = "…")]`)
        let untagged_enum_names: std::collections::HashSet<String> = api
            .enums
            .iter()
            .filter(|e| !exclude_types.contains(&e.name))
            .filter(|e| e.has_serde && e.variants.iter().any(|v| !v.fields.is_empty()))
            .map(|e| e.name.clone())
            .collect();

        let serde_struct_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde && !exclude_types.contains(&t.name))
            .map(|t| t.name.clone())
            .collect();

        let known_dto_names = dto::compute_first_class_dto_names(api, &exclude_types);

        let mut first_class_struct_names: Vec<String> = Vec::new();
        for ty in api
            .types
            .iter()
            .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
            .filter(|t| t.methods.is_empty() || !t.is_opaque && t.has_serde)
        {
            client::emit_doc_comment(&ty.doc, "", &mut body);
            if dto::can_emit_first_class_struct(ty, &mapper, &exclude_fields, &known_dto_names) {
                let dto_error_name = api
                    .errors
                    .first()
                    .map(|e| {
                        if e.name == "Error" {
                            format!("{module_name}Error")
                        } else {
                            e.name.clone()
                        }
                    })
                    .unwrap_or_else(|| {
                        let raw = config.error_type_name();
                        if raw == "Error" {
                            format!("{module_name}Error")
                        } else {
                            raw
                        }
                    });
                first_class_struct_names.push(ty.name.clone());
                dto::emit_first_class_struct(
                    ty,
                    &mapper,
                    &exclude_fields,
                    &known_dto_names,
                    &unit_serde_enum_names,
                    &untagged_enum_names,
                    &serde_struct_names,
                    &dto_error_name,
                    &configured_features,
                    &mut body,
                );
            } else {
                body.push_str(&crate::backends::swift::template_env::render(
                    "typealias.jinja",
                    minijinja::context! {
                        name => &ty.name,
                    },
                ));
            }
            body.push('\n');
        }

        for type_name in &first_class_struct_names {
            body.push_str(&crate::backends::swift::template_env::render(
                "typealias.jinja",
                minijinja::context! {
                    name => &format!("{type_name}Ref"),
                },
            ));
            body.push('\n');
            body.push_str(&crate::backends::swift::template_env::render(
                "typealias.jinja",
                minijinja::context! {
                    name => &format!("{type_name}RefMut"),
                },
            ));
            body.push('\n');
        }

        let result_type_enums: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.result_type.as_deref().map(|s| s.to_string()))
            .collect();

        for en in api.enums.iter().filter(|e| !exclude_types.contains(&e.name)) {
            if result_type_enums.contains(&en.name) {
                enums::emit_enum_without_into_rust(en, &mut body, &mapper, &known_dto_names);
            } else {
                enums::emit_enum(
                    en,
                    &mut body,
                    &mapper,
                    &known_dto_names,
                    &config.untagged_union_text_types,
                );
            }
            body.push('\n');
        }

        for error in &api.errors {
            errors::emit_error(error, &module_name, &mut body, &mapper);
            body.push('\n');
        }

        let client_constructor_types: std::collections::HashSet<&str> = config
            .swift
            .as_ref()
            .map(|c| c.client_constructor_body.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let first_class_types: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
            .filter(|t| dto::can_emit_first_class_struct(t, &mapper, &exclude_fields, &known_dto_names))
            .map(|t| t.name.clone())
            .collect();
        let mut sendable_emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for ty in api.types.iter().filter(|t| {
            !t.is_trait
                && !exclude_types.contains(&t.name)
                && !t.methods.is_empty()
                && (t.is_opaque || !t.has_serde)
                && client_constructor_types.contains(t.name.as_str())
        }) {
            client::emit_client_class(
                ty.name.as_str(),
                &ty.methods,
                &mapper,
                config,
                &first_class_types,
                &mut body,
            );
            body.push('\n');
            let streaming_adapters: Vec<&AdapterConfig> = config
                .adapters
                .iter()
                .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
                .filter(|a| !a.skip_languages.iter().any(|l| l == "swift"))
                .filter(|a| a.owner_type.as_deref() == Some(ty.name.as_str()))
                .collect();
            if !streaming_adapters.is_empty() {
                let inner_ty = ty.name.as_str();
                if sendable_emitted.insert(inner_ty.to_string()) {
                    emit_sendable_conformance(
                        &mut body,
                        inner_ty,
                        Some("streaming client inner"),
                        &[
                            "swift-bridge opaque types are not automatically Sendable.",
                            "Captured by Task.detached in streaming methods — Rust type is Send + Sync.",
                        ],
                    );
                }
            }
            for adapter in &streaming_adapters {
                client::emit_stream_handle_sendable(adapter, ty.name.as_str(), &mut body);
            }
            {
                for adapter in &streaming_adapters {
                    for param in &adapter.params {
                        let simple_ty = param.ty.rsplit("::").next().unwrap_or(&param.ty).to_string();
                        if sendable_emitted.insert(simple_ty.clone()) {
                            emit_sendable_conformance(
                                &mut body,
                                &simple_ty,
                                Some("streaming request param"),
                                &[
                                    "swift-bridge opaque types are not automatically Sendable.",
                                    "Passed into Task.detached for streaming — Rust type is Send + Sync.",
                                ],
                            );
                        }
                    }
                }
            }
        }

        client::emit_convenience_wrappers(api, &exclude_types, &mut body);

        overloads::emit_json_string_overloads(api, &exclude_types, &mut body);

        overloads::emit_from_json_forwarders(
            api,
            &exclude_types,
            &mapper,
            &exclude_fields,
            &known_dto_names,
            &mut body,
        );

        // NOTE: The `Swift{Trait}Box` class is emitted into Sources/RustBridge/ (separate file)
        bridge_artifacts::emit_inbound_protocols(api, config, &exclude_types, &mut body);

        let client_class_names: std::collections::HashSet<String> =
            client_constructor_types.iter().map(|&s| s.to_string()).collect();
        let all_enum_names: std::collections::HashSet<String> = unit_serde_enum_names
            .iter()
            .chain(untagged_enum_names.iter())
            .cloned()
            .collect();
        // Mirrors the fallback used above for first-class struct emission: prefer the ~keep
        // crate's declared error type, falling back to `{module_name}Error` when the IR ~keep
        // uses the generic placeholder name `Error`. ~keep
        let forwarder_error_type_name = api
            .errors
            .first()
            .map(|e| {
                if e.name == "Error" {
                    format!("{module_name}Error")
                } else {
                    e.name.clone()
                }
            })
            .unwrap_or_else(|| {
                let raw = config.error_type_name();
                if raw == "Error" {
                    format!("{module_name}Error")
                } else {
                    raw
                }
            });
        forwarders::emit_free_function_forwarders(
            api,
            config,
            &known_dto_names,
            &all_enum_names,
            &unit_serde_enum_names,
            &forwarder_error_type_name,
            &client_class_names,
            &exclude_types,
            &mut body,
        );

        forwarders::emit_trait_bridge_forwarders(config, &mut body);

        client::emit_streaming_free_functions(config, &first_class_types, &mut sendable_emitted, &mut body);

        {
            for ty in api
                .types
                .iter()
                .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
            {
                if sendable_emitted.insert(ty.name.clone()) {
                    emit_sendable_conformance(
                        &mut body,
                        &ty.name,
                        None,
                        &["swift-bridge opaque type used across Task.detached boundaries — Rust type is Send + Sync."],
                    );
                }
            }

            fn collect_async_vec_named<'a>(
                ty: &'a crate::core::ir::TypeRef,
                names: &mut std::collections::HashSet<&'a str>,
            ) {
                use crate::core::ir::TypeRef;
                match ty {
                    TypeRef::Vec(inner) | TypeRef::Optional(inner) => {
                        collect_async_vec_named(inner, names);
                    }
                    TypeRef::Named(n) => {
                        names.insert(n.as_str());
                    }
                    _ => {}
                }
            }

            let mut referenced_async_named: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for f in &api.functions {
                if !f.is_async || f.binding_excluded {
                    continue;
                }
                collect_async_vec_named(&f.return_type, &mut referenced_async_named);
            }
            for name in referenced_async_named {
                if sendable_emitted.insert(name.to_string()) {
                    emit_sendable_conformance(
                        &mut body,
                        name,
                        None,
                        &[
                            "swift-bridge opaque type referenced in async forwarder return — Rust type is Send + Sync.",
                            "Auto-included even when the IR filter excluded it (e.g. cfg-gated alef(skip) stub).",
                        ],
                    );
                }
            }
        }

        let imports = imports.iter().cloned().collect::<Vec<_>>().join("\n");
        let mut content = crate::backends::swift::template_env::render(
            "swift_module_header.swift.jinja",
            minijinja::context! {
                imports => imports,
            },
        );
        content.push_str(&body);

        let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
        let base_path = PathBuf::from(&base_dir);
        let path = if config.explicit_output.swift.is_some() {
            base_path.join(format!("{module_name}.swift"))
        } else {
            base_path
                .join("Sources")
                .join(&module_name)
                .join(format!("{module_name}.swift"))
        };

        let mut files = vec![GeneratedFile {
            path,
            content,
            generated_header: false,
        }];

        let rust_crate_files = gen_rust_crate::emit(original_api, config)?;
        files.extend(rust_crate_files);

        let binding_crate_name = format!("{}-swift", &api.crate_name);
        let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
        let package_root = PathBuf::from(&base_dir)
            .ancestors()
            .find(|p| p.join("Sources").is_dir())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                PathBuf::from(&base_dir)
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("packages/swift"))
            });
        if let Some(bridge_files) =
            bridge_artifacts::emit_swift_bridge_files(&api.crate_name, &binding_crate_name, &package_root)?
        {
            files.extend(bridge_files);
        }

        let rust_bridge_sources = package_root.join("Sources").join("RustBridge");
        for box_file in boxes::emit_inbound_box_files(api, config, &rust_bridge_sources) {
            files.push(box_file);
        }

        let mut box_exclude = exclude_types.clone();
        for ty in &api.types {
            if !ty.is_trait && !ty.is_opaque && ty.has_serde {
                box_exclude.insert(ty.name.clone());
            }
        }
        for box_file in boxes::emit_function_param_box_files(api, config, &rust_bridge_sources, &box_exclude) {
            files.push(box_file);
        }

        // Emit class triples for opaque handle types marked with #[swift_bridge(already_declared)]
        if let Some(opaque_file) = opaque_handles::emit_opaque_class_declarations(config, &rust_bridge_sources) {
            files.push(opaque_file);
        }

        let trait_bridge_configs: Vec<(String, &TraitBridgeConfig, &TypeDef)> = config
            .trait_bridges
            .iter()
            .filter_map(|b| {
                api.types
                    .iter()
                    .find(|t| t.is_trait && t.name == b.trait_name)
                    .map(|t| (b.trait_name.clone(), b, t))
            })
            .collect();

        let module_dir = if config.explicit_output.swift.is_some() {
            base_path.clone()
        } else {
            base_path.join("Sources").join(&module_name)
        };

        for (filename, content) in
            trait_bridge::gen_trait_bridge_files(&trait_bridge_configs, &box_exclude, &first_class_types)
        {
            let path = rust_bridge_sources.join(&filename);
            files.push(GeneratedFile {
                path,
                content,
                generated_header: false,
            });
        }

        if let Some((filename, content)) = bridge_artifacts::emit_ref_property_extensions(api) {
            let path = module_dir.join(&filename);
            files.push(GeneratedFile {
                path,
                content,
                generated_header: true,
            });
        }

        if let Some((filename, content)) = trait_bridge::gen_bridge_registration_overloads_file(&trait_bridge_configs) {
            let path = module_dir.join(&filename);
            files.push(GeneratedFile {
                path,
                content,
                generated_header: false,
            });
        }

        Ok(files)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "swift",
            crate_suffix: "-swift",
            build_dep: BuildDependency::None,
            post_build: vec![PostBuildStep::RunCommand {
                cmd: "cargo",
                args: vec![
                    "build",
                    "--manifest-path",
                    "packages/swift/rust/Cargo.toml",
                    "--release",
                ],
            }],
        })
    }

    fn build_config_with_config(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        let mut build_config = self.build_config()?;

        let binding_crate_name = format!("{}-swift", config.name);

        let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
        let package_root = PathBuf::from(&base_dir)
            .ancestors()
            .find(|p| p.join("Sources").is_dir())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                PathBuf::from(&base_dir)
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("packages/swift"))
            });

        build_config.post_build.push(PostBuildStep::MaterializeSwiftBridge {
            binding_crate_name,
            package_root: package_root.to_string_lossy().to_string(),
        });

        Some(build_config)
    }
}
