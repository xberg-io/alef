mod dart_traits;
mod errors;
mod functions;
mod render_type;
pub(super) mod service_api;
mod trait_bridge;
mod types;
mod wire_value;

use crate::backends::dart::naming::{dart_frb_version, dart_style};
use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep, PostProcessor,
    TraitBridgeRegistrationSurface,
};
use crate::core::config::{DartStyle, Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, FunctionDef};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::backends::dart::gen_ffi;
use crate::backends::dart::gen_rust_crate;

use dart_traits::emit_dart_traits;
pub(crate) use functions::config_param_is_named_optional;
use functions::emit_function;
use service_api as gen_service_api;
use trait_bridge::{dart_bridge_method_name, emit_trait_bridge_methods};

pub struct DartBackend;

impl Backend for DartBackend {
    fn name(&self) -> &str {
        "dart"
    }

    fn language(&self) -> Language {
        Language::Dart
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
        if dart_style(config) == DartStyle::Ffi {
            return gen_ffi::emit(api, config);
        }

        let module_name = dart_module_name(&config.name);
        let barrel_name = config
            .dart
            .as_ref()
            .and_then(|c| c.lib_name.as_deref())
            .map(|n| n.replace('-', "_"))
            .unwrap_or_else(|| module_name.clone());

        let exclude_functions: std::collections::HashSet<&str> = config
            .dart
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: std::collections::HashSet<&str> = config
            .dart
            .as_ref()
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let dart_wire_enums = wire_value::flat_wire_enums(&api.enums, &exclude_types);
        let dart_source_crate_name = config.name.replace('-', "_");
        let dart_configured_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Dart);

        let deduped_functions = crate::codegen::fn_dedup::dedup_same_name_functions(&api.functions);
        let visible_functions: Vec<&FunctionDef> = deduped_functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
            .filter(|f| {
                !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
            })
            .collect();

        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();

        body.push_str(&crate::backends::dart::template_env::render(
            "dart_bridge_export.jinja",
            minijinja::context! {
                module_name => module_name.as_str(),
            },
        ));
        body.push_str("export 'traits.dart';\n");
        // ~keep The matching `import 'traits.dart';` is appended here only if the body below
        // turns out to reference a trait name — see the insert after the body is complete.
        let traits_import_insert_at = body.len();

