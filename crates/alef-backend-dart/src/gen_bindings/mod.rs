mod dart_traits;
mod errors;
mod functions;
mod render_type;
mod types;

use crate::naming::dart_style;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use alef_core::config::{DartStyle, Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use alef_core::ir::{ApiSurface, FunctionDef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::gen_ffi;
use crate::gen_rust_crate;

use dart_traits::emit_dart_traits;
use functions::emit_function;

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
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        if dart_style(config) == DartStyle::Ffi {
            return gen_ffi::emit(api, config);
        }

        let module_name = dart_module_name(&config.name);
        // D9: the barrel file should use `lib_name` when configured (e.g. `lib_name = "h2m"`
        // produces `lib/h2m.dart`), falling back to the crate-name-derived module name.
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

        let visible_functions: Vec<&FunctionDef> = api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
            .collect();

        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();

        // For FRB style: all types come from the FRB-generated lib.dart.
        // We export it so that callers of `kreuzberg.dart` get all types from
        // one import and there are no duplicate-type conflicts.
        //
        // We also import it as `rust_bridge` for calling bridge functions
        // from within the wrapper class. Dart allows both export and import
        // of the same URI — the export makes types visible to callers; the
        // import (with prefix) makes the free functions callable here.
        body.push_str(&crate::template_env::render(
            "dart_bridge_export.jinja",
            minijinja::context! {
                module_name => module_name.as_str(),
            },
        ));

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
            // (ExtractionConfig, ResultFormat, etc.) available unqualified inside the
            // class body for use in return-type annotations and default-value literals.
            body.push_str(&crate::template_env::render(
                "dart_bridge_imports.jinja",
                minijinja::context! {
                    module_name => module_name.as_str(),
                },
            ));
            body.push('\n');

            let bridge_class = dart_bridge_class_name(&config.name);
            body.push_str(&crate::template_env::render(
                "dart_bridge_class_open.jinja",
                minijinja::context! {
                    bridge_class => bridge_class.as_str(),
                },
            ));
            for f in &visible_functions {
                emit_function(f, &mut body, &mut imports);
                body.push('\n');
            }
            // Emit static register/unregister/clear wrapper methods for each active
            // trait bridge config. FRB bridges the underlying `pub fn`s as free Dart
            // functions; these wrappers expose them as named static methods on the
            // bridge class so Dart callers have a single, discoverable entry point.
            for bridge_cfg in &active_bridge_configs {
                emit_trait_bridge_methods(bridge_cfg, &mut body);
            }
            body.push_str("}\n");
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
        let path = PathBuf::from(dir).join(format!("{module_name}.dart"));

        // Emit the top-level barrel file `lib/<barrel>.dart` so that consumers
        // can import `package:<pkg>/<pkg>.dart` (the canonical Dart import path).
        // Uses `lib_name` when configured (D9 fix), otherwise falls back to module_name.
        let barrel_dir = resolve_output_dir(None, &config.name, "packages/dart/lib");
        let barrel_path = PathBuf::from(barrel_dir).join(format!("{barrel_name}.dart"));
        let barrel_content = crate::template_env::render(
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

        // Emit traits.dart when at least one [[trait_bridges]] entry is configured
        // for the Dart backend (not excluded).
        let trait_names: Vec<&str> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == dart_backend_name))
            .map(|b| b.trait_name.as_str())
            .collect();

        if !trait_names.is_empty() {
            let (traits_body, traits_imports) = emit_dart_traits(api, &trait_names);
            if !traits_body.is_empty() {
                let mut traits_content = String::new();
                traits_content.push_str("// Generated by alef. Do not edit by hand.\n\n");
                // Trait files use types generated by FRB (ExtractionResult, OcrConfig, etc.)
                // so they must import the bridge-generated lib to resolve those types.
                traits_content.push_str(&crate::template_env::render(
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

                let traits_dir = resolve_output_dir(None, &config.name, "packages/dart/lib/src");
                let traits_path = PathBuf::from(traits_dir).join("traits.dart");
                files.push(GeneratedFile {
                    path: traits_path,
                    content: traits_content,
                    generated_header: false,
                });
            }
        }

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-dart",
            build_dep: BuildDependency::None,
            post_build: vec![PostBuildStep::RunCommand {
                cmd: "flutter_rust_bridge_codegen",
                args: vec!["generate"],
            }],
        })
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
            DartStyle::Frb => Some(BuildConfig {
                tool: "cargo",
                crate_suffix: "-dart",
                build_dep: BuildDependency::None,
                post_build: vec![PostBuildStep::RunCommand {
                    cmd: "flutter_rust_bridge_codegen",
                    args: vec!["generate"],
                }],
            }),
        }
    }
}

/// Emit `static Future<void>` wrapper methods for a trait bridge in the Dart bridge class.
///
/// For each of `register_fn`, `unregister_fn`, and `clear_fn` that is set on the config,
/// emits a corresponding Dart static method that delegates to the FRB-bridged free function:
///
/// - `register_fn`   → `static Future<void> registerXxx(XxxDartImpl impl_) async { ... }`
/// - `unregister_fn` → `static Future<void> unregisterXxx(String name) async { ... }`
/// - `clear_fn`      → `static Future<void> clearXxxs() async { ... }`
///
/// The names are converted to lowerCamelCase (matching FRB's Dart naming convention).
fn emit_trait_bridge_methods(bridge_cfg: &TraitBridgeConfig, out: &mut String) {
    let trait_name = &bridge_cfg.trait_name;
    let impl_type = format!("{trait_name}DartImpl");

    if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
        let dart_name = register_fn.to_lower_camel_case();
        out.push_str(&crate::template_env::render(
            "dart_trait_register_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
                impl_type => impl_type.as_str(),
            },
        ));
    }

    if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
        let dart_name = unregister_fn.to_lower_camel_case();
        out.push_str(&crate::template_env::render(
            "dart_trait_unregister_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
            },
        ));
    }

    if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
        let dart_name = clear_fn.to_lower_camel_case();
        out.push_str(&crate::template_env::render(
            "dart_trait_clear_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
            },
        ));
    }
}

/// Converts a crate name like `"my-lib"` to snake_case `"my_lib"`.
fn dart_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

/// Converts a crate name like `"my-lib"` to `"MyLibBridge"` for the bridge class.
fn dart_bridge_class_name(crate_name: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in crate_name.chars() {
        if ch == '-' || ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out.push_str("Bridge");
    out
}
