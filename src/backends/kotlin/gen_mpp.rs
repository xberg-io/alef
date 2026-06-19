//! Kotlin Multiplatform (KMP) binding generator — Phase 3.
//!
//! Emits a KMP project layout under `packages/kotlin-mpp/`:
//!
//! - `src/commonMain/kotlin/<package>/<Module>.kt`  — shared DTOs + `expect object` declarations
//! - `src/jvmMain/kotlin/<package>/<Module>.kt`     — `actual object` delegating to the JVM facade
//! - `src/nativeMain/kotlin/<package>/<Module>.kt`  — `actual object` using `kotlinx.cinterop.*`
//! - `<crate>.def`                                  — cinterop definition (same as Native target)
//! - `build.gradle.kts`                             — KMP project build script

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FunctionDef};
use crate::core::template_versions;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::backends::kotlin::gen_bindings::{
    emit_enum_pub, emit_error_type_pub, emit_function_jvm, emit_type_pub, format_param_pub, kotlin_type_str_pub,
    to_lower_camel, to_pascal_case,
};
use crate::backends::kotlin::gen_native::emit_native_function_pub;

const BRIDGE_ALIAS: &str = "Bridge";

/// Emit all Kotlin Multiplatform files for the given API surface.
///
/// Returns five generated files:
/// 1. `packages/kotlin-mpp/src/commonMain/kotlin/<package>/<Module>.kt`
/// 2. `packages/kotlin-mpp/src/jvmMain/kotlin/<package>/<Module>.kt`
/// 3. `packages/kotlin-mpp/src/nativeMain/kotlin/<package>/<Module>.kt`
/// 4. `packages/kotlin-mpp/<crate>.def`
/// 5. `packages/kotlin-mpp/build.gradle.kts`
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let package = config.kotlin_package();
    let package_path = package.replace('.', "/");
    let module_name = to_pascal_case(&config.name);
    let crate_name = &config.name;

    let mpp_root = "packages/kotlin-mpp".to_string();

    let common_kt_path = PathBuf::from(&mpp_root)
        .join("src/commonMain/kotlin")
        .join(&package_path)
        .join(format!("{module_name}.kt"));

    let jvm_kt_path = PathBuf::from(&mpp_root)
        .join("src/jvmMain/kotlin")
        .join(&package_path)
        .join(format!("{module_name}.kt"));

    let native_kt_path = PathBuf::from(&mpp_root)
        .join("src/nativeMain/kotlin")
        .join(&package_path)
        .join(format!("{module_name}.kt"));

    let def_path = PathBuf::from(&mpp_root).join(format!("{crate_name}.def"));
    let gradle_path = PathBuf::from(&mpp_root).join("build.gradle.kts");

    Ok(vec![
        GeneratedFile {
            path: common_kt_path,
            content: emit_common(api, config),
            generated_header: false,
        },
        GeneratedFile {
            path: jvm_kt_path,
            content: emit_jvm_actual(api, config),
            generated_header: false,
        },
        GeneratedFile {
            path: native_kt_path,
            content: emit_native_actual(api, config),
            generated_header: false,
        },
        GeneratedFile {
            path: def_path,
            content: emit_def_file(config),
            generated_header: false,
        },
        GeneratedFile {
            path: gradle_path,
            content: emit_gradle_build(config),
            generated_header: false,
        },
    ])
}

// ---------------------------------------------------------------------------
// commonMain — shared DTOs + expect object declarations
// ---------------------------------------------------------------------------

fn emit_common(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = config.kotlin_package();
    let module_name = to_pascal_case(&config.name);

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

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    // DTOs (data classes), enums, and error hierarchies are pure Kotlin — shared in commonMain.
    for ty in api.types.iter().filter(|t| !exclude_types.contains(t.name.as_str())) {
        emit_type_pub(ty, &mut body, &mut imports);
        body.push('\n');
    }

    let text_types = &config.untagged_union_text_types;
    for en in api.enums.iter().filter(|e| !exclude_types.contains(e.name.as_str())) {
        emit_enum_pub(en, &mut body, &package, text_types);
        body.push('\n');
    }

    for error in &api.errors {
        emit_error_type_pub(error, &mut body, &mut imports);
        body.push('\n');
    }

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    // Functions: emit `expect object` with only signatures (no bodies).
    if !visible_functions.is_empty() {
        body.push_str(&crate::backends::kotlin::template_env::render(
            "expect_object_declaration.jinja",
            minijinja::context! {
                name => module_name,
            },
        ));
        for f in &visible_functions {
            emit_expect_function(f, &mut body, &mut imports);
            body.push('\n');
        }
        body.push_str("}\n");
    }

    render_kt_file(&package, &imports, &body)
}