        let dart_backend_name = "dart";
        let active_bridge_configs: Vec<&TraitBridgeConfig> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == dart_backend_name))
            .filter(|b| b.register_fn.is_some() || b.unregister_fn.is_some() || b.clear_fn.is_some())
            .collect();

        // The `.wireValue` extensions need the same unprefixed import to the frb-generated
        // bridge as the bridge class does, so an enum-only crate (no visible functions, no
        // active trait bridges) must still open this block to get it.
        if !visible_functions.is_empty() || !active_bridge_configs.is_empty() || !dart_wire_enums.is_empty() {
            body.push_str(&crate::backends::dart::template_env::render(
                "dart_bridge_imports.jinja",
                minijinja::context! {
                    module_name => module_name.as_str(),
                },
            ));
            body.push('\n');

            if !visible_functions.is_empty() || !active_bridge_configs.is_empty() {
                let bridge_class = config.dart_bridge_class_name();
                body.push_str(&crate::backends::dart::template_env::render(
                    "dart_bridge_class_open.jinja",
                    minijinja::context! {
                        bridge_class => bridge_class.as_str(),
                    },
                ));
                for f in &visible_functions {
                    emit_function(f, &api.types, &api.enums, &mut body, &mut imports);
                    body.push('\n');
                }
                for bridge_cfg in &active_bridge_configs {
                    emit_trait_bridge_methods(bridge_cfg, &mut body);
                }
                emit_streaming_adapter_methods(config, &mut body, &mut imports);
                body.push_str("}\n");
                if !dart_wire_enums.is_empty() {
                    body.push('\n');
                }
            }

            wire_value::emit_wire_value_extensions(
                &dart_wire_enums,
                &dart_source_crate_name,
                Some(dart_configured_features.as_slice()),
                &mut body,
            );
        }

        // Whenever a typed-list name shows up anywhere in the body — either as a
        // default-value literal (`Int64List(0)`) or as a bare type (a function
        // returning `Int64List` directly) — every reference must resolve to
        // flutter_rust_bridge's generalized typed-list class, not the SDK's
        // `dart:typed_data` one; the two classes are not assignable to each
        // other. `render_type` still adds `dart:typed_data` per-type (needed by
        // `traits.dart`, which has no FRB import of its own), so drop it here
        // once the FRB import supersedes it, to avoid an `unused_import` lint
        // on the now-redundant SDK import.
        if body.contains("Int64List") || body.contains("Uint8List") || body.contains("Float64List") {
            imports.insert("import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';".to_string());
            imports.remove("import 'dart:typed_data';");
        }

        let dart_trait_names: Vec<&str> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == dart_backend_name))
            .map(|b| b.trait_name.as_str())
            .collect();

        // ~keep `export 'traits.dart';` does not bring the trait names into this file's own
        // scope, so a doc comment pointing at one (`[OcrBackend]`) trips `comment_references`
        // without an accompanying import. But emitting that import unconditionally trips the
        // opposite lint — `unused_import` — in every crate whose module file never names a
        // trait, which is the common case. Emit it only when the body actually refers to one.
        if dart_trait_names
            .iter()
            .any(|name| body[traits_import_insert_at..].contains(name))
        {
            body.insert_str(traits_import_insert_at, "import 'traits.dart';\n");
        }

        let mut content = String::new();
        content.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
        content.push_str("\n\n");
        for import in &imports {
            content.push_str(import);
            content.push('\n');
        }
        if !imports.is_empty() {
            content.push('\n');
        }
        content.push_str(&body);

        let dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
        let path = PathBuf::from(format!("{dir}/{module_name}.dart"));

        let barrel_dir = resolve_output_dir(None, &config.name, "packages/dart/lib");
        let barrel_path = PathBuf::from(format!("{barrel_dir}/{barrel_name}.dart"));
        let barrel_content = crate::backends::dart::template_env::render(
            "dart_barrel_file.jinja",
            minijinja::context! {
                module_name => module_name.as_str(),
            },
        );

        let mut files = vec![
            GeneratedFile {
                path,
                content,
                generated_header: false,
            },
            GeneratedFile {
                path: barrel_path,
                content: barrel_content,
                generated_header: false,
            },
        ];

        let rust_crate_files = gen_rust_crate::emit(api, config)?;
        files.extend(rust_crate_files);

        let mut traits_content = String::new();
        traits_content.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
        traits_content.push_str("\n\n");

        if !dart_trait_names.is_empty() {
            let (traits_body, traits_imports) = emit_dart_traits(api, &dart_trait_names);
            if !traits_body.is_empty() {
                traits_content.push_str(&crate::backends::dart::template_env::render(
                    "dart_bridge_import.jinja",
                    minijinja::context! {
                        module_name => module_name.as_str(),
                    },
                ));
                for import in &traits_imports {
                    traits_content.push_str(import);
                    traits_content.push('\n');
                }
                traits_content.push('\n');
                traits_content.push_str(&traits_body);
            } else {
                traits_content.push_str("// Traits module (generated stub — no trait bridges configured).\n");
                traits_content.push_str("// This file is kept for API surface consistency across language bindings.\n");
            }
        } else {
            traits_content
                .push_str("// Traits module (empty in Dart as Dart does not have trait systems like Rust).\n");
            traits_content.push_str("// This file is kept for API surface consistency across language bindings.\n");
        }

        let traits_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
        let traits_path = PathBuf::from(format!("{traits_dir}/traits.dart"));
        files.push(GeneratedFile {
            path: traits_path,
            content: traits_content,
            generated_header: false,
        });

        let lib_stem = config.name.replace('-', "_");
        let repo_url = config.github_repo();
        let crate_version = api.version.to_string();
        let package_name = config.dart_pubspec_name();
        let native_loader_ctx = minijinja::context! {
            crate_name => config.name.as_str(),
            lib_stem => lib_stem.as_str(),
            version => &crate_version,
            repo_url => &repo_url,
            package_name => package_name.as_str(),
            module_name => module_name.as_str(),
        };

        let helper_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
        let helper_path = PathBuf::from(format!("{helper_dir}/native_loader.dart"));
        let helper_content =
            crate::backends::dart::template_env::render("dart_native_loader_helper.jinja", native_loader_ctx.clone());
        files.push(GeneratedFile {
            path: helper_path,
            content: helper_content,
            generated_header: false,
        });

        let bin_dir = resolve_output_dir(None, &config.name, "packages/dart/bin");
        let bin_path = PathBuf::from(format!("{bin_dir}/download_libs.dart"));
        let bin_content = crate::backends::dart::template_env::render("bin_download_libs.jinja", native_loader_ctx);
        files.push(GeneratedFile {
            path: bin_path,
            content: bin_content,
            generated_header: false,
        });

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-dart",
            build_dep: BuildDependency::None,
            post_build: vec![PostBuildStep::RunCommand {
                cmd: "flutter_rust_bridge_codegen",
                args: vec![
                    "generate",
                    "--config-file",
                    "packages/dart/rust/flutter_rust_bridge.yaml",
                    "--no-deps-check",
                ],
            }],
        })
    }

    fn build_config_with_config(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        self.build_config_for(config)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        gen_service_api::generate(api, config)
    }

    /// Dart hangs the registration methods off the FRB bridge class, so each reported symbol is
    /// `<bridge class>.<method>`. `DartStyle::Ffi` short-circuits `generate_bindings` into
    /// `gen_ffi::emit`, which emits no trait-bridge methods at all — hence the style gate. ~keep
    fn trait_bridge_registration_surface(
        &self,
        _api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        if dart_style(config) == DartStyle::Ffi {
            return Vec::new();
        }
        let bridge_class = config.dart_bridge_class_name();
        let qualified = |configured: &Option<String>| {
            configured
                .as_deref()
                .map(|name| format!("{bridge_class}.{}", dart_bridge_method_name(name)))
        };
        config
            .trait_bridges
            .iter()
            .filter(|bridge| bridge.is_active_for("dart"))
            .filter(|bridge| {
                bridge.register_fn.is_some() || bridge.unregister_fn.is_some() || bridge.clear_fn.is_some()
            })
            .map(|bridge| TraitBridgeRegistrationSurface {
                trait_name: bridge.trait_name.clone(),
                register_symbol: qualified(&bridge.register_fn),
                unregister_symbol: qualified(&bridge.unregister_fn),
                clear_symbol: qualified(&bridge.clear_fn),
            })
            .collect()
    }
}

