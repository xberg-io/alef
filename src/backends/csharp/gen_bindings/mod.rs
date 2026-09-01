use crate::codegen::naming::{
    csharp_type_name, csharp_wrapper_class_name, field_uses_duration_map_wire, to_csharp_name,
};
use crate::codegen::shared::binding_fields;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, FieldDef, TypeRef};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use files::{
    csharp_file_header, gen_directory_build_props, report_unemitted_visitor_files, stale_visitor_filenames,
    strip_trailing_whitespace, superseded_visitor_filenames,
};
use marshalling::{
    CAPSULE_PINVOKE_RETURN_TYPE, HANDLE_PINVOKE_TYPE, bytes_len_arg, emit_named_param_setup, emit_named_param_teardown,
    emit_named_param_teardown_indented, enum_names_with_data_variants, is_bridge_param, is_capsule_return,
    native_call_arg, native_call_args, needs_param_teardown, returns_bool_via_int, returns_json_object, returns_ptr,
    returns_string, zero_sentinel, zero_sentinel_for_pinvoke_type,
};

/// Metadata for a streaming adapter, used to drive emission of an
/// `IAsyncEnumerable<Item>` method over the FFI iterator-handle protocol
/// (`_start` / `_next` / `_free`).
#[derive(Debug, Clone)]
pub(super) struct StreamingMethodMeta {
    /// Owner type (e.g. `DefaultClient`). Retained for future routing decisions even when the
    /// current emitter derives the receiver type from the enclosing class.
    #[allow(dead_code)]
    pub owner_type: String,
    pub item_type: String,
}

#[cfg(test)]
mod abi_parity_tests;
pub(super) mod enums;
pub(super) mod errors;
// Re-exported at crate visibility (narrower than the module itself) so
// `e2e::codegen::declared_error_variant::substantiates_variant_identity` can ask the exact same
// question the generator's own dispatch-line builder asks — see that function's `~keep` doc.
pub(crate) use errors::variant_dispatch_prefix;
mod files;
pub(super) mod functions;
pub(super) mod marshalling;
pub(super) mod methods;
mod pinvoke;
mod result_presence;
pub(super) mod service_api;
pub(crate) mod types;

/// Sanitise a rustdoc string for safe embedding in C# XML doc comments.
///
/// Wraps [`crate::codegen::doc_emission::sanitize_rust_idioms`] with the
/// [`crate::codegen::doc_emission::DocTarget::CSharpDoc`] target so every C#
/// backend doc-emission site (templates that take `doc_lines`, helpers that
/// emit `/// <summary>` blocks directly) routes through the same pipeline.
pub(crate) fn sanitize_rust_syntax_for_csharp(doc: &str) -> String {
    crate::codegen::doc_emission::sanitize_rust_idioms(doc, crate::codegen::doc_emission::DocTarget::CSharpDoc)
}

/// Sanitise a rustdoc string and split it into lines for `doc_lines` template variables.
///
/// Returns an empty `Vec` when the sanitised doc is empty. The companion
/// `has_doc` flag should be set to `!doc_lines.is_empty()` rather than checking
/// the raw input, because sanitisation may drop the entire body (e.g. a doc
/// that is nothing but a rust code-fence example).
pub(crate) fn sanitize_doc_lines_for_csharp(doc: &str) -> Vec<String> {
    if doc.is_empty() {
        return Vec::new();
    }
    let sanitized = sanitize_rust_syntax_for_csharp(doc);
    if sanitized.trim().is_empty() {
        return Vec::new();
    }
    sanitized.lines().map(ToString::to_string).collect()
}

pub struct CsharpBackend;

impl CsharpBackend {}

