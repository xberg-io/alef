mod dart_traits;
mod errors;
mod functions;
mod render_type;
pub(super) mod service_api;
mod trait_bridge;
mod types;

use crate::backends::dart::naming::dart_style;
use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep, PostProcessor,
};
use crate::core::config::{DartStyle, Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, FunctionDef};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::backends::dart::gen_ffi;
use crate::backends::dart::gen_rust_crate;

use dart_traits::emit_dart_traits;
use functions::emit_function;
use service_api as gen_service_api;
use trait_bridge::emit_trait_bridge_methods;

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
        // The barrel file should use `lib_name` when configured, falling back to
        // the crate-name-derived module name.
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
        let _exclude_types: std::collections::HashSet<&str> = config
            .dart
            .as_ref()
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        // The Dart host facade is a single compiled surface with no Rust-cfg gating, so same-named
        // cfg-variant functions (real impl + no-ORT stub fallback) must collapse to a single
        // forwarder to avoid "already declared in this scope" errors. The frb Rust bridge below
        // keeps the original multi-entry `api`, which it cfg-filters itself. See codegen::fn_dedup.
        let deduped_functions = crate::codegen::fn_dedup::dedup_same_name_functions(&api.functions);
        let visible_functions: Vec<&FunctionDef> = deduped_functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
            // Skip trait-bridge-managed lifecycle names — `emit_trait_bridge_methods`
            // emits its own static wrapper for them. Without this filter the userland
            // Dart class declares lifecycle methods twice (regular forwarder + bridge
            // wrapper) which Dart rejects with "already declared in this scope".
            .filter(|f| {
                !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
            })
            .collect();

        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();

        // For FRB style: all types come from the FRB-generated lib.dart.
        // We export it so that callers of the package barrel get all types from
        // one import and there are no duplicate-type conflicts.
        //
        // We also import it as `rust_bridge` for calling bridge functions
        // from within the wrapper class. Dart allows both export and import
        // of the same URI — the export makes types visible to callers; the
        // import (with prefix) makes the free functions callable here.
        body.push_str(&crate::backends::dart::template_env::render(
            "dart_bridge_export.jinja",
            minijinja::context! {
                module_name => module_name.as_str(),
            },
        ));
        body.push_str("export 'traits.dart';\n");

        // Collect trait bridge configs that are not excluded for Dart and have at least
        // one of register_fn / unregister_fn / clear_fn set. These produce additional
        // static wrapper methods in the bridge class.
        let dart_backend_name = "dart";
        let active_bridge_configs: Vec<&TraitBridgeConfig> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == dart_backend_name))
            .filter(|b| b.register_fn.is_some() || b.unregister_fn.is_some() || b.clear_fn.is_some())
            .collect();

        if !visible_functions.is_empty() || !active_bridge_configs.is_empty() {
            // FRB places its generated Dart code in a subdirectory named
            // `{module_name}_bridge_generated/` and exposes it via `lib.dart`.
            //
            // The prefixed import (`as rust_bridge`) lets us call bridge free-functions
            // without namespace collisions. The unqualified import makes all FRB types
            // generated bridge types available unqualified inside the
            // class body for use in return-type annotations and default-value literals.
            body.push_str(&crate::backends::dart::template_env::render(
                "dart_bridge_imports.jinja",
                minijinja::context! {
                    module_name => module_name.as_str(),
                },
            ));
            body.push('\n');

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
            // Emit static register/unregister/clear wrapper methods for each active
            // trait bridge config. FRB bridges the underlying `pub fn`s as free Dart
            // functions; these wrappers expose them as named static methods on the
            // bridge class so Dart callers have a single, discoverable entry point.
            for bridge_cfg in &active_bridge_configs {
                emit_trait_bridge_methods(bridge_cfg, &mut body);
            }
            // Emit streaming adapter methods for adapters with owner_type set.
            emit_streaming_adapter_methods(config, &mut body, &mut imports);
            body.push_str("}\n");
        }

        // Ensure flutter_rust_bridge's generalized typed-list types are imported
        // when the body references a typed-list constructor in a default-value
        // literal. `empty_vec_literal` emits `Int64List(0)`, `Uint8List(0)`, or
        // `Float64List(0)` for empty-Vec defaults of widened-integer / byte /
        // float element types, but the literal sites don't thread the imports
        // set through, leaving these unresolved. We import FRB's generalized
        // typed-list module (not `dart:typed_data`) because FRB's generated
        // structs use FRB's `Int64List`, not the SDK's — the two are not
        // assignable to each other.
        if body.contains("Int64List(") || body.contains("Uint8List(") || body.contains("Float64List(") {
            imports.insert("import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';".to_string());
        }

        let mut content = String::new();
        content.push_str("// Generated by alef. Do not edit by hand.\n\n");
        // Write collected imports (e.g. dart:typed_data for Uint8List) before the body.
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

        // Emit the top-level barrel file `lib/<barrel>.dart` so that consumers
        // can import `package:<pkg>/<pkg>.dart` (the canonical Dart import path).
        // Uses `lib_name` when configured (D9 fix), otherwise falls back to module_name.
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

        // Emit traits.dart — either with trait bridge content or as an empty stub.
        // The export statement above always references traits.dart, so it must exist.
        let trait_names: Vec<&str> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == dart_backend_name))
            .map(|b| b.trait_name.as_str())
            .collect();

        let mut traits_content = String::new();
        traits_content.push_str("// Generated by alef. Do not edit by hand.\n\n");

        if !trait_names.is_empty() {
            let (traits_body, traits_imports) = emit_dart_traits(api, &trait_names);
            if !traits_body.is_empty() {
                // Trait files use types generated by FRB (ExtractionResult, OcrConfig, etc.)
                // so they must import the bridge-generated lib to resolve those types.
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
                // No trait names produced content; emit stub comment.
                traits_content.push_str("// Traits module (generated stub — no trait bridges configured).\n");
                traits_content.push_str("// This file is kept for API surface consistency across language bindings.\n");
            }
        } else {
            // No trait bridges configured; emit stub.
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

        // Emit bin/download_libs.dart — runtime script to fetch native libs from GitHub releases.
        // This script resolves the published native libraries to lib/src/native/<rid>/ on first import.
        let bin_dir = resolve_output_dir(None, &config.name, "packages/dart/bin");
        let bin_path = PathBuf::from(format!("{bin_dir}/download_libs.dart"));
        let lib_stem = config.name.replace('-', "_");
        let repo_url = config.github_repo();
        let crate_version = api.version.to_string();
        let bin_content = crate::backends::dart::template_env::render(
            "bin_download_libs.jinja",
            minijinja::context! {
                crate_name => config.name.as_str(),
                lib_stem => lib_stem.as_str(),
                version => &crate_version,
                repo_url => &repo_url,
            },
        );
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
                // flutter_rust_bridge places the generated Dart code at
                // `{dart_output}/lib.dart` where `dart_output` defaults to
                // `../lib/src/{module_name}_bridge_generated` relative to the rust
                // crate root.  Post-processing rewrites positional field names
                // (`field0`) to payload-derived names so callers get an ergonomic API.
                let lib_dart_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
                let lib_dart_path = PathBuf::from(format!("{lib_dart_dir}/{module_name}_bridge_generated/lib.dart"));
                let lib_freezed_path = PathBuf::from(format!(
                    "{lib_dart_dir}/{module_name}_bridge_generated/lib.freezed.dart"
                ));
                // `frb_generated.dart` carries flutter_rust_bridge's entrypoint and
                // its default external-library loader config (the build-tree-relative
                // `ioDirectory`). Post-processing injects a published-package loader
                // so the native library resolves from the package's own installed
                // location instead of a path that only exists in the build tree.
                let frb_generated_path = PathBuf::from(format!(
                    "{lib_dart_dir}/{module_name}_bridge_generated/frb_generated.dart"
                ));

                // Collect excluded functions to pass to the post-processor.
                let exclude_functions: Vec<String> = config
                    .dart
                    .as_ref()
                    .map(|c| c.exclude_functions.clone())
                    .unwrap_or_default();

                let skip_frb = config.dart.as_ref().map(|c| c.skip_frb).unwrap_or(false);

                // RunCommand invokes flutter_rust_bridge_codegen to generate the
                // Dart bridge from the Rust binding crate.  Skip when the caller
                // has opted out via `[crates.dart] skip_frb = true` or the
                // `--skip-frb` CLI flag (which sets ALEF_SKIP_COMMANDS).
                let mut post_build_steps: Vec<PostBuildStep> = if skip_frb {
                    vec![]
                } else {
                    vec![PostBuildStep::RunCommand {
                        cmd: "flutter_rust_bridge_codegen",
                        args: vec![
                            "generate",
                            "--config-file",
                            "packages/dart/rust/flutter_rust_bridge.yaml",
                        ],
                    }]
                };

                // Use the dedicated post-processor to filter excluded functions from lib.dart.
                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: lib_dart_path.clone(),
                    processor: PostProcessor::FrbDartExcludeFunctions(exclude_functions.clone()),
                });

                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: lib_dart_path.clone(),
                    processor: PostProcessor::FrbDartSealedVariants,
                });

                // Inject display-as-text extension methods on untagged union types so they
                // can be stringified in assertions. This must run after variant rewriting
                // so parameter names are resolved when the extension accesses them.
                if !config.untagged_union_text_types.is_empty() {
                    post_build_steps.push(PostBuildStep::PostProcessFile {
                        path: lib_dart_path.clone(),
                        processor: PostProcessor::FrbDartInjectTextMethods(config.untagged_union_text_types.clone()),
                    });
                }

                // Filter excluded functions from frb_generated.dart as well, since FRB
                // generates Rust FFI bridge wrappers there (e.g., `crateCalculateQualityScore`).
                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: frb_generated_path.clone(),
                    processor: PostProcessor::FrbDartExcludeFunctions(exclude_functions),
                });

                // Inject the published-package native-library loader into
                // `frb_generated.dart`. `FrbDartSealedVariants` also applies the
                // loader fix (keyed off the FRB loader config present only in this
                // file); it is idempotent and a no-op when already applied.
                post_build_steps.push(PostBuildStep::PostProcessFile {
                    path: frb_generated_path.clone(),
                    processor: PostProcessor::FrbDartSealedVariants,
                });

                // Fix FRB-generated Dart code that incorrectly calls executeSync/executeNormal
                // on callback function parameters. The handler is a function type, not an object
                // with these methods, so we rewrite the calls to use the RustLib binding instead.
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

                // Stage prebuilt native libraries from the build output into the Dart package.
                // This allows flutter_rust_bridge to find the native library at runtime
                // without requiring a local Rust build by the consumer.
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

        // Ensure Stream type is imported
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
