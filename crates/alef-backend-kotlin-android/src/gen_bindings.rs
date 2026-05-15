//! Kotlin source emitter for the AAR module (JNI mode).
//!
//! Produces a pure-Kotlin JNI layout with no bundled Java facade:
//!
//! - `<Module>Bridge.kt` — a Kotlin `object` with `external fun` JNI
//!   declarations and `init { System.loadLibrary("<crate>_jni") }`.
//! - `DefaultClient.kt` — coroutine-friendly client class holding a `Long`
//!   handle when the API has methodful types.
//! - Data classes, enums, and error types are emitted as `.kt` files via the
//!   kotlin backend's pub helpers.
//!
//! `KotlinFfiStyle::Jni` is forced by the parent backend (`lib.rs`) before
//! this module is called, so `config.kotlin_ffi_style()` will always return
//! `KotlinFfiStyle::Jni` here.

use std::collections::BTreeSet;
use std::path::Path;

use alef_backend_kotlin::{emit_enum_pub, emit_error_type_pub, emit_jni_bridge_object, emit_jni_client_class, emit_type_pub, to_pascal_case};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;

use crate::naming::kotlin_package;

/// Emit all Kotlin source files for the AAR module.
///
/// `kotlin_source_dir` is the resolved Kotlin source destination —
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/` in the Gradle
/// Android source-set layout.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, kotlin_source_dir: &Path) -> Vec<GeneratedFile> {
    let package = kotlin_package(config);
    let mut files = Vec::new();

    // Bridge object: external fun declarations + System.loadLibrary init block.
    let mut bridge_file = emit_jni_bridge_object(api, config);
    bridge_file.path = kotlin_source_dir.join(
        bridge_file
            .path
            .file_name()
            .expect("bridge file must have a filename"),
    );
    files.push(bridge_file);

    // DefaultClient.kt — only emitted when the API has methodful opaque types.
    if let Some(mut client_file) = emit_jni_client_class(api, config, Some(&package)) {
        client_file.path = kotlin_source_dir.join("DefaultClient.kt");
        files.push(client_file);
    }

    // Data classes, enums, and error types as pure Kotlin.
    for ty in &api.types {
        if ty.is_opaque || ty.is_trait {
            continue;
        }
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();
        emit_type_pub(ty, &mut body, &mut imports);
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", ty.name)),
            content,
            generated_header: false,
        });
    }

    for en in &api.enums {
        let mut body = String::new();
        emit_enum_pub(en, &mut body);
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &BTreeSet::new(), &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", en.name)),
            content,
            generated_header: false,
        });
    }

    for error in &api.errors {
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();
        emit_error_type_pub(error, &mut body, &mut imports);
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", error.name)),
            content,
            generated_header: false,
        });
    }

    // Emit the free-function facade object (Module.kt) when visible functions exist.
    emit_module_kt(api, config, kotlin_source_dir, &package, &mut files);

    files
}

/// Emit `<Module>.kt` — a Kotlin `object` that re-exposes every free function
/// by delegating to `<Module>Bridge`. This preserves the ergonomic call-site
/// pattern `Module.foo(...)` while keeping all JNI declarations in the Bridge.
fn emit_module_kt(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    use alef_backend_kotlin::to_lower_camel;

    let module_name = to_pascal_case(&config.name);
    let bridge_name = format!("{module_name}Bridge");

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    if visible_functions.is_empty() {
        return;
    }

    let imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    body.push_str(&format!("object {module_name} {{\n"));
    for f in &visible_functions {
        let method_name = to_lower_camel(&f.name);
        let native_name = format!("native{}", to_pascal_case(&f.name));
        let return_ty = jni_return_type_str(&f.return_type);
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let ty = jni_param_type_str(&p.ty);
                format!("{}: {ty}", to_lower_camel(&p.name))
            })
            .collect();
        let args: Vec<String> = f.params.iter().map(|p| to_lower_camel(&p.name)).collect();
        body.push_str(&format!(
            "    fun {method_name}({}): {return_ty} = {bridge_name}.{native_name}({})\n",
            params.join(", "),
            args.join(", ")
        ));
    }
    body.push_str("}\n");

    let content = assemble_kt_content(package, &imports, &body);
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{module_name}.kt")),
        content,
        generated_header: false,
    });
}

/// Assemble a complete `.kt` file from package, imports, and body.
fn assemble_kt_content(package: &str, imports: &BTreeSet<String>, body: &str) -> String {
    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n\n");
    content.push_str(&format!("package {package}\n\n"));
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

/// Map a `TypeRef` to a JNI return type string for the delegate wrapper.
fn jni_return_type_str(ty: &alef_core::ir::TypeRef) -> &'static str {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Unit => "Unit",
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean",
            PrimitiveType::I8 => "Byte",
            PrimitiveType::I16 => "Short",
            PrimitiveType::I32 => "Int",
            PrimitiveType::I64 => "Long",
            PrimitiveType::U8 => "Byte",
            PrimitiveType::U16 => "Short",
            PrimitiveType::U32 => "Int",
            PrimitiveType::U64 => "Long",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
            PrimitiveType::Usize | PrimitiveType::Isize => "Long",
        },
        TypeRef::String => "String",
        TypeRef::Optional(_) => "String?",
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => "String",
        _ => "Long",
    }
}

/// Map a `TypeRef` to a JNI parameter type string for the delegate wrapper.
fn jni_param_type_str(ty: &alef_core::ir::TypeRef) -> &'static str {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean",
            PrimitiveType::I8 => "Byte",
            PrimitiveType::I16 => "Short",
            PrimitiveType::I32 => "Int",
            PrimitiveType::I64 => "Long",
            PrimitiveType::U8 => "Byte",
            PrimitiveType::U16 => "Short",
            PrimitiveType::U32 => "Int",
            PrimitiveType::U64 => "Long",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
            PrimitiveType::Usize | PrimitiveType::Isize => "Long",
        },
        TypeRef::String => "String",
        _ => "String",
    }
}
