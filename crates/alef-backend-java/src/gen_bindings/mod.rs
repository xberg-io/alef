use ahash::AHashSet;
use alef_codegen::naming::{to_class_name, to_java_name};
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::ApiSurface;
use std::collections::HashSet;
use std::path::PathBuf;

mod functions;
mod helpers;
mod marshal;
mod types;

use functions::{gen_facade_class, gen_main_class, gen_native_lib};
use helpers::gen_exception_class;
use types::{gen_builder_class, gen_enum_class, gen_opaque_handle_class, gen_record_type};

/// Names that conflict with methods on `java.lang.Object` and are therefore
/// illegal as record component names or method names in generated Java code.
const JAVA_OBJECT_METHOD_NAMES: &[&str] = &[
    "wait",
    "notify",
    "notifyAll",
    "getClass",
    "hashCode",
    "equals",
    "toString",
    "clone",
    "finalize",
];

/// Returns true if `name` is a tuple/unnamed field index such as `"0"`, `"1"`, `"_0"`, `"_1"`.
/// Serde represents tuple and newtype variant fields with these numeric names. They are not
/// real JSON keys and must not be used as Java identifiers.
/// Escape a string for use inside a Javadoc comment.
/// Replaces `*/` (which would close the comment) and `@` (which starts a tag).
pub(crate) fn escape_javadoc_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut code = String::new();
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                code.push(c);
            }
            result.push_str("{@code ");
            result.push_str(&code);
            result.push('}');
        } else if ch == '<' {
            result.push_str("&lt;");
        } else if ch == '>' {
            result.push_str("&gt;");
        } else if ch == '&' {
            result.push_str("&amp;");
        } else if ch == '*' && chars.peek() == Some(&'/') {
            chars.next();
            result.push_str("* /");
        } else if ch == '@' {
            result.push_str("{@literal @}");
        } else {
            result.push(ch);
        }
    }
    result
}

pub(crate) fn is_tuple_field_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
}

/// Sanitise a field/parameter name that would conflict with `java.lang.Object`
/// methods.  Conflicting names get a `_` suffix (e.g. `wait` -> `wait_`), which
/// is then converted to camelCase by `to_java_name`.
pub(crate) fn safe_java_field_name(name: &str) -> String {
    let java_name = to_java_name(name);
    if JAVA_OBJECT_METHOD_NAMES.contains(&java_name.as_str()) {
        format!("{}Value", java_name)
    } else {
        java_name
    }
}

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

        let base_path = PathBuf::from(&output_dir).join(&package_path);

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
        let has_visitor_bridge = !config.trait_bridges.is_empty();

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
            content: gen_native_lib(api, config, &package, &prefix, has_visitor_bridge),
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
                has_visitor_bridge,
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
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !typ.is_opaque && !typ.fields.is_empty() {
                // Skip types that gen_visitor handles with richer visitor-specific versions
                if has_visitor_bridge && (typ.name == "NodeContext" || typ.name == "VisitResult") {
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
            .filter(|t| !t.is_opaque && !t.fields.is_empty() && t.has_default)
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
            if has_visitor_bridge && enum_def.name == "VisitResult" {
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

        // 7. Visitor support files (when a trait bridge is configured)
        if has_visitor_bridge {
            for (filename, content) in crate::gen_visitor::gen_visitor_files(&package, &main_class) {
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content,
                    generated_header: false, // already has header comment
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

        let base_path = PathBuf::from(&output_dir).join(&package_path);

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
        let has_visitor_bridge = !config.trait_bridges.is_empty();

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
            has_visitor_bridge,
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
            depends_on_ffi: true,
            post_build: vec![],
        })
    }
}
