use crate::codegen::naming::to_class_name;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{BridgeBinding, JavaBuilderMode, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;
use std::path::PathBuf;

mod facade;
mod ffi_class;
pub mod helpers;
mod line_wrap;
mod marshal;
mod native_lib;
mod service_api;
pub mod trait_bridge;
mod types;

use facade::gen_facade_class;
use ffi_class::gen_main_class;
use helpers::{gen_exception_class, gen_infrastructure_exception_class, gen_json_util_class};
use native_lib::gen_native_lib;
use types::{gen_byte_array_serializer, gen_enum_class, gen_opaque_handle_class, gen_record_type};

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

fn effective_exclude_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(java) = &config.java {
        exclude_types.extend(java.exclude_types.iter().cloned());
    }
    // Also exclude types flagged binding_excluded by the service extraction pass
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    // Mirror the FFI backend's `contains('<')` filter for workspace-declared opaque types
    // with generic-parameter rust_paths — the FFI backend skips `_new`/`_free` symbols for
    // them, so Java (Panama/JNI) downcalls would link against missing symbols.
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
        // Do NOT filter trait methods — they will use type substitution in trait bridge generation.
        // Only filter methods on non-trait types.
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
        // Java is a single compiled surface with no Rust-cfg gating at the Java-source level, so
        // same-named cfg-variant functions (real impl + no-ORT stub fallback) must collapse to a
        // single method to avoid `duplicate method` javac errors. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;
        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = config
            .output_for("java")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

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
        let has_visitor_pattern = crate::backends::java::gen_visitor::has_visitor_generation_metadata(api, config);
        let mut files = Vec::new();

        // 0. package-info.java - required by Checkstyle
        let description = config
            .scaffold
            .as_ref()
            .and_then(|s| s.description.as_deref())
            .unwrap_or("Generated Java bindings.");
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

        // 3b. Infrastructure exception classes for FFI error codes 1 and 2.
        // These are always emitted because checkLastError() hardcodes:
        //   case 1 -> throw new InvalidInputException(msg);
        //   case 2 -> throw new ConversionErrorException(msg);
        // Code 1 = null pointer / invalid UTF-8 in an input arg (invalid input).
        // Code 2 = JSON serialisation/deserialisation failure (type conversion).
        for (class_name, code, doc) in [
            (
                "InvalidInputException",
                1i32,
                "Exception thrown when input validation fails.",
            ),
            (
                "ConversionErrorException",
                2i32,
                "Exception thrown when type conversion fails.",
            ),
        ] {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.java", class_name)),
                content: gen_infrastructure_exception_class(&package, &main_class, class_name, code, doc),
                generated_header: true,
            });
        }

        // Build a map of enum names to their default variant metadata.
        // This is used when a struct field has #[serde(default)] and the field type is an enum.
        // The metadata includes whether the variant has zero fields, which Java needs to determine
        // whether to emit `new EnumName.Variant()` (record/sealed interface) or `EnumName.Variant` (static).
        let enum_defaults = crate::extract::default_value_for_enum::enum_default_variants_map_with_metadata(api);

        // Untagged unions with data variants now emit as JsonNode-wrapper classes
        // (see gen_java_untagged_wrapper). The set is intentionally empty so that
        // record fields keep their wrapper type instead of being downcast to Object.
        let complex_enums: AHashSet<String> = AHashSet::new();

        // Collect sealed union types with unwrapped/tuple variants that need custom deserializers.
        // When a record field references one of these types, we need to add a @JsonDeserialize
        // annotation to the field so Jackson uses the custom deserializer.
        let sealed_unions_with_unwrapped: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                e.serde_tag.is_some()
                    && e.variants
                        .iter()
                        .any(|v| v.fields.len() == 1 && helpers::is_tuple_field_name(&v.fields[0].name))
            })
            .map(|e| e.name.clone())
            .collect();

        // Collect sealed interface names. These are enums with serde tagging (internally tagged)
        // which generate as sealed interfaces with nested record variants in Java,
        // requiring `new EnumName.Variant()` syntax for instantiation.
        // Traditional enums (no serde tagging) use static references like `EnumName.Variant`.
        let sealed_interface_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_some())
            .map(|e| e.name.clone())
            .collect();

        // Resolve language-level serde rename strategy (always wins over IR type-level).
        let lang_rename_all = config.serde_rename_all_for_language(Language::Java);

        // Compute visible type names early. These are struct + enum names that get a generated
        // companion Java class. Types not in this set are JSON-bridged to JsonNode.
        let visible_type_names: HashSet<&str> = api
            .types
            .iter()
            .filter(|t| !t.is_trait)
            .map(|t| t.name.as_str())
            .chain(api.enums.iter().map(|e| e.name.as_str()))
            .collect();

        // 4. Record types
        // Include non-opaque types that either have fields OR are serializable unit structs
        // (has_serde + has_default, empty fields). Unit structs like `ExcelMetadata` need a
        // concrete Java class so they can be referenced as record components in tagged-union
        // variant records (e.g. FormatMetadata.Excel(@JsonUnwrapped ExcelMetadata value)).
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            let is_unit_serde = !typ.is_opaque && typ.fields.is_empty() && typ.has_serde;
            if !typ.is_opaque && (!typ.fields.is_empty() || is_unit_serde) {
                // The visitor context remains a normal IR-derived record; only the result enum
                // and bridge/interface files are visitor-specific.
                let builder_mode = config
                    .java
                    .as_ref()
                    .map(|j| j.dto.builder)
                    .unwrap_or(JavaBuilderMode::Auto);
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_record_type(
                        &package,
                        typ,
                        &complex_enums,
                        &sealed_unions_with_unwrapped,
                        &lang_rename_all,
                        &config.trait_bridges,
                        &main_class,
                        builder_mode,
                        &enum_defaults,
                        &sealed_interface_names,
                        &visible_type_names,
                    ),
                    generated_header: true,
                });
                // The builder is now emitted as a nested static class inside the record file —
                // no separate *Builder.java file is created.
            }
        }

        // 4a. Utility serializer for byte[] → JSON int-array (needed when any record
        // has a non-optional Bytes field). Jackson's default byte[] serialiser emits
        // base64, which Rust's serde Vec<u8> cannot accept. Emit the class once.
        let needs_bytes_serializer = api
            .types
            .iter()
            .any(|t| !t.is_opaque && t.fields.iter().any(|f| !f.optional && matches!(f.ty, TypeRef::Bytes)));
        if needs_bytes_serializer {
            files.push(GeneratedFile {
                path: base_path.join("ByteArrayToIntArraySerializer.java"),
                content: gen_byte_array_serializer(&package),
                generated_header: true,
            });
        }

        // 4a. JsonUtil class for centralized JSON deserialization
        files.push(GeneratedFile {
            path: base_path.join("JsonUtil.java"),
            content: gen_json_util_class(&package, &main_class),
            generated_header: true,
        });

        // 4b. Opaque handle types
        let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
        // Build type classification sets for gating _to_json, _from_json, and _default handle references
        let opaque_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        let to_json_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| {
                !t.is_opaque
                    && t.has_serde
                    && !t.name.ends_with("Update")
                    && !t.methods.iter().any(|m| m.name == "to_json")
            })
            .map(|t| t.name.clone())
            .collect();
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_opaque_handle_class(
                        &package,
                        typ,
                        &prefix,
                        &config.adapters,
                        &main_class,
                        &enum_names,
                        &opaque_type_names,
                        &to_json_type_names,
                    ),
                    generated_header: true,
                });
            }
        }

        // 5. Enums
        for enum_def in &api.enums {
            // Skip enums that gen_visitor handles with richer visitor-specific versions
            if has_visitor_pattern
                && config
                    .trait_bridges
                    .iter()
                    .any(|bridge| bridge.result_type.as_deref() == Some(enum_def.name.as_str()))
            {
                continue;
            }
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.java", enum_def.name)),
                content: gen_enum_class(&package, enum_def, &main_class),
                generated_header: true,
            });
        }

        // 6. Error exception classes
        //
        // Filter out variants whose generated class name collides with the FFI infrastructure
        // exceptions emitted at step 3b. Both paths target the same .java file; without this
        // filter, the gen_java_error_types content was overwriting (or worse, mangling — the
        // InvalidInputException file ended up with a duplicate constructor block appended
        // after the closing brace) the canonical infrastructure-emitted class.
        let infrastructure_exception_names: AHashSet<&str> = ["InvalidInputException", "ConversionErrorException"]
            .into_iter()
            .collect();
        let mut emitted_exception_names: AHashSet<String> = AHashSet::new();
        for error in &api.errors {
            for (class_name, content) in crate::codegen::error_gen::gen_java_error_types(error, &package) {
                if infrastructure_exception_names.contains(class_name.as_str()) {
                    continue;
                }
                if !emitted_exception_names.insert(class_name.clone()) {
                    continue;
                }
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", class_name)),
                    content,
                    generated_header: true,
                });
            }
        }

        // 7. Visitor support files (only when compatible trait-bridge metadata exists)
        if has_visitor_pattern {
            for (filename, content) in
                crate::backends::java::gen_visitor::gen_visitor_files(api, config, &package, &main_class)
                    .unwrap_or_default()
            {
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content,
                    generated_header: false, // already has header comment
                });
            }
        }

        // 8. Trait bridge plugin registration files
        // Emits four files per trait:
        // - I{Trait}.java (managed interface)
        // - {Trait}Bridge.java (Panama upcall stubs + register/unregister helpers)
        // - {Trait}Adapter.java (Path A wrapper implementing the interface and delegating to user impl)
        //
        // visible_type_names was computed earlier for use with record types.
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.contains(&Language::Java.to_string()) {
                continue;
            }

            // When visitor_callbacks is active, visitor traits bound via options_field are
            // surfaced through Visitor.java + VisitorBridge.java (generated by gen_visitor_files).
            // The raw trait bridge I{Trait}.java emitted here would be an unreferenced orphan
            // with snake_case method names. Suppress it for options_field-bound visitor traits.
            if has_visitor_pattern && bridge_cfg.bind_via == BridgeBinding::OptionsField {
                continue;
            }

            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name && t.is_trait) {
                let has_super_trait = bridge_cfg.super_trait.is_some();
                let trait_bridge::BridgeFiles {
                    interface_content,
                    bridge_content,
                } = trait_bridge::gen_trait_bridge_files(
                    trait_def,
                    &prefix,
                    &package,
                    has_super_trait,
                    bridge_cfg.unregister_fn.as_deref(),
                    bridge_cfg.clear_fn.as_deref(),
                    &visible_type_names,
                    &exclude_types,
                    &bridge_cfg.ffi_skip_methods,
                );

                // Path A: Generate adapter bridge wrapper
                let adapter_content = trait_bridge::gen_trait_adapter_bridge_file(
                    trait_def,
                    &package,
                    &visible_type_names,
                    &exclude_types,
                    &bridge_cfg.ffi_skip_methods,
                );

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
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}Adapter.java", trait_def.name)),
                    content: adapter_content,
                    generated_header: true,
                });
            }
        }

        // Apply downstream Checkstyle line-length wrapping to every generated
        // Java source. The templates emit some compound statements on one line;
        // this pass splits at logical points (annotation lists, call args,
        // method signatures) without changing semantics.
        for file in &mut files {
            file.content = line_wrap::wrap_long_java_lines(&file.content);
        }

        Ok(files)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // The public Java facade is a single compiled surface, so same-named cfg-variant
        // functions must collapse to a single method to avoid `duplicate method` javac
        // errors. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = config
            .output_for("java")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

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
        let has_visitor_pattern = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false)
            || config
                .trait_bridges
                .iter()
                .any(|b| b.bind_via == BridgeBinding::OptionsField);
        // Generate a high-level public API class that wraps the raw FFI class.
        // Class name = main_class without "Rs" suffix (e.g., SampleMarkdownRs -> SampleMarkdown)
        let public_class = main_class.trim_end_matches("Rs").to_string();
        let facade_content = gen_facade_class(
            api,
            &package,
            &public_class,
            &main_class,
            &prefix,
            &bridge_param_names,
            &bridge_type_aliases,
            has_visitor_pattern,
            config,
        );

        Ok(vec![GeneratedFile {
            path: base_path.join(format!("{}.java", public_class)),
            content: line_wrap::wrap_long_java_lines(&facade_content),
            generated_header: true,
        }])
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
            tool: "mvn",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}
