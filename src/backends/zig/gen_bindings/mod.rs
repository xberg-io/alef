use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, EmittedSignature, GeneratedFile};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use std::path::PathBuf;

use crate::backends::zig::trait_bridge::emit_trait_bridge;

mod errors;
mod functions;
mod helpers;
mod opaque_handles;
mod result_presence;
mod service_api;
mod types;

use errors::emit_error_set;
use functions::emit_function;
use helpers::emit_helpers;
use opaque_handles::{emit_opaque_constructor, emit_opaque_handle};
use types::{emit_enum, emit_type};

fn zig_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

pub struct ZigBackend;

fn type_references_excluded(ty: &TypeRef, exclude_types: &std::collections::HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn signature_references_excluded(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &std::collections::HashSet<String>,
) -> bool {
    type_references_excluded(return_type, exclude_types)
        || params
            .iter()
            .any(|param| type_references_excluded(&param.ty, exclude_types))
}

/// Names of types the Zig wrapper renders as a serde struct (JSON-encoded across the
/// wrapper boundary). Shared by `generate_bindings` and `public_function_signatures` so
/// both agree on which `Named` types the emitted signature treats as opaque-JSON. ~keep
///
/// Takes a slice rather than an `ApiSurface` so the Zig e2e generator can ask the same
/// question: it only ever holds `type_defs`, and when it answered independently the two
/// disagreed for any plain function returning a serde struct — the generator emitted
/// `result.<field>` against what the backend had already decided was a `[]u8`. One
/// predicate, one answer. ~keep
pub(crate) fn zig_struct_names(type_defs: &[crate::core::ir::TypeDef]) -> std::collections::HashSet<String> {
    type_defs
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde)
        .map(|t| t.name.clone())
        .collect()
}

/// Maps an opaque type name to `(creator function name, creator's config param name in
/// snake_case)`, used to render a parameter/return of that type as the JSON-string wrapper
/// boundary rather than a raw handle. Shared by `generate_bindings` and
/// `public_function_signatures` for the same reason as `zig_struct_names`. ~keep
pub(crate) fn zig_opaque_creator_map(api: &ApiSurface) -> std::collections::HashMap<String, (String, String)> {
    let mut map = std::collections::HashMap::new();
    for opaque_ty in api
        .types
        .iter()
        .filter(|t| !t.is_trait && (t.is_opaque || !t.has_serde))
    {
        if let Some(creator) = api
            .functions
            .iter()
            .find(|f| matches!(&f.return_type, crate::core::ir::TypeRef::Named(n) if n == &opaque_ty.name))
            && let Some(config_param) = creator.params.first()
            && let Some(config_name) = functions::opaque_type_name_inner(&config_param.ty)
        {
            map.insert(
                opaque_ty.name.clone(),
                (creator.name.clone(), heck::AsSnakeCase(config_name).to_string()),
            );
        }
    }
    map
}

/// The Zig wrapper-boundary type for a function/method parameter of type `ty` -- delegates to
/// the same `functions::zig_param_type` the emitter itself calls to decide whether a `Named`
/// DTO crosses as its struct type or as JSON-encoded `[]const u8`, so the docs layer can never
/// independently re-derive a shape the backend does not actually emit. See `zig_struct_names`'s
/// doc comment for why that predicate, not a docs-local guess, is the single source of truth. ~keep
pub(crate) fn zig_boundary_param_type(ty: &TypeRef, optional: bool, api: &ApiSurface) -> String {
    let struct_names = zig_struct_names(&api.types);
    let opaque_creator_map = zig_opaque_creator_map(api);
    functions::zig_param_type(ty, optional, &struct_names, &opaque_creator_map)
}

/// The Zig wrapper-boundary type for a function/method return of type `ty` -- see
/// `zig_boundary_param_type`'s doc comment. ~keep
pub(crate) fn zig_boundary_return_type(ty: &TypeRef, api: &ApiSurface) -> String {
    let struct_names = zig_struct_names(&api.types);
    functions::zig_return_type(ty, &struct_names)
}

