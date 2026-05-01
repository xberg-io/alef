use ahash::AHashSet;
use alef_codegen::naming::to_class_name;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::ApiSurface;
use std::collections::HashSet;
use std::path::PathBuf;

mod facade;
mod ffi_class;
mod helpers;
mod marshal;
mod native_lib;
mod trait_bridge;
mod types;

use facade::gen_facade_class;
use ffi_class::gen_main_class;
use helpers::gen_exception_class;
use native_lib::gen_native_lib;
use types::{gen_builder_class, gen_enum_class, gen_opaque_handle_class, gen_record_type};

pub struct JavaBackend;

impl JavaBackend {
    /// Convert crate name to main class name (PascalCase + "Rs" suffix).
    ///
    /// The "Rs" suffix ensures the raw FFI wrapper class has a distinct name from
    /// the public facade class (which strips the "Rs" suffix). Without this, the
    /// facade would delegate to itself, causing infinite recursion.
    fn resolve_main_class(api: &ApiSurface) -> String {
        let base = to_class_name(&api.crate_name.replace('-', "_"));
        if base.ends_with("Rs") {
            base
        } else {
            format!("{}Rs", base)
        }
    }
}

impl Backend for JavaBackend {
    fn name(&self) -> &str {
        "java"
    }

    fn language(&self) -> Language {
        Language::Java
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = resolve_output_dir(
            config.output.java.as_ref(),
            &config.crate_config.name,
            "packages/java/src/main/java/",
        );

        // If output_dir already ends with the package path (user configured the full path),
        // use it as-is. Otherwise, append the package path.
        let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
            PathBuf::from(&output_dir)
        } else {
            PathBuf::from(&output_dir).join(&package_path)
        };

