use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep, TraitBridgeRegistrationSurface,
};
use crate::core::config::{
    AdapterConfig, AdapterPattern, BridgeBinding, Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir,
};
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::backends::swift::gen_rust_crate;
use crate::backends::swift::type_map::SwiftMapper;

mod adjacent_codable;
pub(super) mod boxes;
pub(crate) mod bridge_artifacts;
mod client;
mod components;
pub(crate) mod dto;
mod enums;
mod errors;
mod forwarders;
mod module_imports;
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

/// The SwiftPM package root that owns the shared `Sources/RustBridge{,C}` targets, derived only
/// from the resolved output dir and whether the crate has an explicit `[crates.output] swift`
/// override -- never from the filesystem.
///
/// Two layouts are supported, matching the two branches `generate_bindings` and
/// `generate_service_api` already use for the main module file's own path:
///
/// - No override (the common case): `base_dir` (e.g. `packages/swift`, or `packages/swift/{crate}`
///   for a multi-crate workspace) already IS the package root -- the main module file is placed at
///   `<base_dir>/Sources/<Module>/<Module>.swift`, so `<base_dir>` is where `Package.swift` and the
///   shared `Sources/RustBridge{,C}` targets live too.
/// - Explicit `[crates.output] swift = "..."` override: the configured path names the FINAL leaf
///   directory the module file is written into directly (`<base_dir>/<Module>.swift`, no
///   `Sources/<Module>` nesting -- the user controls that layout). That leaf plays the role
///   `Sources/<Module>` plays in the default case, so the package root sits two levels above it,
///   the same relationship as the default layout's `<root>/Sources/<Module>`.
///
/// This used to be discovered by walking `base_dir`'s ancestors for one that already had a
/// `Sources/` directory ON DISK -- nondeterministic, since that directory is created and removed
/// by this very build (and by concurrent scaffold/test runs), so the same config could resolve to
/// a different package root depending on what else had run. Codegen output must be a pure
/// function of (IR, config); this derivation reads neither `std::fs` nor `std::env`.
///
/// `pub(crate)` rather than private: `gen_rust_crate::emit` (a sibling module) derives the
/// swift-bridge Rust crate's own directory -- `<package_root>/rust` -- from this same function,
/// so the crate `build_config_with_config` tells `cargo build --manifest-path` to build is
/// always the same crate `gen_rust_crate::emit` actually wrote to disk. ~keep
pub(crate) fn swift_package_root(base_dir: &str, has_explicit_output: bool) -> PathBuf {
    let base_path = PathBuf::from(base_dir);
    if has_explicit_output {
        base_path
            .parent()
            .and_then(|p| p.parent())
            .map(std::path::Path::to_path_buf)
            .unwrap_or(base_path)
    } else {
        base_path
    }
}

/// The binding crate name, SwiftPM package root, and Cargo manifest path shared by both
/// `build_config_with_config` (the real, release-mode `alef build` config) and
/// `generate_post_build_config` (the cheap, check-mode `alef generate`/`alef all` config) --
/// factored out so the two post-build configs can never disagree on which crate they are
/// pointing `cargo` at. ~keep
struct SwiftBuildPaths {
    binding_crate_name: String,
    package_root: String,
    manifest_path: &'static str,
}