/// Function names `generate_bindings`'s top-level loop skips because a trait bridge emits
/// them itself (see `emit_trait_bridge`) instead of the ordinary `emit_function` path.
/// Shared with `public_function_signatures` for the same reason as `zig_struct_names`. ~keep
fn zig_trait_bridge_fn_names(api: &ApiSurface, config: &ResolvedCrateConfig) -> std::collections::HashSet<String> {
    config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|lang| lang == "zig"))
        .flat_map(|b| {
            let mut names = Vec::new();
            if let Some(trait_def) = api.types.iter().find(|t| t.name == b.trait_name && t.is_trait) {
                let snake = heck::AsSnakeCase(&trait_def.name).to_string();
                names.push(format!("register_{snake}"));
                names.push(format!("unregister_{snake}"));
            }
            if let Some(clear_fn) = b.clear_fn.as_deref() {
                names.push(clear_fn.to_string());
            }
            names
        })
        .collect()
}

impl Backend for ZigBackend {
    fn name(&self) -> &str {
        "zig"
    }

    fn language(&self) -> Language {
        Language::Zig
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: false,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        if let Some(zig) = config.zig.as_ref() {
            crate::core::config::languages::require_shared_native_runtime(
                &zig.capsule_types,
                zig.shares_native_runtime,
                "zig",
            )?;
        }
        let api = api.with_deduped_functions();
        crate::codegen::cfg::warn_on_ffi_feature_drift(&api, config, Language::Zig);
        let zig_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Zig);
        let enabled_features: std::collections::HashSet<&str> = zig_features.iter().map(String::as_str).collect();
        // `@cImport` compiles the C header verbatim and Zig resolves declared externs at
        // comptime/link time — same failure mode as Go's cgo: a function/type/enum the FFI
        // library dropped under `#[cfg(feature = "X")]` is a build-time error, not a graceful
        // fallback. Filtering to what the configured Zig feature set actually satisfies (and
        // dropping cfg-gated fields/variants on surviving types) keeps the generated module
        // consistent with the linked native library, mirroring `with_cfg_filtered_deep`'s
        // existing use in Swift, Kotlin Android, and JNI. ~keep
        let api = api.with_cfg_filtered_deep(&enabled_features);
        let module_name = zig_module_name(&config.name);
        let header = config.ffi_header_name();
        let prefix = config.ffi_prefix();

        let mut exclude_functions: std::collections::HashSet<String> = config
            .zig
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let mut exclude_types: std::collections::HashSet<String> = config
            .ffi
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(zig) = &config.zig {
            exclude_types.extend(zig.exclude_types.iter().cloned());
        }
        if let Some(ffi) = &config.ffi {
            exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        }
        exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        exclude_types.extend(
            config
                .opaque_types
                .iter()
                .filter(|(_, path)| path.contains('<'))
                .map(|(name, _)| name.clone()),
        );

        let type_is_visible = |name: &str| !exclude_types.contains(name);
        let method_is_visible = |method: &crate::core::ir::MethodDef| {
            !signature_references_excluded(&method.params, &method.return_type, &exclude_types)
        };

        let api = if exclude_types.is_empty() {
            api
        } else {
            let mut filtered = api.clone();
            filtered.types.retain(|typ| type_is_visible(&typ.name));
            for typ in &mut filtered.types {
                typ.fields
                    .retain(|field| !type_references_excluded(&field.ty, &exclude_types));
                typ.methods.retain(method_is_visible);
            }
            filtered.enums.retain(|en| !exclude_types.contains(&en.name));
            filtered
                .functions
                .retain(|func| !signature_references_excluded(&func.params, &func.return_type, &exclude_types));
            filtered
        };

        let zig_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
            config.zig.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();
        // Matches both bare `Named(name)` and `Optional(Named(name))` returns — capsule
        // returns share one raw C ABI regardless of IR optionality, see
        // `functions::opaque_type_name_inner` and `emit_function`'s capsule guard. ~keep
        let uses_capsule = !zig_capsule_types.is_empty()
            && api.functions.iter().any(|f| {
                functions::opaque_type_name_inner(&f.return_type).is_some_and(|n| zig_capsule_types.contains_key(n))
            });

        let mut content = String::new();
        content.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
        content.push('\n');
        content.push('\n');
        content.push_str("const std = @import(\"std\");\n");
        if uses_capsule {
            let capsule_import_names: std::collections::BTreeSet<&str> = api
                .functions
                .iter()
                .filter_map(|f| {
                    functions::opaque_type_name_inner(&f.return_type).and_then(|n| zig_capsule_types.get(n))
                })
                .filter_map(|cap| crate::core::config::languages::zig_capsule_import_name(&cap.host_type))
                .collect();
            for import_name in capsule_import_names {
                content.push_str(&format!("const {import_name} = @import(\"{import_name}\");\n"));
            }
        }
        content.push_str(&crate::backends::zig::template_env::render(
            "c_import.jinja",
            minijinja::context! {
                header => header,
            },
        ));
        content.push('\n');