        // Collect bridge param names and type aliases so we can strip them from generated
        // function signatures and emit convertWithVisitor instead.
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
        // Only generate visitor support if visitor_callbacks is explicitly enabled in FFI config
        let has_visitor_pattern = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false);

        let mut files = Vec::new();

        // 0. package-info.java - required by Checkstyle
        let description = config
            .scaffold
            .as_ref()
            .and_then(|s| s.description.as_deref())
            .unwrap_or("High-performance HTML to Markdown converter.");
        files.push(GeneratedFile {
            path: base_path.join("package-info.java"),
            content: format!(
                "/**\n * {description}\n */\npackage {package};\n",
                description = description,
                package = package,
            ),
            generated_header: true,
        });

        // 1. NativeLib.java - FFI method handles
        files.push(GeneratedFile {
            path: base_path.join("NativeLib.java"),
            content: gen_native_lib(api, config, &package, &prefix, has_visitor_pattern),
            generated_header: true,
        });

        // 2. Main wrapper class
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.java", main_class)),
            content: gen_main_class(
                api,
                config,
                &package,
                &main_class,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_pattern,
            ),
            generated_header: true,
        });

        // 3. Exception class
        files.push(GeneratedFile {
            path: base_path.join(format!("{}Exception.java", main_class)),
            content: gen_exception_class(&package, &main_class),
            generated_header: true,
        });

        // Collect complex enums (enums with data variants and no serde tag) — use Object for these fields.
        // Tagged unions (serde_tag is set) are now generated as proper sealed interfaces
        // and can be deserialized as their concrete types, so they are NOT complex_enums.
        let complex_enums: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_none() && e.variants.iter().any(|v| !v.fields.is_empty()))
            .map(|e| e.name.clone())
            .collect();

        // Resolve language-level serde rename strategy (always wins over IR type-level).
        let lang_rename_all = config.serde_rename_all_for_language(Language::Java);

        // 4. Record types
        // Include non-opaque types that either have fields OR are serializable unit structs
        // (has_serde + has_default, empty fields). Unit structs like `ExcelMetadata` need a
        // concrete Java class so they can be referenced as record components in tagged-union
        // variant records (e.g. FormatMetadata.Excel(@JsonUnwrapped ExcelMetadata value)).
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            let is_unit_serde = !typ.is_opaque && typ.fields.is_empty() && typ.has_serde;
            if !typ.is_opaque && (!typ.fields.is_empty() || is_unit_serde) {
                // Skip types that gen_visitor handles with richer visitor-specific versions
                if has_visitor_pattern && (typ.name == "NodeContext" || typ.name == "VisitResult") {
                    continue;
                }
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_record_type(&package, typ, &complex_enums, &lang_rename_all),
                    generated_header: true,
                });
                // Generate builder class for types with defaults
                if typ.has_default {
                    files.push(GeneratedFile {
                        path: base_path.join(format!("{}Builder.java", typ.name)),
                        content: gen_builder_class(&package, typ),
                        generated_header: true,
                    });
                }
            }
        }

        // Collect builder class names generated from record types with defaults,
        // so we can skip opaque types that would collide with them.
        let builder_class_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_opaque && (!t.fields.is_empty() || (t.has_serde && t.fields.is_empty())) && t.has_default)
            .map(|t| format!("{}Builder", t.name))
            .collect();

        // 4b. Opaque handle types (skip if a pure-Java builder already covers this name)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque && !builder_class_names.contains(&typ.name) {
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_opaque_handle_class(&package, typ, &prefix),
                    generated_header: true,
                });
            }
        }

        // 5. Enums
        for enum_def in &api.enums {
            // Skip enums that gen_visitor handles with richer visitor-specific versions
            if has_visitor_pattern && enum_def.name == "VisitResult" {
                continue;
            }
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.java", enum_def.name)),
                content: gen_enum_class(&package, enum_def),
                generated_header: true,
            });
        }

        // 6. Error exception classes
        for error in &api.errors {
            for (class_name, content) in alef_codegen::error_gen::gen_java_error_types(error, &package) {
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", class_name)),
                    content,
                    generated_header: true,
                });
            }
        }

        // 7. Visitor support files (only when ConversionOptions/ConversionResult types exist)
        if has_visitor_pattern {
            for (filename, content) in crate::gen_visitor::gen_visitor_files(&package, &main_class) {
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content,
                    generated_header: false, // already has header comment
                });
            }
        }

        // 8. Trait bridge plugin registration files
        // Emits two files per trait: I{Trait}.java (managed interface) and
        // {Trait}Bridge.java (Panama upcall stubs + register/unregister helpers).
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.contains(&Language::Java.to_string()) {
                continue;
            }

            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name && t.is_trait) {
                let has_super_trait = bridge_cfg.super_trait.is_some();
                let trait_bridge::BridgeFiles {
                    interface_content,
                    bridge_content,
                } = trait_bridge::gen_trait_bridge_files(trait_def, &prefix, &package, has_super_trait);

                files.push(GeneratedFile {
                    path: base_path.join(format!("I{}.java", trait_def.name)),
                    content: interface_content,
                    generated_header: true,
                });
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}Bridge.java", trait_def.name)),
                    content: bridge_content,
                    generated_header: true,
                });
            }
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Java)?;

        Ok(files)
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = resolve_output_dir(
            config.output.java.as_ref(),
            &config.crate_config.name,
            "packages/java/src/main/java/",
        );

        // If output_dir already ends with the package path (user configured the full path),
        // use it as-is. Otherwise, append the package path.
        let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
            PathBuf::from(&output_dir)
        } else {
            PathBuf::from(&output_dir).join(&package_path)
        };

        // Collect bridge param names/aliases to strip from the public facade.
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

        // Generate a high-level public API class that wraps the raw FFI class.
        // Class name = main_class without "Rs" suffix (e.g., HtmlToMarkdownRs -> HtmlToMarkdown)
        let public_class = main_class.trim_end_matches("Rs").to_string();
        let facade_content = gen_facade_class(
            api,
            &package,
            &public_class,
            &main_class,
            &prefix,
            &bridge_param_names,
            &bridge_type_aliases,
        );

        Ok(vec![GeneratedFile {
            path: base_path.join(format!("{}.java", public_class)),
            content: facade_content,
            generated_header: true,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "mvn",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}