/// Derive `SwiftBuildPaths` from `config`.
///
/// The literal `--manifest-path` baked into `build_config()` is only correct for a single,
/// default-location crate. `gen_rust_crate::emit` writes the swift-bridge Rust crate to
/// `<package_root>/rust` (see that function's own use of `swift_package_root`), so the manifest
/// path any `cargo` invocation runs against must be derived from the SAME `package_root` rather
/// than staying independent of `config.output_paths` -- a multi-crate workspace or an explicit
/// `[crates.output] swift` override moves `package_root` away from the default `packages/swift`,
/// and a manifest path that does not move with it points `cargo` at a `Cargo.toml` that was
/// never written there. `RunCommand::args` requires `&'static str`; the path is only known once
/// `config` is in hand, so it is leaked once per process -- the same pattern
/// `cli::pipeline::commands::build::build_languages` already uses for a comparable
/// computed-crate-name case. ~keep
fn swift_build_paths(config: &ResolvedCrateConfig) -> SwiftBuildPaths {
    let binding_crate_name = format!("{}-swift", config.name);

    let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
    let package_root = swift_package_root(&base_dir, config.explicit_output.swift.is_some());

    let manifest_path = package_root.join("rust").join("Cargo.toml");
    let manifest_path: &'static str = Box::leak(manifest_path.to_string_lossy().into_owned().into_boxed_str());

    SwiftBuildPaths {
        binding_crate_name,
        package_root: package_root.to_string_lossy().to_string(),
        manifest_path,
    }
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
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        if let Some(swift) = config.swift.as_ref() {
            crate::core::config::languages::require_shared_native_runtime(
                &swift.capsule_types,
                swift.shares_native_runtime,
                "swift",
            )?;
        }
        let module_name = config.swift_module();
        let mapper = SwiftMapper;

        let original_api = api;

        let core_crate_dir = config.core_crate_for_language(crate::core::config::extras::Language::Swift);
        let effective_features =
            gen_rust_crate::feature_gate::effective_swift_codegen_features(original_api, config, &core_crate_dir);
        let configured_features: std::collections::HashSet<&str> =
            effective_features.iter().map(String::as_str).collect();
        // Deliberately NOT `effective_features`: see `enums::EnumDeclarationCfg`'s doc comment. ~keep
        let configured_enum_features =
            crate::codegen::cfg::enabled_features_for_language(config, crate::core::config::extras::Language::Swift);

        // Drop any type/enum/function whose `#[cfg(feature = "...")]` gate is not satisfied
        let cfg_filtered_api = original_api.with_cfg_filtered(&configured_features);
        // ...then the same for gated *methods* on whatever survived. The Rust bridge crate drops
        // them through this identical helper on an identically-derived feature set, so the Swift
        // facade can never call an extern the bridge crate did not declare. ~keep
        let cfg_filtered_api =
            gen_rust_crate::feature_gate::with_cfg_filtered_methods(&cfg_filtered_api, &configured_features);
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
        if !api.types.is_empty() || !api.enums.is_empty() || !api.errors.is_empty() || !config.components.is_empty() {
            imports.insert("import RustBridge".to_string());
        }

        if let Some(swift_config) = &config.swift {
            for capsule_cfg in swift_config.capsule_types.values() {
                if let Some(host_module) = capsule_cfg.host_type.split('.').next()
                    && !host_module.is_empty()
                    && !host_module.contains(['*', '?', '&'])
                {
                    imports.insert(format!("import {host_module}"));
                }
            }
        }

        let mut body = String::new();
        if let Some(component_api) = components::generate(config) {
            body.push_str(&component_api);
        }

        // Types with a configured `client_constructor_body` get a real `public class`
        // via `emit_client_class` below instead of a typealias — excluded here to
        // avoid a duplicate declaration.
        let client_constructor_types: std::collections::HashSet<&str> = config
            .swift
            .as_ref()
            .map(|c| c.client_constructor_body.keys().map(String::as_str).collect())
            .unwrap_or_default();
        // Types configured as `capsule_types` are bridged to a native host type
        // (e.g. Foundation) via a dedicated forwarder, not a plain `RustBridge`
        // typealias — excluded here for the same reason.
        let capsule_type_names: std::collections::HashSet<&str> = config
            .swift
            .as_ref()
            .map(|c| c.capsule_types.keys().map(String::as_str).collect())
            .unwrap_or_default();

        // Opaque RustBridge handle types with methods (e.g. registries, compiled
        // validators — any struct whose fields are all private) are never picked up
        // by the main DTO/typealias loop below: its filter deliberately defers to
        // this block for `is_opaque && !methods.is_empty()` types. This alias is
        // unconditional — it must not be gated on unrelated per-crate Swift config
        // (previously gated on `capsule_types` being non-empty, which left every
        // crate without capsule config unable to name these types outside
        // `RustBridge` at all). See task #541.
        let mut opaque_handle_aliases: Vec<String> = api
            .types
            .iter()
            .filter(|t| !t.is_trait && t.is_opaque && !t.methods.is_empty() && !exclude_types.contains(&t.name))
            .filter(|t| !client_constructor_types.contains(t.name.as_str()))
            .filter(|t| !capsule_type_names.contains(t.name.as_str()))
            .map(|t| format!("public typealias {0} = RustBridge.{0}", t.name))
            .collect();
        opaque_handle_aliases.sort();
        opaque_handle_aliases.dedup();
        if !opaque_handle_aliases.is_empty() {
            body.push_str(&opaque_handle_aliases.join("\n"));
            body.push_str("\n\n");
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

        // `gen_rust_crate::emit` derives its `source_crate` from this same `api.crate_name`. ~keep
        let enum_declaration_cfg = enums::EnumDeclarationCfg::new(&api.crate_name, &configured_enum_features);
        for en in api.enums.iter().filter(|e| !exclude_types.contains(&e.name)) {
            if result_type_enums.contains(&en.name) {
                enums::emit_enum_without_into_rust(en, &mut body, &mapper, &known_dto_names, &enum_declaration_cfg);
            } else {
                enums::emit_enum(
                    en,
                    &mut body,
                    &mapper,
                    &known_dto_names,
                    &config.untagged_union_text_types,
                    &enum_declaration_cfg,
                );
            }
            body.push('\n');
        }

        for error in &api.errors {
            errors::emit_error(error, &module_name, &mut body, &mapper);
            body.push('\n');
        }

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
                names: &mut std::collections::BTreeSet<&'a str>,
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

            let mut referenced_async_named: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
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

        let binding_crate_name = format!("{}-swift", api.crate_name);
        let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
        let package_root = swift_package_root(&base_dir, config.explicit_output.swift.is_some());
        // `consult_build_output: false` -- reading `target/`'s swift-bridge build output here
        // is exactly the alef #A/#B bug: that directory is populated by this same command's
        // own post-build step (`PostBuildStep::MaterializeSwiftBridge`, wired below in
        // `build_config_with_config`), so whether it exists yet depends on run ordering, not
        // on source input. `MaterializeSwiftBridge` already writes the real trio unguarded
        // once the build succeeds; this call only ever emits the static, build-independent
        // placeholder (or nothing, when a populated header is already committed). ~keep
        if let Some(bridge_files) =
            bridge_artifacts::emit_swift_bridge_files(&api.crate_name, &binding_crate_name, &package_root, false)?
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

        Ok(module_imports::strip_self_module_imports(files))
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        service_api::generate(api, config).map(module_imports::strip_self_module_imports)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        // This literal manifest path is only reachable when no `ResolvedCrateConfig` is
        // available at all (the `Backend::build_config_with_config` default falls back to this
        // method verbatim) -- every real caller in this codebase goes through
        // `build_config_with_config` below, which replaces it with the config-derived path
        // before any command actually runs. Kept as a reasonable single-crate default so this
        // method still returns something sensible standalone. ~keep
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
        let paths = swift_build_paths(config);

        build_config.post_build[0] = PostBuildStep::RunCommand {
            cmd: "cargo",
            args: vec!["build", "--manifest-path", paths.manifest_path, "--release"],
        };

        build_config.post_build.push(PostBuildStep::MaterializeSwiftBridge {
            binding_crate_name: paths.binding_crate_name,
            package_root: paths.package_root,
        });

        Some(build_config)
    }

    /// The post-build config `alef generate`/`alef all` run (`complete_generated_artifacts`,
    /// never `alef build`'s own dispatch).
    ///
    /// `build_config_with_config` above compiles the swift-bridge crate in **release** mode --
    /// correct for `alef build`, which needs a real linkable artifact, but wrong for generation:
    /// generation only needs the `SwiftBridgeCore.swift`/`{crate}.swift`/`RustBridgeC.h` trio
    /// `MaterializeSwiftBridge` copies out of `OUT_DIR`, and that trio is written by this crate's
    /// own `build.rs` (`gen_rust_crate::cargo::emit_build_rs`), which does nothing but run
    /// `swift_bridge_build::parse_bridges` (a `syn` source parse) and is executed identically by
    /// `cargo check` and `cargo build` -- Cargo always runs a package's build script before
    /// checking or building it, regardless of profile. `cargo check` skips the codegen,
    /// optimization, and linking `--release` performs on top of that, which is what turned a
    /// contractually no-build `alef generate` into a multi-minute compile (task #541). Swapping
    /// `build`/`--release` for a bare `check` here, while `build_config_with_config` keeps the
    /// real release build for `alef build`, is the whole fix: same `OUT_DIR` contents, none of
    /// the compilation `alef generate` was never supposed to do. ~keep
    fn generate_post_build_config(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        let mut build_config = self.build_config()?;
        let paths = swift_build_paths(config);

        build_config.post_build[0] = PostBuildStep::RunCommand {
            cmd: "cargo",
            args: vec!["check", "--manifest-path", paths.manifest_path],
        };

        build_config.post_build.push(PostBuildStep::MaterializeSwiftBridge {
            binding_crate_name: paths.binding_crate_name,
            package_root: paths.package_root,
        });

        Some(build_config)
    }

    /// Swift's registration entry points are the top-level `public func` forwarders emitted by
    /// `forwarders::emit_trait_bridge_forwarders`, not the protocol/adapter file — which
    /// deliberately emits no register function. The forwarder loop is the authority for the
    /// gates repeated here: `bind_via = "options_field"` bridges get a handle instead of a
    /// registry, and a bridge with none of the three functions configured emits nothing. ~keep
    fn trait_bridge_registration_surface(
        &self,
        _api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        config
            .trait_bridges
            .iter()
            .filter(|bridge| bridge.bind_via == BridgeBinding::FunctionParam)
            .filter(|bridge| bridge.is_active_for("swift"))
            .filter(|bridge| {
                bridge.register_fn.is_some() || bridge.unregister_fn.is_some() || bridge.clear_fn.is_some()
            })
            .map(|bridge| {
                let forwarder =
                    |configured: &Option<String>| configured.as_deref().map(forwarders::swift_trait_forwarder_name);
                TraitBridgeRegistrationSurface {
                    trait_name: bridge.trait_name.clone(),
                    register_symbol: forwarder(&bridge.register_fn),
                    unregister_symbol: forwarder(&bridge.unregister_fn),
                    clear_symbol: forwarder(&bridge.clear_fn),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod package_root_tests {
    use super::swift_package_root;
    use std::path::Path;

    #[test]
    fn default_layout_treats_base_dir_as_the_package_root() {
        assert_eq!(swift_package_root("packages/swift", false), Path::new("packages/swift"));
    }

    #[test]
    fn multi_crate_layout_treats_base_dir_as_the_package_root() {
        assert_eq!(
            swift_package_root("packages/swift/sample-crate", false),
            Path::new("packages/swift/sample-crate")
        );
    }

    #[test]
    fn explicit_output_walks_up_two_levels_from_the_leaf_module_directory() {
        assert_eq!(
            swift_package_root("packages/swift/Sources/SampleCrate", true),
            Path::new("packages/swift")
        );
    }

    #[test]
    fn explicit_output_shorter_than_two_components_falls_back_to_itself() {
        assert_eq!(swift_package_root("swift", true), Path::new("swift"));
    }

    #[test]
    fn same_config_never_depends_on_the_flag_it_was_not_given() {
        // Regression guard for the bug this replaces: the old ancestor-probing logic could
        // silently collapse to an empty path for a short `base_dir` under the explicit branch.
        // `unwrap_or(base_path)` here must return the ORIGINAL base_dir, never an empty one.
        let result = swift_package_root("swift", true);
        assert!(
            !result.as_os_str().is_empty(),
            "package root must never be empty: {result:?}"
        );
    }
}

#[cfg(test)]
mod build_config_manifest_path_tests {
    use super::SwiftBackend;
    use crate::core::backend::{Backend, PostBuildStep};
    use crate::core::config::NewAlefConfig;

    /// alef #169: `build_config()`'s `--manifest-path` used to be the literal
    /// `packages/swift/rust/Cargo.toml`, independent of `config.output_paths` -- wrong for any
    /// consumer that sets an explicit `[crates.output] swift` path, since `gen_rust_crate::emit`
    /// (see its own regression test) writes the crate `cargo build` needs to build under the
    /// SAME config-derived `swift_package_root`, not under the fixed default location. This
    /// proves `build_config_with_config`'s manifest path follows an explicit override rather
    /// than staying pinned to the default.
    #[test]
    fn manifest_path_follows_an_explicit_swift_output_override() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]

[crates.output]
swift = "custom/output/Sources/TestLib"
"#,
        )
        .expect("test config must parse");
        let config = cfg.resolve().expect("test config must resolve").remove(0);

        let build_config = SwiftBackend
            .build_config_with_config(&config)
            .expect("swift backend must produce a build config");

        // The manifest path is built with `Path::join`, so its separators are the host's --
        // `\` on Windows, which is what `cargo --manifest-path` wants there. Comparing the
        // raw string would assert the host OS, not the override-following behaviour under
        // test, so separators are normalised at the assertion boundary. ~keep
        let manifest_path_args: Vec<String> = build_config
            .post_build
            .iter()
            .find_map(|step| match step {
                PostBuildStep::RunCommand { cmd, args } if *cmd == "cargo" => {
                    Some(args.iter().map(|arg| arg.replace('\\', "/")).collect())
                }
                _ => None,
            })
            .expect("swift build config must have a cargo RunCommand post-build step");

        assert_eq!(
            manifest_path_args,
            vec!["build", "--manifest-path", "custom/output/rust/Cargo.toml", "--release"],
            "manifest path must follow the explicit [crates.output] swift override, not stay \
             pinned to the packages/swift/rust default; got: {manifest_path_args:?}"
        );
    }

    /// Positive control for task #541: `alef build`'s post-build config (`build_config_with_config`,
    /// the config `cli::pipeline::commands::build`'s own dispatch loop runs) must still perform a
    /// real `cargo build --release` of the swift-bridge crate. If this regresses to something
    /// cheaper, `alef build` would ship a Swift package that was never actually compiled.
    #[test]
    fn build_config_with_config_still_compiles_for_the_real_build_path() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .expect("test config must parse");
        let config = cfg.resolve().expect("test config must resolve").remove(0);

        let build_config = SwiftBackend
            .build_config_with_config(&config)
            .expect("swift backend must produce a build config");

        let cargo_args: Vec<&str> = build_config
            .post_build
            .iter()
            .find_map(|step| match step {
                PostBuildStep::RunCommand { cmd, args } if *cmd == "cargo" => Some(args.clone()),
                _ => None,
            })
            .expect("alef build's swift config must have a cargo RunCommand post-build step");

        assert!(
            cargo_args.contains(&"build") && cargo_args.contains(&"--release"),
            "alef build must still compile the swift-bridge crate in release mode: {cargo_args:?}"
        );
    }

    /// THE FIX for task #541: `alef generate`/`alef all` are contractually no-build steps, but
    /// Swift's post-build used to invoke the exact same `cargo build --release` as `alef build`
    /// itself -- a cold release compile the consumer measured at 12m37s inside what was supposed
    /// to be a cheap generate-only step. `generate_post_build_config` is the config
    /// `bin_cli::helpers::post_build::run_required_post_builds` (the generate-only caller) now
    /// resolves instead of `build_config_with_config`. This asserts its cargo invocation is a
    /// bare `check` -- no `build`, no `--release` -- so the underlying crate's build script (the
    /// only thing that actually needs to run to materialize the swift-bridge Swift/header trio)
    /// executes without paying for codegen, optimization, or linking.
    #[test]
    fn generate_post_build_config_only_checks_never_compiles() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .expect("test config must parse");
        let config = cfg.resolve().expect("test config must resolve").remove(0);

        let build_config = SwiftBackend
            .generate_post_build_config(&config)
            .expect("swift backend must produce a generate-time post-build config");

        let cargo_args: Vec<&str> = build_config
            .post_build
            .iter()
            .find_map(|step| match step {
                PostBuildStep::RunCommand { cmd, args } if *cmd == "cargo" => Some(args.clone()),
                _ => None,
            })
            .expect("alef generate's swift config must have a cargo RunCommand post-build step");

        assert_eq!(
            cargo_args,
            vec!["check", "--manifest-path", "packages/swift/rust/Cargo.toml"],
            "alef generate must only `cargo check` the swift-bridge crate, never `build`/`--release`, \
             or it re-introduces the no-build contract violation task #541 fixed: {cargo_args:?}"
        );

        // The materialization step must still run after the (now cheap) cargo invocation --
        // otherwise generation would never pick up fresh OUT_DIR content at all.
        assert!(
            build_config
                .post_build
                .iter()
                .any(|step| matches!(step, PostBuildStep::MaterializeSwiftBridge { .. })),
            "generate_post_build_config must still re-materialize the swift-bridge trio after \
             the cargo check step: {:?}",
            build_config.post_build
        );
    }
}
