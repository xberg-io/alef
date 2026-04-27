//! Kotlin JVM binding generator — orchestration and `KotlinBackend` impl.
//!
//! The `KotlinBackend` struct implements [`Backend`] and dispatches to the
//! appropriate target-specific emitter based on the configured [`KotlinTarget`].

mod helpers;
mod object_wrapper;
mod shared;
mod traits;
mod typealiases;

use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, KotlinTarget, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;
use std::path::PathBuf;

// Re-export shared utilities used by gen_native and gen_mpp.
pub(crate) use shared::{kotlin_field_name, to_lower_camel, to_pascal_case, to_screaming_snake};

// Re-export emitters used by gen_mpp.
pub(crate) fn emit_type_pub(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_type_with_imports(ty, out, imports)
}

pub(crate) fn emit_enum_pub(en: &EnumDef, out: &mut String) {
    object_wrapper::emit_enum(en, out)
}

pub(crate) fn emit_error_type_pub(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_error_type_with_imports(error, out, imports)
}

/// Format a function parameter with its Kotlin type, collecting any needed imports.
pub(crate) fn format_param_pub(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    object_wrapper::format_param_with_imports(p, imports)
}

/// Render a Kotlin type reference, collecting any needed imports.
pub(crate) fn kotlin_type_str_pub(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<String>) -> String {
    object_wrapper::kotlin_type_with_string_imports(ty, optional, imports)
}

/// Emit a JVM function body (delegates to Bridge) inside an `object` block.
pub(crate) fn emit_function_jvm(
    f: &FunctionDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    java_package: &str,
) {
    object_wrapper::emit_function(f, out, imports, java_package)
}

// ---------------------------------------------------------------------------
// KotlinBackend
// ---------------------------------------------------------------------------

const BRIDGE_ALIAS: &str = "Bridge";

pub struct KotlinBackend;

impl Backend for KotlinBackend {
    fn name(&self) -> &str {
        "kotlin"
    }

    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: false,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        match config.kotlin_target() {
            KotlinTarget::Jvm => generate_jvm(api, config),
            KotlinTarget::Native => crate::gen_native::emit(api, config),
            KotlinTarget::Multiplatform => crate::gen_mpp::emit(api, config),
        }
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// JVM code generation
// ---------------------------------------------------------------------------

fn generate_jvm(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let java_package = config.java_package();
    let module_name = to_pascal_case(&config.crate_config.name);
    // If the user's Kotlin and Java packages collide on the same FQN as the
    // generated module, the Kotlin object would shadow the Java facade class
    // (both compile to `<package>/<module>.class`). Push the Kotlin code into
    // a `.kt` sub-package in that case so the Java class remains importable.
    let configured_kotlin_package = config.kotlin_package();
    let package = if configured_kotlin_package == java_package {
        format!("{configured_kotlin_package}.kt")
    } else {
        configured_kotlin_package
    };

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let exclude_types: std::collections::HashSet<&str> = config
        .kotlin
        .as_ref()
        .map(|c| c.exclude_types.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let configured_trait_bridges: std::collections::HashSet<&str> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.contains(&"kotlin".to_string()))
        .map(|b| b.trait_name.as_str())
        .collect();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    typealiases::emit_typealiases(api, &java_package, &exclude_types, &configured_trait_bridges, &mut body);

    // Functions whose signature involves a trait type are excluded — the Java
    // facade handles trait registration via a separate trait-bridge interface
    // that we don't expose through the Kotlin wrapper. Trait registry helpers
    // (register_*, unregister_*, list_*, clear_*) follow the same pattern.
    let trait_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_trait)
        .map(|t| t.name.as_str())
        .collect();
    let function_uses_trait = |f: &FunctionDef| -> bool {
        // Only skip functions that take a trait type as a parameter (those
        // need the Java facade's trait-bridge wrapper, which can't be reached
        // via a typealias-only Kotlin shim). Functions returning trait types
        // or trait registry helpers (register_*, list_*, clear_*) flow through
        // the Java facade unchanged.
        f.params.iter().any(|p| traits::type_ref_uses_named(&p.ty, &trait_type_names))
    };

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()) && !function_uses_trait(f))
        .collect();

    if !visible_functions.is_empty() {
        // Import the Java facade class with an alias so it does not collide with the
        // Kotlin object that wraps it (both share the PascalCase crate name).
        imports.insert(format!("import {java_package}.{module_name} as {BRIDGE_ALIAS}"));
        if visible_functions.iter().any(|f| f.is_async) {
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
        }

        body.push_str(&format!("object {module_name} {{\n"));
        for f in &visible_functions {
            object_wrapper::emit_function(f, &mut body, &mut imports, &java_package);
            body.push('\n');
        }
        body.push_str("}\n");
    }

    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n\n");
    content.push_str(&format!("package {package}\n\n"));
    for import in &imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(&body);

    let package_path = package.replace('.', "/");
    let dir = resolve_output_dir(
        None,
        &config.crate_config.name,
        &format!("packages/kotlin/src/main/kotlin/{package_path}"),
    );
    let path = PathBuf::from(dir).join(format!("{module_name}.kt"));

    Ok(vec![GeneratedFile {
        path,
        content,
        generated_header: false,
    }])
}