        emit_helpers(&prefix, &api.errors, &mut content);
        content.push('\n');

        for bridge in &config.trait_bridges {
            if bridge.exclude_languages.iter().any(|lang| lang == "zig") {
                continue;
            }
            if let Some(alias) = &bridge.type_alias {
                content.push_str(&crate::backends::zig::template_env::render(
                    "trait_bridge_alias.jinja",
                    minijinja::context! {
                        alias => alias,
                    },
                ));
                content.push('\n');
            }
        }

        for error in &api.errors {
            emit_error_set(error, &mut content);
            content.push('\n');
        }

        for ty in api
            .types
            .iter()
            .filter(|t| !exclude_types.contains(&t.name) && !t.is_opaque && t.has_serde)
        {
            emit_type(ty, config, &mut content);
            content.push('\n');
        }

        for en in api.enums.iter().filter(|e| !exclude_types.contains(&e.name)) {
            emit_enum(en, &mut content);
            content.push('\n');
        }

        let declared_errors: Vec<String> = api.errors.iter().map(|e| e.name.clone()).collect();
        let mut top_level_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for f in &api.functions {
            top_level_names.insert(f.name.clone());
        }
        for ty in &api.types {
            top_level_names.insert(ty.name.clone());
        }
        for en in &api.enums {
            top_level_names.insert(en.name.clone());
        }
        let struct_names = zig_struct_names(&api.types);
        let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
        let opaque_creator_map = zig_opaque_creator_map(&api);