/// Emit a single `expect` function signature (no body).
fn emit_expect_function(f: &FunctionDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !f.doc.is_empty() {
        let doc_lines: Vec<String> = f.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "    ",
                lines => doc_lines,
            },
        ));
    }
    let params: Vec<String> = f.params.iter().map(|p| format_param_pub(p, imports)).collect();
    let return_ty = kotlin_type_str_pub(&f.return_type, false, imports);
    let async_kw = if f.is_async { "suspend " } else { "" };
    let func_name_camel = to_lower_camel(&f.name);
    out.push_str(&crate::backends::kotlin::template_env::render(
        "expect_function_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            name => func_name_camel,
            params => params.join(", "),
            return_type => return_ty,
        },
    ));
}

// ---------------------------------------------------------------------------
// jvmMain — actual object delegating to the JVM Bridge facade
// ---------------------------------------------------------------------------

fn emit_jvm_actual(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = config.kotlin_package();
    let module_name = to_pascal_case(&config.name);
    let java_package = config.java_package();

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    if !visible_functions.is_empty() {
        imports.insert(format!("import {java_package}.{module_name} as {BRIDGE_ALIAS}"));
        if visible_functions.iter().any(|f| f.is_async) {
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.future.await".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
        }

        body.push_str(&crate::backends::kotlin::template_env::render(
            "actual_object_declaration.jinja",
            minijinja::context! {
                name => module_name,
            },
        ));
        for f in &visible_functions {
            emit_function_jvm(f, &mut body, &mut imports, &java_package);
            body.push('\n');
        }
        body.push_str("}\n");
    }

    render_kt_file(&package, &imports, &body)
}

// ---------------------------------------------------------------------------
// nativeMain — actual object using kotlinx.cinterop
// ---------------------------------------------------------------------------

fn emit_native_actual(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = config.kotlin_package();
    let module_name = to_pascal_case(&config.name);
    let prefix = config.ffi_prefix();
    let crate_name = &config.name;

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    imports.insert("import kotlinx.cinterop.*".to_string());
    imports.insert(format!("import {crate_name}.*"));

    let mut body = String::new();

    if !visible_functions.is_empty() {
        body.push_str(&crate::backends::kotlin::template_env::render(
            "actual_object_declaration.jinja",
            minijinja::context! {
                name => module_name,
            },
        ));
        for f in &visible_functions {
            emit_native_function_pub(f, &prefix, &mut body);
            body.push('\n');
        }
        body.push_str("}\n");
    }

    render_kt_file(&package, &imports, &body)
}

// ---------------------------------------------------------------------------
// cinterop .def file (same as Native target)
// ---------------------------------------------------------------------------

fn emit_def_file(config: &ResolvedCrateConfig) -> String {
    let header = config.ffi_header_name();
    let lib_name = config.ffi_lib_name();
    let prefix = config.ffi_prefix();

    format!("headers = {header}\nheaderFilter = {prefix}_*\nlinkerOpts = -L../../../target/release -l{lib_name}\n")
}

// ---------------------------------------------------------------------------
// build.gradle.kts — KMP project
// ---------------------------------------------------------------------------

fn emit_gradle_build(config: &ResolvedCrateConfig) -> String {
    let crate_name = &config.name;
    let kotlin_version = template_versions::maven::KOTLIN_JVM_PLUGIN;

    format!(
        r#"// Generated by alef. Do not edit by hand.

plugins {{
    kotlin("multiplatform") version "{kotlin_version}"
}}

kotlin {{
    jvm()

    linuxX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}

    macosArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}

    iosX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            framework()
        }}
    }}

    iosArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            framework()
        }}
    }}

    iosSimulatorArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            framework()
        }}
    }}

    androidNativeArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}

    sourceSets {{
        val commonMain by getting
        val jvmMain by getting
        val nativeMain by creating {{
            dependsOn(commonMain)
        }}
        val linuxX64Main by getting {{
            dependsOn(nativeMain)
        }}
        val macosArm64Main by getting {{
            dependsOn(nativeMain)
        }}
        val iosMain by creating {{
            dependsOn(nativeMain)
        }}
        val iosX64Main by getting {{
            dependsOn(iosMain)
        }}
        val iosArm64Main by getting {{
            dependsOn(iosMain)
        }}
        val iosSimulatorArm64Main by getting {{
            dependsOn(iosMain)
        }}
        val androidNativeArm64Main by getting {{
            dependsOn(nativeMain)
        }}
    }}
}}
"#
    )
}

// ---------------------------------------------------------------------------
// Shared rendering helper
// ---------------------------------------------------------------------------

fn render_kt_file(package: &str, imports: &BTreeSet<String>, body: &str) -> String {
    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n\n");
    content.push_str(&crate::backends::kotlin::template_env::render(
        "package_declaration.jinja",
        minijinja::context! {
            package => package,
        },
    ));
    content.push_str("\n\n");
    for import in imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(body);
    content
}