impl DartBackend {
    /// Return a `BuildConfig` that reflects the active bridging style from `config`.
    ///
    /// - `DartStyle::Ffi` — no Rust crate; use the shared C FFI library.
    /// - `DartStyle::Frb` — Rust crate + flutter_rust_bridge codegen (default).
    pub fn build_config_for(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        match dart_style(config) {
            DartStyle::Ffi => Some(BuildConfig {
                tool: "dart",
                crate_suffix: "",
                build_dep: BuildDependency::Ffi,
                post_build: vec![],
            }),
            DartStyle::Frb => {
                let module_name = dart_module_name(&config.name);
                let lib_dart_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
                let Some(lib_dart_path) = frb_dart_bridge_path(config) else {
                    unreachable!("frb_dart_bridge_path only returns None for DartStyle::Ffi, already matched above");
                };
                let lib_freezed_path = PathBuf::from(format!(
                    "{lib_dart_dir}/{module_name}_bridge_generated/lib.freezed.dart"
                ));
                let frb_generated_path = PathBuf::from(format!(
                    "{lib_dart_dir}/{module_name}_bridge_generated/frb_generated.dart"
                ));

                let exclude_functions: Vec<String> = config
                    .dart
                    .as_ref()
                    .map(|c| c.exclude_functions.clone())
                    .unwrap_or_default();

                let skip_frb = config.dart.as_ref().map(|c| c.skip_frb).unwrap_or(false);

                let Some((rust_lib_rs_path, rust_frb_generated_path)) = frb_rust_facade_paths(config) else {
                    unreachable!("frb_rust_facade_paths only returns None for DartStyle::Ffi, already matched above");
                };

                let mut post_build_steps: Vec<PostBuildStep> = if skip_frb {
                    vec![]
                } else {
                    vec![
                        // Must precede the RunCommand below: catches a `flutter_rust_bridge_codegen`
                        // on PATH whose version disagrees with the declared `frb_version` pin before
                        // that mismatched binary ever writes non-deterministic output (alef #204).
                        PostBuildStep::VerifyFrbCodegenVersion {
                            expected_version: dart_frb_version(config),
                        },
                        PostBuildStep::RunCommand {
                            cmd: "flutter_rust_bridge_codegen",
                            args: vec![
                                "generate",
                                "--config-file",
                                "packages/dart/rust/flutter_rust_bridge.yaml",
                                "--no-deps-check",
                            ],
                        },
                    ]
                };

                // Gate every post-processor below on the facade/bridge actually agreeing --
                // see `PostBuildStep::VerifyFrbBridgeCoverage`'s doc for why this must run before
                // any `PostProcessFile` rewrite of `lib_dart_path` (alef #135). ~keep
                post_build_steps.push(PostBuildStep::VerifyFrbBridgeCoverage {
                    facade_path: rust_lib_rs_path.clone(),
                    bridge_path: lib_dart_path.clone(),
                    exclude_functions: exclude_functions.clone(),
                });

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: lib_dart_path.clone(),
                    processor: PostProcessor::FrbDartExcludeFunctions(exclude_functions.clone()),
                });

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: lib_dart_path.clone(),
                    processor: PostProcessor::FrbDartSealedVariants,
                });

                if !config.untagged_union_text_types.is_empty() {
                    post_build_steps.push(PostBuildStep::PostProcessFile {
                        path: lib_dart_path.clone(),
                        processor: PostProcessor::FrbDartInjectTextMethods(config.untagged_union_text_types.clone()),
                    });
                }

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: frb_generated_path.clone(),
                    processor: PostProcessor::FrbDartExcludeFunctions(exclude_functions),
                });

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: frb_generated_path.clone(),
                    processor: PostProcessor::FrbDartSealedVariants,
                });

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: frb_generated_path.clone(),
                    processor: PostProcessor::FrbDartFixHandlerExecutorCalls,
                });

                for path in [lib_dart_path, frb_generated_path.clone(), lib_freezed_path] {
                    post_build_steps.push(PostBuildStep::PostProcessFile {
                        path,
                        processor: PostProcessor::DartStripTrailingWhitespace,
                    });
                }

                post_build_steps.push(PostBuildStep::CarryFrbCfgGates {
                    source_path: rust_lib_rs_path,
                    target_path: rust_frb_generated_path,
                });

                let lib_stem = format!("{}_dart", config.name.replace('-', "_"));
                post_build_steps.push(PostBuildStep::StageDartNatives { lib_stem });

                Some(BuildConfig {
                    tool: "cargo",
                    crate_suffix: "-dart",
                    build_dep: BuildDependency::None,
                    post_build: post_build_steps,
                })
            }
        }
    }
}