        let trait_bridge_fn_names = zig_trait_bridge_fn_names(&api, config);
        for f in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
            if trait_bridge_fn_names.contains(&f.name) {
                continue;
            }
            emit_function(
                f,
                &prefix,
                &declared_errors,
                &top_level_names,
                &struct_names,
                &opaque_creator_map,
                &zig_capsule_types,
                &mut content,
            );
            content.push('\n');
        }

        let error_type = config.error_type.as_deref().unwrap_or("error");
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.iter().any(|lang| lang == "zig") {
                continue;
            }
            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name && t.is_trait) {
                emit_trait_bridge(&prefix, error_type, bridge_cfg, trait_def, &exclude_types, &mut content);
                content.push('\n');
            } else {
                // Refused rather than skipped: without the trait definition the `make_<trait>_vtable`
                // builder is never emitted, and every generated e2e reference to it fails to
                // compile. ~keep
                return Err(anyhow::anyhow!(
                    "zig backend: trait bridge '{}' has no trait definition in the binding surface. \
                     Check that the trait is not in `exclude_types` or marked `binding_excluded`.",
                    bridge_cfg.trait_name,
                ));
            }
        }

        let streaming_item_types: std::collections::HashMap<String, String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .filter_map(|a| a.item_type.as_ref().map(|item| (a.name.clone(), item.clone())))
            .collect();

        let trait_bridge_type_aliases: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|lang| lang == "zig"))
            .filter_map(|b| b.type_alias.clone())
            .collect();

        for ty in api
            .types
            .iter()
            .filter(|t| !t.is_trait && (t.is_opaque || !t.has_serde))
            .filter(|t| !exclude_types.contains(&t.name))
            .filter(|t| !trait_bridge_type_aliases.contains(&t.name))
        {
            emit_opaque_handle(
                ty,
                &prefix,
                &declared_errors,
                &struct_names,
                &streaming_item_types,
                &enum_names,
                &mut content,
            );
            content.push('\n');
            if let Some(ctor) = config.client_constructors.get(&ty.name) {
                emit_opaque_constructor(ty, &prefix, ctor, &top_level_names, &mut content);
                content.push('\n');
            }
        }

        let dir = resolve_output_dir(None, &config.name, "packages/zig/src");
        let path = PathBuf::from(dir).join(format!("{module_name}.zig"));

        // The emitted module hand-declares the opaque handle as `_handle: u64` (see
        // `opaque_handle_header.jinja` / `opaque_stream_struct.jinja`) rather than reading the
        // width back from the `@cImport`ed header, so a pointer-vs-`uint64_t` straddle against
        // the FFI crate still compiles and links — `@cImport` only has to agree that the extern
        // exists — and misbehaves at runtime. Stamping lets `find_stamp_disagreement` fail
        // `alef verify` when one tree holds a zig binding and an FFI artifact from two handle
        // generations, which is the exact pair that gate was built for. Injected here, inside
        // the backend body, so the marker is part of the content `finalize_hashes` hashes
        // afterwards; `inject_stamp_line` claims the marker-adjacent slot, so stamping after
        // `inject_hash_line` would displace the hash line and hide it from verify. ~keep
        let content = crate::core::hash::inject_stamp_line(
            &content,
            crate::core::hash::HANDLE_ABI_STAMP_KEY,
            crate::core::template_versions::abi::HANDLE_ABI_VERSION,
        );

        Ok(vec![GeneratedFile {
            path,
            content,
            generated_header: false,
        }])
    }

    /// Signatures for the breaking-signature-change baseline (see
    /// `cli::breaking_changes`). Mirrors `generate_bindings`'s filtering closely enough to
    /// stay accurate for the common case, but is deliberately not identical: it does not
    /// apply `exclude_types`'s deep retain over `api.functions`/`api.types`/`api.enums`, so
    /// a function hidden from Zig output entirely via that config still appears here. That
    /// is harmless rather than a false negative -- a function alef never actually emits has
    /// no real caller to attribute, so `check_signature_breakage`'s "callers must exist"
    /// gate suppresses it. Capsule-return functions (`emit_capsule_function`'s path) are
    /// excluded outright rather than approximated, since that path's return-type rule
    /// differs from the ordinary wrapper's. ~keep
    fn public_function_signatures(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<EmittedSignature> {
        let api = api.with_deduped_functions();
        let zig_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Zig);
        let enabled_features: std::collections::HashSet<&str> = zig_features.iter().map(String::as_str).collect();
        let api = api.with_cfg_filtered_deep(&enabled_features);

        let declared_errors: Vec<String> = api.errors.iter().map(|e| e.name.clone()).collect();
        let struct_names = zig_struct_names(&api.types);
        let opaque_creator_map = zig_opaque_creator_map(&api);
        let trait_bridge_fn_names = zig_trait_bridge_fn_names(&api, config);
        let zig_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
            config.zig.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();
        let mut exclude_functions: std::collections::HashSet<String> = config
            .zig
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(ffi) = &config.ffi {
            exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        }

        api.functions
            .iter()
            .filter(|f| !exclude_functions.contains(&f.name))
            .filter(|f| !trait_bridge_fn_names.contains(&f.name))
            .filter(|f| {
                functions::opaque_type_name_inner(&f.return_type)
                    .is_none_or(|name| !zig_capsule_types.contains_key(name))
            })
            .map(|f| EmittedSignature {
                symbol: f.name.clone(),
                params: functions::wrapper_param_types(f, &struct_names, &opaque_creator_map).join(", "),
                return_type: functions::wrapper_return_type(f, &declared_errors, &struct_names, &opaque_creator_map),
            })
            .collect()
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
        let zig_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Zig);
        let enabled_features: std::collections::HashSet<&str> = zig_features.iter().map(String::as_str).collect();
        let filtered_api = api.with_cfg_filtered_deep(&enabled_features);
        service_api::generate(&filtered_api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "zig",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }

    /// Zig derives its `register_{trait_snake}`/`unregister_{trait_snake}` names from the
    /// trait name itself (see `zig_trait_bridge_fn_names`) rather than honoring a configured
    /// `register_fn`/`unregister_fn` — unlike backends (e.g. PyO3) whose registration symbol
    /// is the configured name verbatim. Both names, along with `clear_fn`, are only emitted
    /// when `bind_via = "function_param"` (see `emit_trait_bridge`); the `options_field` mode
    /// emits a handle constructor instead, so it reports no registration surface here. ~keep
    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<crate::core::backend::TraitBridgeRegistrationSurface> {
        config
            .trait_bridges
            .iter()
            .filter(|bridge| bridge.is_active_for("zig"))
            .filter(|bridge| matches!(bridge.bind_via, crate::core::config::BridgeBinding::FunctionParam))
            .filter_map(|bridge| {
                let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name && t.is_trait)?;
                let snake = heck::AsSnakeCase(&trait_def.name).to_string();
                Some(crate::core::backend::TraitBridgeRegistrationSurface {
                    trait_name: trait_def.name.clone(),
                    register_symbol: Some(format!("register_{snake}")),
                    unregister_symbol: Some(format!("unregister_{snake}")),
                    clear_symbol: bridge.clear_fn.clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod handle_abi_stamp_tests {
    use super::*;
    use crate::core::ir::TypeDef;

    fn zig_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// An opaque type routes through `emit_opaque_handle` (see the `is_opaque || !has_serde`
    /// filter in `generate_bindings`) and is what makes the module hand-declare a handle.
    fn opaque_api() -> ApiSurface {
        ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "Session".to_string(),
                rust_path: "demo::Session".to_string(),
                is_opaque: true,
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    /// The zig module hand-declares the opaque handle's width instead of reading it back from
    /// the `@cImport`ed header, so a pointer-vs-`uint64_t` straddle against the FFI crate links
    /// cleanly and only misbehaves at runtime — the exact pair `find_stamp_disagreement` exists
    /// to catch. Asserting the emitted `_handle: u64` *first* is what keeps this from passing
    /// vacuously over an output that never declared a handle at all; only then is the stamp
    /// meaningful. ~keep
    #[test]
    fn emitted_zig_module_stamps_the_handle_abi_it_hand_declares() {
        let files = ZigBackend
            .generate_bindings(&opaque_api(), &zig_config())
            .expect("zig backend must generate bindings");
        let content = &files[0].content;

        assert!(
            content.contains("pub const Session = struct {"),
            "fixture must actually emit the opaque handle struct: {content}"
        );
        assert!(
            content.contains("_handle: u64,"),
            "fixture must actually emit the hand-declared handle this stamp guards: {content}"
        );

        crate::backends::ffi::handle_abi_stamp::assert_stamped_before_hashing(content, "zig binding module");
    }

    /// The stamp is injected relative to the module's own `// Generated by alef` marker, which
    /// `inject_stamp_line` requires within its scan window — if that header ever moved or
    /// changed wording the injection would silently no-op and leave the file unstamped.
    #[test]
    fn stamp_lands_directly_beneath_the_generated_header_marker() {
        let files = ZigBackend
            .generate_bindings(&opaque_api(), &zig_config())
            .expect("zig backend must generate bindings");
        let mut lines = files[0].content.lines();

        let expected_stamp = format!(
            "// alef:{}:{}",
            crate::core::hash::HANDLE_ABI_STAMP_KEY,
            crate::core::template_versions::abi::HANDLE_ABI_VERSION
        );

        assert_eq!(lines.next(), Some(crate::core::hash::SELF_MARKING_HEADER_LINE));
        assert_eq!(lines.next(), Some(expected_stamp.as_str()));
    }
}

#[cfg(test)]
mod public_function_signatures_tests {
    use super::*;
    use crate::core::backend::EmittedSignature;
    use crate::core::ir::FunctionDef;

    fn zig_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn api_with_function(f: FunctionDef, errors: Vec<crate::core::ir::ErrorDef>) -> ApiSurface {
        ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            functions: vec![f],
            errors,
            ..ApiSurface::default()
        }
    }

    fn my_error_def() -> crate::core::ir::ErrorDef {
        crate::core::ir::ErrorDef {
            name: "MyError".to_string(),
            rust_path: "sample::MyError".to_string(),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant {
                error_code: None,
                name: "Boom".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// The motivating regression this whole mechanism exists for: a function that returns a
    /// plain type in one run and gains a declared `error_type` (an error union, Zig's `!T`)
    /// in the next -- exactly the shape of change a hand-written Zig test would not survive
    /// uncompiled, and the case reported against the real bug this module fixes. ~keep
    #[test]
    fn gaining_a_declared_error_type_changes_the_captured_return_signature() {
        let config = zig_config();
        let plain_fn = FunctionDef {
            name: "do_thing".to_string(),
            return_type: crate::core::ir::TypeRef::Unit,
            ..Default::default()
        };
        let fallible_fn = FunctionDef {
            error_type: Some("MyError".to_string()),
            ..plain_fn.clone()
        };

        let before = ZigBackend.public_function_signatures(&api_with_function(plain_fn, vec![]), &config);
        let after =
            ZigBackend.public_function_signatures(&api_with_function(fallible_fn, vec![my_error_def()]), &config);

        assert_eq!(
            before,
            vec![EmittedSignature {
                symbol: "do_thing".to_string(),
                params: String::new(),
                return_type: "void".to_string(),
            }]
        );
        assert_eq!(
            after,
            vec![EmittedSignature {
                symbol: "do_thing".to_string(),
                params: String::new(),
                return_type: "MyError!void".to_string(),
            }]
        );

        let changes = crate::cli::breaking_changes::detect_breaking_changes(&before, &after);
        assert_eq!(
            changes.len(),
            1,
            "the error-union transition must be detected as breaking: {changes:?}"
        );
        assert_eq!(changes[0].symbol, "do_thing");
    }
}
