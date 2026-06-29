use crate::codegen::naming::{csharp_type_name, csharp_wrapper_class_name, to_csharp_name};
use crate::codegen::shared::binding_fields;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, FieldDef, TypeRef};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use files::{
    csharp_file_header, delete_stale_visitor_files, delete_superseded_visitor_files, gen_directory_build_props,
    strip_trailing_whitespace,
};
use marshalling::{
    bytes_len_arg, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, needs_param_teardown, pinvoke_param_type, pinvoke_return_type,
    returns_bool_via_int, returns_json_object, returns_ptr, returns_string,
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

pub(super) mod enums;
pub(super) mod errors;
mod files;
pub(super) mod functions;
mod marshalling;
pub(super) mod methods;
pub(super) mod service_api;
pub(super) mod types;

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

impl CsharpBackend {
    // lib_name comes from config.ffi_lib_name()
}

fn effective_exclude_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(csharp) = &config.csharp {
        exclude_types.extend(csharp.exclude_types.iter().cloned());
    }
    // Also exclude types marked as binding_excluded (service-owned types emitted via service API)
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    // Mirror the FFI backend's `contains('<')` filter: workspace-declared opaque types whose
    // `rust_path` carries generic parameters cannot be represented in the C ABI, so the FFI
    // backend skips emitting `_new`/`_free` symbols. C# P/Invoke imports those symbols, so
    // the C# backend must follow the same rule to avoid `EntryPointNotFoundException` at
    // runtime (or DllNotFoundException-shaped linker errors during AOT build).
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
        typ.methods
            .retain(|method| !signature_references_excluded_type(&method.params, &method.return_type, exclude_types));
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
        let exclude_types = effective_exclude_types(api, config);
        let filtered_api;
        let api = if exclude_types.is_empty() {
            api
        } else {
            filtered_api = api_without_excluded_types(api, &exclude_types);
            &filtered_api
        };
        // C# is a single compiled surface (P/Invoke extern + wrapper) with no Rust-cfg gating, so
        // same-named cfg-variant functions must collapse to one method/extern to avoid duplicate
        // member errors. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;
        let namespace = config.csharp_namespace();
        let prefix = config.ffi_prefix();
        let lib_name = config.ffi_lib_name();

        // Collect bridge param names and type aliases from trait_bridges config so we can strip
        // them from generated function signatures and emit ConvertWithVisitor instead.
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
        // Only emit ConvertWithVisitor method if visitor_callbacks is explicitly enabled in FFI config
        let has_visitor_callbacks = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false);
        let bridge_associated_types = config.bridge_associated_types();

        // Streaming adapter methods are emitted via the iterator-handle FFI protocol
        // (`{prefix}_{owner}_{name}_start` / `_next` / `_free`) — not as direct P/Invoke calls
        // of the callback-based variant. The set is still used to skip the default
        // method-emission path; the parallel meta map drives the `IAsyncEnumerable` emitters.
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

        // Functions explicitly excluded from C# bindings (e.g., not present in the C FFI layer).
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

        // Fallback generic exception class name (used by GetLastError and as base for typed errors)
        let exception_class_name = format!("{}Exception", to_csharp_name(&api.crate_name));

        // 1. Generate NativeMethods.cs
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
            )),
            generated_header: true,
        });

        // 2. Generate error types from thiserror enums (if any), otherwise generic exception.
        //
        // Two thiserror enums in the same crate can declare variants with identical
        // names (e.g. `GraphQLError::ValidationError` and `SchemaError::ValidationError`
        // in sample_project). Each variant emits `{VariantName}Exception.cs`, so without
        // deduplication two `GeneratedFile` entries share the same `path` and the
        // parallel `write_files` step racily overwrites the file — leaving a tail of
        // bytes from whichever payload was longer past the file's logical end
        // (because the second writer's truncate-on-open happens at file open time,
        // before the first writer's pending bytes have all reached disk).
        //
        // Keep the first emission per class name; subsequent same-named variants
        // are dropped. The base error class (`{ErrorEnum}Exception`) naturally
        // varies by error name and does not collide.
        if !api.errors.is_empty() {
            let mut seen_exception_files: HashSet<String> = HashSet::new();
            for error in &api.errors {
                let error_files =
                    crate::codegen::error_gen::gen_csharp_error_types(error, &namespace, Some(&exception_class_name));
                for (class_name, content) in error_files {
                    if !seen_exception_files.insert(class_name.clone()) {
                        // Duplicate variant name across error enums — earlier
                        // emission wins. Without this skip, two `GeneratedFile`
                        // entries share the same path and racily overwrite each
                        // other in `write_files`.
                        continue;
                    }
                    files.push(GeneratedFile {
                        path: base_path.join(format!("{}.cs", class_name)),
                        content: strip_trailing_whitespace(&content),
                        generated_header: false, // already has header
                    });
                }
            }
        }

        // Fallback generic exception class (always generated for GetLastError)
        if api.errors.is_empty()
            || !api
                .errors
                .iter()
                .any(|e| format!("{}Exception", e.name) == exception_class_name)
        {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", exception_class_name)),
                content: strip_trailing_whitespace(&errors::gen_exception_class(&namespace, &exception_class_name)),
                generated_header: true,
            });
        }

        // Collect all opaque type names (pascal-cased) for instance method detection
        let all_opaque_type_names: HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| csharp_type_name(&t.name))
            .collect();

        // 3. Generate main wrapper class
        let wrapper_class_name = csharp_wrapper_class_name(&api.crate_name, &namespace);
        let capsule_types = config
            .csharp
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
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

        // 3b. Generate visitor support files when a bridge is configured.
        if has_visitor_callbacks {
            // Look up the visitor trait def from the IR via TraitBridgeConfig.trait_name,
            // mirroring the Go backend's pattern so that gen_visitor_files is IR-driven.
            let visitor_bridge_cfg = config
                .trait_bridges
                .iter()
                .find(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField);
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
                eprintln!(
                    "[alef] gen_visitor(csharp): skip visitor support files — configured trait `{}` is absent from IR",
                    visitor_bridge_cfg.map_or("<unknown>", |bridge| bridge.trait_name.as_str())
                );
            }
            // IVisitor.cs and VisitorCallbacks.cs were removed from gen_visitor_files() in favour
            // of the configured bridge path in TraitBridges.cs. Delete any stale copies left
            // over from earlier generator runs.
            delete_superseded_visitor_files(&base_path)?;
        } else {
            // When visitor_callbacks is disabled, delete stale files from prior runs
            // to prevent CS8632 warnings (nullable context not enabled).
            delete_stale_visitor_files(&base_path, config)?;
        }

        // 3c. Generate trait bridge classes when configured.
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
                // Collect visible type names (non-trait types that have C# bindings).
                // Includes both `api.types` and `api.enums` so trait-bridge method signatures
                // can reference visitor result enum types without falling back to `string`.
                let visible_type_names: HashSet<&str> = api
                    .types
                    .iter()
                    .filter(|t| !t.is_trait)
                    .map(|t| t.name.as_str())
                    .chain(api.enums.iter().map(|e| e.name.as_str()))
                    .collect();
                let (filename, content) = crate::backends::csharp::trait_bridge::gen_trait_bridges_file(
                    &namespace,
                    &prefix,
                    &bridges,
                    &visible_type_names,
                );
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content: strip_trailing_whitespace(&content),
                    generated_header: true,
                });

                // Generate bridge adapters (Path A pattern: sealed adapter classes wrapping user impls)
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

        // Collect enum names so record generation can distinguish enum fields from class fields.
        let enum_names: HashSet<String> = api.enums.iter().map(|e| csharp_type_name(&e.name)).collect();

        // 4. Generate opaque handle classes
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
                    )),
                    generated_header: true,
                });
            }
        }

        // Untagged unions with data variants now emit as JsonElement-wrapper classes
        // (see gen_untagged_wrapper). The set is intentionally empty so record fields
        // keep their wrapper-class type instead of being downcast to JsonElement.
        let complex_enums: HashSet<String> = HashSet::new();

        // Tagged-union enums (serde-tagged data enums) are emitted as
        // `public abstract record Base { public sealed record Variant() : Base; }`
        // where `Base.Variant` is a TYPE — property defaults must be `new Base.Variant()`
        // rather than the bare `Base.Variant`, otherwise C# raises CS0119
        // ("X is a type, which is not valid in the given context").
        let tagged_union_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
            .map(|e| csharp_type_name(&e.name))
            .collect();

        // Collect enums that require a custom JsonConverter (non-standard serialized names only).
        // Tagged unions are generated as abstract records with [JsonPolymorphic] and do NOT need
        // a custom converter — the attribute on the type itself handles polymorphic deserialization.
        // When a property has a custom-converter enum as its type, emit a property-level
        // [JsonConverter] attribute so the custom converter wins over the global JsonStringEnumConverter.
        let custom_converter_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                // Skip tagged unions — they use [JsonPolymorphic] instead
                let is_tagged_union = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
                if is_tagged_union {
                    return false;
                }
                // Enums whose `serde_rename_all` is something other than snake_case
                // (e.g. "kebab-case" for `FilePurpose::FineTune` → `"fine-tune"`)
                // need a custom converter — `JsonStringEnumConverter(SnakeCaseLower)`
                // would write `"fine_tune"` instead.
                let rename_all_differs = matches!(
                    e.serde_rename_all.as_deref(),
                    Some("kebab-case") | Some("SCREAMING-KEBAB-CASE") | Some("camelCase") | Some("PascalCase")
                );
                if rename_all_differs {
                    return true;
                }
                // Enums with non-standard variant names need a custom converter
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

        // Resolve the language-level serde rename_all strategy (always wins over IR type-level).
        let lang_rename_all = config.serde_rename_all_for_language(Language::Csharp);

        // 5. Generate record types (structs)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !typ.is_opaque {
                // Skip types where all fields are unnamed tuple positions — they have no
                // meaningful properties to expose in C#.
                let has_visible_fields = binding_fields(&typ.fields).next().is_some();
                let has_named_fields = binding_fields(&typ.fields).any(|f| !is_tuple_field(f));
                if has_visible_fields && !has_named_fields {
                    continue;
                }
                // Skip types that gen_visitor handles with richer visitor-specific versions
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

        // 6. Generate enums
        let text_types = &config.untagged_union_text_types;
        for enum_def in &api.enums {
            // Skip enums that gen_visitor handles with richer visitor-specific versions
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

        // 7. Generate ByteArrayJsonConverter if any generated record can reference it.
        // record_json_options.jinja always registers the converter so generated record
        // code compiles even before a byte[] field is added.
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

        // 7b. Generate the JsonLeniency helper (lenient deserialization that drops
        // unknown properties). Always emitted so it is alef-owned rather than a
        // hand-maintained support file in the package.
        files.push(GeneratedFile {
            path: base_path.join("JsonLeniency.cs"),
            content: types::gen_json_leniency(&namespace),
            generated_header: true,
        });

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Csharp)?;

        // 8. Generate Directory.Build.props at the package root (always overwritten).
        // This file enables Nullable=enable and latest LangVersion for all C# projects
        // in the packages/csharp hierarchy without requiring per-csproj configuration.
        files.push(GeneratedFile {
            path: PathBuf::from("packages/csharp/Directory.Build.props"),
            content: gen_directory_build_props(),
            generated_header: true,
        });

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
        // C#'s wrapper class IS the public API — no additional wrapper needed.
        Ok(vec![])
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
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}