/// Emit streaming adapter methods (Stream<ItemType>) for adapters with owner_type set.
fn emit_streaming_adapter_methods(config: &ResolvedCrateConfig, out: &mut String, imports: &mut BTreeSet<String>) {
    use crate::core::config::AdapterPattern;
    use heck::ToLowerCamelCase;

    let module_name = dart_module_name(&config.name);

    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        if adapter.owner_type.is_none() || adapter.item_type.is_none() || adapter.params.is_empty() {
            continue;
        }
        if adapter.skip_languages.iter().any(|l| l == "dart") {
            continue;
        }

        let method_name = adapter.name.to_lower_camel_case();
        let item_type = adapter.item_type.as_deref().unwrap_or("Object");
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let owner_param = owner_type.chars().next().unwrap_or('o').to_lowercase().to_string() + &owner_type[1..];
        let request_type_full = adapter.params[0].ty.as_str();
        let request_type = request_type_full.rsplit("::").next().unwrap_or(request_type_full);
        let request_param = adapter.params[0].name.to_lower_camel_case();
        let request_param = if request_param.is_empty() {
            "request".to_string()
        } else {
            request_param
        };

        imports.insert("import 'dart:async' show Stream;".to_string());

        out.push_str(&crate::backends::dart::template_env::render(
            "dart_streaming_method.jinja",
            minijinja::context! {
                method_name => method_name,
                item_type => item_type,
                owner_type => owner_type,
                owner_param => owner_param,
                request_type => request_type,
                request_param => request_param,
                module_name => module_name.as_str(),
            },
        ));
        out.push('\n');
    }
}

