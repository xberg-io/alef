//! Kotlin source emitter for the AAR module.
//!
//! Adapted from the legacy `alef-backend-kotlin::gen_android` module with two
//! corrections relative to the original:
//!
//! 1. The wrapper class is emitted at `<pkg_path>/<Module>.kt` rather than
//!    being split between two diverging directories (the historical
//!    backend/scaffold drift that produced `<pkg>/android/<Module>Android.kt`).
//! 2. A top-of-object `init { System.loadLibrary(...) }` block loads the
//!    bundled native cdylib so consumers do not need to call
//!    `System.loadLibrary` themselves.
//!
//! Kotlin-side type declarations (data classes, enums, sealed errors) are
//! intentionally NOT re-emitted. The bundled Java facade at
//! `src/main/java/<java_pkg>/*.java` already declares every value type as a
//! Java record, every enum as a Java enum, and every error as a Java
//! checked exception. Kotlin and Java share the same FQN package layout in
//! the AAR — re-emitting Kotlin twins would trigger duplicate-declaration
//! errors at `compileKotlin` time. Kotlin code references those Java types
//! directly thanks to Kotlin/Java interop.

use std::collections::BTreeSet;
use std::path::PathBuf;

use alef_backend_kotlin::{emit_function_jvm, emit_jvm_client_class_with_package, to_pascal_case};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::{ApiSurface, TypeRef};

use crate::naming::{java_package, kotlin_package};

/// Emit `<kotlin_source_dir>/<Module>.kt` and (when the API has methodful
/// types) `<kotlin_source_dir>/DefaultClient.kt`.
///
/// `kotlin_source_dir` is the resolved Kotlin source destination —
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/` in the Gradle
/// Android source-set layout.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, kotlin_source_dir: &std::path::Path) -> Vec<GeneratedFile> {
    let package = kotlin_package(config);
    let module_name = to_pascal_case(&config.name);
    let java_pkg = java_package(config);
    let lib_name = config.ffi_lib_name();
    // The Java backend names its raw FFI class `<Module>Rs` (see
    // `alef-backend-java::JavaBackend::resolve_main_class`). When the Kotlin
    // `object` wrapper happens to share its simple name with the Java facade
    // (both `Kreuzberg`), aliasing `Bridge` to the Java class via its short
    // name would resolve back to the Kotlin object itself (Kotlin/Java co-
    // located in the same package), producing infinite recursion or
    // unresolved members. Alias `Bridge` to the `Rs`-suffixed Java class so
    // the Kotlin wrapper delegates to the JNI layer, never to itself.
    let java_facade_class = if module_name.ends_with("Rs") {
        module_name.clone()
    } else {
        format!("{module_name}Rs")
    };

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    // Always emit a {Module} object — even when the API has zero free
    // functions — so the System.loadLibrary call happens on first class load.
    imports.insert(format!("import {java_pkg}.{java_facade_class} as Bridge"));
    // The Kotlin facade lives in `<kotlin_android.package>` (e.g.
    // `dev.kreuzberg.kreuzcrawl.android`) while the bundled Java DTOs
    // (`CrawlConfig`, `ScrapeResult`, …) live in the parent Java package
    // (e.g. `dev.kreuzberg.kreuzcrawl`). Kotlin sub-packages do NOT inherit
    // their parent's symbols, so without explicit imports every bare type
    // reference in a method signature is unresolved. Walk every visible
    // function signature, collect every `TypeRef::Named`, and emit one
    // explicit import per type — ktlint disallows wildcard imports under
    // `standard:no-wildcard-imports`.
    if java_pkg != package {
        let mut named_types: BTreeSet<String> = BTreeSet::new();
        for f in &visible_functions {
            collect_named_types(&f.return_type, &mut named_types);
            for p in &f.params {
                collect_named_types(&p.ty, &mut named_types);
            }
        }
        for ty in &named_types {
            imports.insert(format!("import {java_pkg}.{ty}"));
        }
    }
    if visible_functions.iter().any(|f| f.is_async) {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }
    body.push_str(&format!("object {module_name} {{\n"));
    body.push_str(&format!("    init {{ System.loadLibrary(\"{lib_name}\") }}\n\n"));
    for f in &visible_functions {
        emit_function_jvm(f, &mut body, &mut imports, &java_pkg);
        body.push('\n');
    }
    body.push_str("}\n");

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

    let kt_path = kotlin_source_dir.join(format!("{module_name}.kt"));

    let mut files = vec![GeneratedFile {
        path: kt_path,
        content,
        generated_header: false,
    }];

    // Emit the coroutine-friendly DefaultClient.kt with the kotlin_android
    // package override so it lands at `<kotlin_source_dir>/DefaultClient.kt`
    // with `package <kotlin_android.package>` instead of falling back to the
    // generic `config.kotlin_package()` accessor (which derives a
    // `com.github.<org>` placeholder when no `[crates.kotlin] package` is
    // configured).
    if let Some(client_file) = emit_jvm_client_class_with_package(api, config, Some(&package)) {
        let android_client_path = kotlin_source_dir.join("DefaultClient.kt");
        files.push(GeneratedFile {
            path: android_client_path,
            content: client_file.content,
            generated_header: false,
        });
    }

    let _ = PathBuf::new(); // suppress unused import on some toolchains
    files
}

/// Walk a `TypeRef` and collect every `TypeRef::Named` simple-name into
/// `out`. Used to derive explicit per-type Kotlin imports for the bundled
/// Java DTO package when the Kotlin facade lives in a sub-package
/// (e.g. `dev.kreuzberg.kreuzcrawl.android` vs `dev.kreuzberg.kreuzcrawl`).
fn collect_named_types(ty: &TypeRef, out: &mut BTreeSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}