fn effective_exclude_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(csharp) = &config.csharp {
        exclude_types.extend(csharp.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    exclude_types
}

fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

fn api_without_excluded_types(api: &ApiSurface, exclude_types: &HashSet<String>) -> ApiSurface {
    let mut filtered = api.clone();
    filtered.types.retain(|typ| !exclude_types.contains(&typ.name));
    for typ in &mut filtered.types {
        typ.fields
            .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        // Trait methods are exempt: each one owns a positional slot in the C vtable the
        // FFI crate declares from the unfiltered surface, and `csharp_type_visible` already
        // degrades an excluded type in a bridge signature to a JSON `string`. Dropping the
        // method here would leave the bridge class allocating and writing N-1 function
        // pointers into a struct Rust reads as N slots wide. ~keep
        if !typ.is_trait {
            typ.methods.retain(|method| {
                !signature_references_excluded_type(&method.params, &method.return_type, exclude_types)
            });
        }
    }
    filtered
        .enums
        .retain(|enum_def| !exclude_types.contains(&enum_def.name));
    for enum_def in &mut filtered.enums {
        for variant in &mut enum_def.variants {
            variant
                .fields
                .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        }
    }
    filtered
        .functions
        .retain(|func| !signature_references_excluded_type(&func.params, &func.return_type, exclude_types));
    filtered.errors.retain(|error| !exclude_types.contains(&error.name));
    filtered
}

/// Fail generation when the C# bridge's vtable does not slot-for-slot match the Rust
/// vtable struct the FFI crate declares for the same trait.
///
/// The two sides are derived independently: `emitted_slot_names` comes from the
/// `Marshal.WriteIntPtr` calls the bridge class actually writes (built from the
/// C#-filtered surface), while the expected list comes from `source_api` — the same
/// unfiltered surface the FFI backend reads. Any C#-side filtering that drops, adds, or
/// reorders a trait method therefore shows up here.
///
/// This is checked at generation time rather than emitted as a runtime guard because both
/// sides are knowable now and a consumer cannot skip it. Nothing on the Rust side can catch
/// it later: the bridge class writes into a `Marshal.AllocHGlobal` block sized from the same
/// (wrong) count, so an omitted slot is not null — it holds the next field's valid pointer
/// shifted one word left, and the final read runs past the allocation. Every slot is written
/// at a fixed byte offset, which is why a reorder is as fatal as an omission. ~keep
fn assert_vtable_matches_rust_struct(
    source_api: &ApiSurface,
    trait_def: &crate::core::ir::TypeDef,
    has_super_trait: bool,
    ffi_skip_methods: &[String],
    emitted_slot_names: &[String],
) -> anyhow::Result<()> {
    let source_trait_def = source_api
        .types
        .iter()
        .find(|typ| typ.name == trait_def.name && typ.is_trait)
        .unwrap_or(trait_def);
    let expected = crate::codegen::generators::trait_bridge::vtable_slot_names(
        source_trait_def,
        has_super_trait,
        ffi_skip_methods,
    );
    if emitted_slot_names == expected.as_slice() {
        return Ok(());
    }
    anyhow::bail!(
        "C# trait bridge for `{}` emits a vtable that does not match the Rust vtable struct.\n\
         Rust slots ({}): {}\n\
         C# slots ({}): {}",
        trait_def.name,
        expected.len(),
        expected.join(", "),
        emitted_slot_names.len(),
        emitted_slot_names.join(", "),
    )
}

impl Backend for CsharpBackend {
    fn name(&self) -> &str {
        "csharp"
    }

    fn language(&self) -> Language {
        Language::Csharp
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // The surface as the FFI backend sees it, kept alongside the C#-filtered one so the
        // trait-bridge block can prove C#'s vtable matches the Rust vtable struct. ~keep
        let source_api = &crate::backends::ir_order::with_sorted_items(api);
        let exclude_types = effective_exclude_types(source_api, config);
        let filtered_api;
        let api = if exclude_types.is_empty() {
            source_api
        } else {
            filtered_api = api_without_excluded_types(source_api, &exclude_types);
            &filtered_api
        };
        let deduped_api = api.with_deduped_functions();
        crate::codegen::cfg::warn_on_ffi_feature_drift(api, config, Language::Csharp);
        let csharp_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Csharp);
        let enabled_features: HashSet<&str> = csharp_features.iter().map(String::as_str).collect();
        // `DllImport` resolves lazily (only when the P/Invoke stub is first called), so an
        // unconditionally-declared method for a symbol the FFI library dropped under
        // `#[cfg(feature = "X")]` compiles cleanly and only throws `EntryPointNotFoundException`
        // at runtime — the failure mode that motivated this filter in the first place. Dropping
        // the function/type/enum (and cfg-gated fields/variants) up front keeps NativeMethods.cs
        // and the wrapper class consistent with what the configured C# feature set actually
        // compiles into the native library. Mirrors `with_cfg_filtered_deep`'s existing use in
        // Swift, Kotlin Android, and JNI. ~keep
        let cfg_filtered_api = deduped_api.with_cfg_filtered_deep(&enabled_features);
        let api = &cfg_filtered_api;
        let namespace = config.csharp_namespace();
        let prefix = config.ffi_prefix();
        let lib_name = config.ffi_lib_name();

        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let has_visitor_callbacks = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false);
        let bridge_associated_types = config.bridge_associated_types();

        let streaming_methods: HashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .map(|a| a.name.clone())
            .collect();
        let streaming_methods_meta: HashMap<String, StreamingMethodMeta> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .filter_map(|a| {
                let owner_type = a.owner_type.clone()?;
                let item_type = a.item_type.clone()?;
                Some((a.name.clone(), StreamingMethodMeta { owner_type, item_type }))
            })
            .collect();

        let mut exclude_functions: HashSet<String> = config
            .csharp
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(ffi) = &config.ffi {
            exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        }

        let output_dir = resolve_output_dir(config.output_paths.get("csharp"), &config.name, "packages/csharp/");

        let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

        let mut files = Vec::new();
        // ~keep Stays empty when this backend emits no visitor surface at all, which is the
        // correct candidate set for the deferred report below.
        let mut stale_candidates: Vec<String> = Vec::new();

        let exception_class_name = format!("{}Exception", to_csharp_name(&api.crate_name));

        let capsule_types = config
            .csharp
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();

        files.push(GeneratedFile {
            path: base_path.join("NativeMethods.cs"),
            content: strip_trailing_whitespace(&functions::gen_native_methods(
                api,
                &namespace,
                &lib_name,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_callbacks,
                &config.trait_bridges,
                &streaming_methods,
                &streaming_methods_meta,
                &exclude_functions,
                &config.client_constructors,
                &config.adapters,
                &capsule_types,
            )?),
            generated_header: true,
        });

        if !api.errors.is_empty() {
            let mut seen_exception_files: HashSet<String> = HashSet::new();
            for error in &api.errors {
                let error_files =
                    crate::codegen::error_gen::gen_csharp_error_types(error, &namespace, Some(&exception_class_name));
                for (class_name, content) in error_files {
                    if !seen_exception_files.insert(class_name.clone()) {
                        continue;
                    }
                    files.push(GeneratedFile {
                        path: base_path.join(format!("{}.cs", class_name)),
                        content: strip_trailing_whitespace(&content),
                        generated_header: false,
                    });
                }
            }
        }

        if api.errors.is_empty()
            || !api
                .errors
                .iter()
                .any(|e| format!("{}Exception", e.name) == exception_class_name)
        {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", exception_class_name)),
                content: strip_trailing_whitespace(&errors::gen_exception_class(
                    &namespace,
                    &exception_class_name,
                    &api.errors,
                )),
                generated_header: true,
            });
        }

        let all_opaque_type_names: HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| csharp_type_name(&t.name))
            .collect();

        let wrapper_class_name = csharp_wrapper_class_name(&api.crate_name, &namespace);
        crate::core::config::languages::require_shared_native_runtime(
            &capsule_types,
            config
                .csharp
                .as_ref()
                .is_some_and(|csharp| csharp.shares_native_runtime),
            "csharp",
        )?;
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.cs", wrapper_class_name)),
            content: strip_trailing_whitespace(&methods::gen_wrapper_class(
                api,
                &namespace,
                &wrapper_class_name,
                &exception_class_name,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_callbacks,
                &streaming_methods,
                &streaming_methods_meta,
                &exclude_functions,
                &config.trait_bridges,
                &all_opaque_type_names,
                &config.adapters,
                &capsule_types,
            )),
            generated_header: true,
        });

        if has_visitor_callbacks {
            let visitor_bridge_cfg = config.trait_bridges.iter().find(|b| {
                b.bind_via == crate::core::config::BridgeBinding::OptionsField
                    && b.is_active_for(&Language::Csharp.to_string())
            });
            let trait_map: std::collections::HashMap<&str, &crate::core::ir::TypeDef> = api
                .types
                .iter()
                .filter(|t| t.is_trait)
                .map(|t| (t.name.as_str(), t))
                .collect();
            let visitor_trait = visitor_bridge_cfg.and_then(|b| trait_map.get(b.trait_name.as_str()).copied());

            if let (Some(bridge_cfg), Some(trait_def)) = (visitor_bridge_cfg, visitor_trait) {
                for (filename, content) in
                    crate::backends::csharp::gen_visitor::gen_visitor_files(&namespace, api, bridge_cfg, trait_def)
                {
                    files.push(GeneratedFile {
                        path: base_path.join(filename),
                        content: strip_trailing_whitespace(&content),
                        generated_header: true,
                    });
                }
            } else {
                tracing::warn!(
                    "gen_visitor(csharp): skip visitor support files — configured trait `{}` is absent from IR",
                    visitor_bridge_cfg.map_or("<unknown>", |bridge| bridge.trait_name.as_str())
                );
            }
            stale_candidates.extend(superseded_visitor_filenames());
        } else {
            stale_candidates.extend(stale_visitor_filenames(config));
        }

        if !config.trait_bridges.is_empty() {
            let trait_defs: Vec<_> = api.types.iter().filter(|t| t.is_trait).collect();
            let bridges: Vec<_> = config
                .trait_bridges
                .iter()
                .filter_map(|cfg| {
                    let trait_name = cfg.trait_name.clone();
                    trait_defs
                        .iter()
                        .find(|t| t.name == trait_name)
                        .map(|trait_def| (trait_name, cfg, *trait_def))
                })
                .collect();

            if !bridges.is_empty() {
                let visible_type_names: HashSet<&str> = api
                    .types
                    .iter()
                    .filter(|t| !t.is_trait)
                    .map(|t| t.name.as_str())
                    .chain(api.enums.iter().map(|e| e.name.as_str()))
                    .collect();
                let crate::backends::csharp::trait_bridge::TraitBridgesFile {
                    filename,
                    content,
                    vtable_slot_names,
                } = crate::backends::csharp::trait_bridge::gen_trait_bridges_file(
                    &namespace,
                    &prefix,
                    &bridges,
                    &visible_type_names,
                );

                for (trait_name, bridge_cfg, trait_def) in &bridges {
                    let Some((_, emitted)) = vtable_slot_names.iter().find(|(name, _)| name == trait_name) else {
                        continue;
                    };
                    assert_vtable_matches_rust_struct(
                        source_api,
                        trait_def,
                        bridge_cfg.super_trait.is_some(),
                        &bridge_cfg.ffi_skip_methods,
                        emitted,
                    )?;
                }

                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content: strip_trailing_whitespace(&content),
                    generated_header: true,
                });

                if let Some((filename, content)) = crate::backends::csharp::trait_bridge::gen_bridge_adapters_file(
                    &namespace,
                    &bridges,
                    &visible_type_names,
                ) {
                    files.push(GeneratedFile {
                        path: base_path.join(filename),
                        content: strip_trailing_whitespace(&content),
                        generated_header: true,
                    });
                }
            }
        }

        let enum_names: HashSet<String> = api.enums.iter().map(|e| csharp_type_name(&e.name)).collect();
        let enum_data_variant_names = enum_names_with_data_variants(api);

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                let type_filename = csharp_type_name(&typ.name);
                let client_ctor = config.client_constructors.get(&typ.name);
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&types::gen_opaque_handle(
                        typ,
                        &api.types,
                        &namespace,
                        &exception_class_name,
                        &enum_names,
                        &streaming_methods,
                        &streaming_methods_meta,
                        &all_opaque_type_names,
                        client_ctor,
                        &enum_data_variant_names,
                    )),
                    generated_header: true,
                });
            }
        }

        let complex_enums: HashSet<String> = HashSet::new();

        let tagged_union_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
            .map(|e| csharp_type_name(&e.name))
            .collect();

        let custom_converter_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                let is_tagged_union = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
                if is_tagged_union {
                    return false;
                }
                let rename_all_differs = matches!(
                    e.serde_rename_all.as_deref(),
                    Some("kebab-case") | Some("SCREAMING-KEBAB-CASE") | Some("camelCase") | Some("PascalCase")
                );
                if rename_all_differs {
                    return true;
                }
                e.variants.iter().any(|v| {
                    if let Some(ref rename) = v.serde_rename {
                        let default_wire_name =
                            crate::codegen::naming::wire_variant_value(&v.name, None, e.serde_rename_all.as_deref());
                        rename != &default_wire_name
                    } else {
                        false
                    }
                })
            })
            .map(|e| csharp_type_name(&e.name))
            .collect();

        let lang_rename_all = config.serde_rename_all_for_language(Language::Csharp);

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !typ.is_opaque {
                let has_visible_fields = binding_fields(&typ.fields).next().is_some();
                let has_named_fields = binding_fields(&typ.fields).any(|f| !is_tuple_field(f));
                if has_visible_fields && !has_named_fields {
                    continue;
                }
                if has_visitor_callbacks && bridge_associated_types.contains(typ.name.as_str()) {
                    continue;
                }

                let type_filename = csharp_type_name(&typ.name);
                let excluded_types: HashSet<String> =
                    api.excluded_type_paths.keys().map(|n| csharp_type_name(n)).collect();
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&types::gen_record_type(
                        typ,
                        &api.types,
                        &namespace,
                        &prefix,
                        &enum_names,
                        &complex_enums,
                        &custom_converter_enums,
                        &lang_rename_all,
                        &bridge_type_aliases,
                        &config.trait_bridges,
                        &exception_class_name,
                        &excluded_types,
                        &tagged_union_enums,
                        &all_opaque_type_names,
                    )),
                    generated_header: true,
                });
            }
        }

        let text_types = &config.untagged_union_text_types;
        for enum_def in &api.enums {
            if has_visitor_callbacks && bridge_associated_types.contains(enum_def.name.as_str()) {
                continue;
            }
            let enum_filename = csharp_type_name(&enum_def.name);
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", enum_filename)),
                content: strip_trailing_whitespace(&enums::gen_enum(enum_def, &namespace, text_types)),
                generated_header: true,
            });
        }

        let needs_byte_array_converter = api
            .types
            .iter()
            .any(|t| !t.is_opaque && !t.is_trait && !exclude_types.contains(&t.name));
        if needs_byte_array_converter {
            files.push(GeneratedFile {
                path: base_path.join("ByteArrayJsonConverter.cs"),
                content: types::gen_byte_array_to_int_array_converter(&namespace),
                generated_header: true,
            });
        }

        let needs_duration_converter = api
            .types
            .iter()
            .any(|t| binding_fields(&t.fields).any(field_uses_duration_map_wire));
        if needs_duration_converter {
            files.push(GeneratedFile {
                path: base_path.join("DurationMillisJsonConverter.cs"),
                content: types::gen_duration_millis_converter(&namespace),
                generated_header: true,
            });
        }

        files.push(GeneratedFile {
            path: base_path.join("JsonLeniency.cs"),
            content: types::gen_json_leniency(&namespace),
            generated_header: true,
        });

        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Csharp)?;

        files.push(GeneratedFile {
            path: PathBuf::from("packages/csharp/Directory.Build.props"),
            content: gen_directory_build_props(),
            generated_header: true,
        });

        // ~keep Deferred to here, after every emitter has pushed: the visitor-support check must be
        // able to see the whole run's output, or it reports files this very run wrote.
        let emitted: std::collections::HashSet<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
        report_unemitted_visitor_files(&base_path, &stale_candidates, &emitted);

        Ok(files)
    }

    /// C# wrapper class is already the public API.
    /// The `gen_wrapper_class` (generated in `generate_bindings`) provides high-level public methods
    /// that wrap NativeMethods (P/Invoke), marshal types, and handle errors.
    /// No additional facade is needed.
    fn generate_public_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let csharp_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Csharp);
        let enabled_features: HashSet<&str> = csharp_features.iter().map(String::as_str).collect();
        let filtered_api = crate::backends::ir_order::with_sorted_items(api).with_cfg_filtered_deep(&enabled_features);
        service_api::generate(&filtered_api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![PostBuildStep::StageFfiLibrary],
        })
    }

    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<crate::core::backend::TraitBridgeRegistrationSurface> {
        crate::backends::csharp::trait_bridge::registration_surface(api, config)
    }
}

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests;