/// Converts a crate name like `"my-lib"` to snake_case `"my_lib"`.
fn dart_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

/// The FRB Rust facade's `(lib.rs, frb_generated.rs)` paths for `config`, or `None` when the
/// active bridging style is `DartStyle::Ffi` (no Rust facade crate at all).
///
/// The single source of truth for these two paths: [`DartBackend::build_config_for`] uses it to
/// build the `CarryFrbCfgGates` post-build step, and `alef verify`'s frb-gate-drift check
/// (`bin_cli::core_commands::verify`) uses it to find the same two files read-only, so the paths
/// a write can target and the paths a check reads can never drift apart. See alef #179.
pub fn frb_rust_facade_paths(config: &ResolvedCrateConfig) -> Option<(PathBuf, PathBuf)> {
    if dart_style(config) == DartStyle::Ffi {
        return None;
    }
    let rust_crate_dir = resolve_output_dir(None, &config.name, "packages/dart/rust");
    let rust_lib_rs_path = PathBuf::from(format!("{rust_crate_dir}/src/lib.rs"));
    let rust_frb_generated_path = PathBuf::from(format!("{rust_crate_dir}/src/frb_generated.rs"));
    Some((rust_lib_rs_path, rust_frb_generated_path))
}

/// The `flutter_rust_bridge_codegen`-written `lib.dart` path for `config`, or `None` when the
/// active bridging style is `DartStyle::Ffi` (no FRB bridge at all).
///
/// The single source of truth for this path, mirroring [`frb_rust_facade_paths`]: this backend's
/// own `build_config_for` uses it to build `VerifyFrbBridgeCoverage`/`PostProcessFile` steps that
/// target the bridge, and `alef verify`'s Dart-bridge-field-drift check
/// (`bin_cli::core_commands::verify`) uses it to find the same file read-only, so a write target
/// and a read-only check can never independently drift apart on where the bridge lives.
pub fn frb_dart_bridge_path(config: &ResolvedCrateConfig) -> Option<PathBuf> {
    if dart_style(config) == DartStyle::Ffi {
        return None;
    }
    let module_name = dart_module_name(&config.name);
    let lib_dart_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
    Some(PathBuf::from(format!(
        "{lib_dart_dir}/{module_name}_bridge_generated/lib.dart"
    )))
}
