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

mod module_facade;
mod trait_interfaces;

pub use trait_interfaces::format_method_signature;

use std::collections::BTreeSet;
use std::path::Path;

use crate::backends::kotlin::{
    emit_enum_pub, emit_error_type_pub, emit_jni_bridge_object, emit_jni_client_class,
    emit_type_pub_with_defaults_sealed_and_constructible,
};
use crate::backends::kotlin_android::naming::kotlin_package;
use crate::backends::kotlin_android::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::jni::bridge_class_name;

/// Emit all Kotlin source files for the AAR module.
///
/// `kotlin_source_dir` is the resolved Kotlin source destination —
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/` in the Gradle
/// Android source-set layout.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, kotlin_source_dir: &Path) -> Vec<GeneratedFile> {
    let package = kotlin_package(config);
    let mut files = Vec::new();

    // Collect type aliases that should be treated as excluded.
    //
    // Two sources contribute:
    //   1. `kotlin_android.exclude_types` (explicit user override).
    //   2. Trait-bridge `type_alias` values whose `param_name` is listed in
    //      `kotlin_android.exclude_functions`. When the user opts out of the
    //      bridge function (e.g. because the JNI bridge has no trait-handle
    //      implementation yet), the bridge's type alias is still unresolvable
    //      in Kotlin code — its corresponding `options_field` on the bridge's
    //      `options_type` would emit a dangling reference like
    //      `val visitor: VisitorHandle?`. Treat the alias as excluded so the
    //      field is dropped along with the function.
    let kotlin_android_excluded_function_names: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut effective_excluded_types: std::collections::HashSet<String> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.iter().any(|l| l == "kotlin_android") {
            if let Some(alias) = &bridge.type_alias {
                effective_excluded_types.insert(alias.clone());
            }
        }
        if let Some(name) = bridge.param_name.as_deref() {
            if kotlin_android_excluded_function_names.contains(name) {
                if let Some(alias) = &bridge.type_alias {
                    effective_excluded_types.insert(alias.clone());
                }
            }
        }
    }
    // Mirror the FFI backend's `contains('<')` filter for workspace-declared opaque types
    // with generic-parameter rust_paths — the FFI backend skips `_new`/`_free` symbols for
    // them, so Kotlin Android JNI external-fun declarations against those symbols would
    // throw `UnsatisfiedLinkError` at runtime.
    for (name, path) in &config.opaque_types {
        if path.contains('<') {
            effective_excluded_types.insert(name.clone());
        }
    }

    // Build an `enum_name → default_variant` map so the data-class emitter
    // can synthesise constructor defaults for Named enum fields (e.g.
    // `headingStyle: HeadingStyle = HeadingStyle.ATX`). The Jackson Kotlin
    // module rejects deserialization of partial JSON when a
    // non-nullable field has no default — every non-optional Named enum
    // field needs one to round-trip.
    //
    // Only true Kotlin enums are included here. Tagged/untagged enums (with
    // `serde_tag` or `serde_untagged` set) are emitted as sealed classes in
    // Kotlin and are handled differently (variant names are PascalCase, not
    // SCREAMING_SNAKE_CASE).
    //
    // Enums without a declared `#[default]` variant map to an empty string;
    // the emitter treats this as "no synthesisable default" and falls
    // through to the type-based path (null for optional fields, no default
    // for required ones). The mere presence of the entry distinguishes
    // true enums from data-class struct types and sealed classes.
    let enum_defaults: std::collections::HashMap<String, String> = api
        .enums
        .iter()
        .filter(|en| en.serde_tag.is_none() && !en.serde_untagged && en.variants.iter().all(|v| v.fields.is_empty()))
        .map(|en| {
            let default_variant = en
                .variants
                .iter()
                .find(|v| v.is_default)
                .map(|v| v.name.clone())
                .unwrap_or_default();
            (en.name.clone(), default_variant)
        })
        .collect();

    // Build the set of Kotlin sealed-class names — Rust enums emitted as
    // sealed classes because they are tagged (`#[serde(tag = "...")]`) or
    // untagged (`#[serde(untagged)]`).  Data-class fields whose declared
    // type references one of these names need a `@field:JsonSerialize(\`as\` = ...)`
    // (or `contentAs` for collections) annotation so Jackson routes through
    // the sealed class's custom serializer (which emits the discriminator)
    // instead of the runtime variant's default POJO serializer.
    let sealed_class_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|en| en.serde_tag.is_some() || en.serde_untagged)
        .map(|en| en.name.clone())
        .collect();

    // Non-enum, non-trait, non-opaque data class types whose Rust source has a
    // `Default` impl (has_default = true). For fields whose declared type
    // references one of these names, the emitter can safely synthesize
    // `= Name()` as the constructor default — preventing Jackson's Kotlin
    // module from raising `MissingKotlinParameterException` when the wire
    // JSON omits the nested struct (the common shape of partial-update
    // payloads in test fixtures).
    let default_constructible_types: std::collections::HashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_default)
        .map(|t| t.name.clone())
        .collect();

    // Bridge object: external fun declarations + System.loadLibrary init block.
    let mut bridge_file = emit_jni_bridge_object(api, config);
    bridge_file.path = kotlin_source_dir.join(bridge_file.path.file_name().expect("bridge file must have a filename"));
    files.push(bridge_file);

    // BridgeException: a RuntimeException subclass thrown by the JNI shim when
    // a native call fails.  The Rust ERROR_CLASS constant references this class
    // as `<package>/<BridgeName>Exception`.  Without this file the JVM raises
    // NoClassDefFoundError on the first JNI call that needs to propagate an error.
    let bridge = bridge_class_name(&config.name);
    let exception_class = format!("{bridge}Exception");
    let exception_content = format!(
        "// Generated by alef. Do not edit by hand.\n\n\
         package {package}\n\n\
         class {exception_class}(message: String?, cause: Throwable?) : RuntimeException(message, cause) {{\n\
         {}\n\
         }}\n",
        "    constructor(message: String?) : this(message, null)"
    );
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{exception_class}.kt")),
        content: exception_content,
        generated_header: false,
    });

    // DefaultClient.kt — only emitted when the API has methodful opaque types.
    if let Some(mut client_file) = emit_jni_client_class(api, config, Some(&package)) {
        client_file.path = kotlin_source_dir.join("DefaultClient.kt");
        files.push(client_file);
    }

    // Data classes, enums, and error types as pure Kotlin.
    for ty in &api.types {
        if ty.is_opaque || ty.is_trait || ty.binding_excluded {
            continue;
        }
        // Skip whole types whose name is in the effective exclude set
        // (e.g. trait-bridge `VisitorHandle` when the bridge function is
        // excluded — the alias has no Kotlin representation).
        if effective_excluded_types.contains(&ty.name) {
            continue;
        }
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();
        // Drop fields whose type references an excluded alias so the data
        // class definition does not emit a dangling reference. The bridge
        // function is already filtered out of the module facade, so its
        // companion field cannot be set by callers — defaulting it to
        // `Default::default()` Rust-side preserves runtime correctness.
        let needs_field_filter = ty
            .fields
            .iter()
            .any(|f| effective_excluded_types.iter().any(|name| f.ty.references_named(name)));
        if needs_field_filter {
            let mut filtered = ty.clone();
            filtered
                .fields
                .retain(|f| !effective_excluded_types.iter().any(|name| f.ty.references_named(name)));
            emit_type_pub_with_defaults_sealed_and_constructible(
                &filtered,
                &mut body,
                &mut imports,
                &enum_defaults,
                &sealed_class_names,
                &default_constructible_types,
            );
        } else {
            emit_type_pub_with_defaults_sealed_and_constructible(
                ty,
                &mut body,
                &mut imports,
                &enum_defaults,
                &sealed_class_names,
                &default_constructible_types,
            );
        }
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
        if en.binding_excluded {
            continue;
        }
        let mut body = String::new();
        emit_enum_pub(en, &mut body, &package);
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

    trait_interfaces::emit_trait_interfaces(api, config, kotlin_source_dir, &package, &mut files);

    // Emit the free-function facade object (Module.kt) when visible functions exist.
    module_facade::emit_module_kt(api, config, kotlin_source_dir, &package, &mut files);

    files
}

/// Assemble a complete `.kt` file from package, imports, and body.
pub(super) fn assemble_kt_content(package: &str, imports: &BTreeSet<String>, body: &str) -> String {
    // File-level suppression annotations silence ktlint / detekt rules that are
    // inherently violated by generated code (trailing commas, annotation spacing,
    // when-entry bracing, etc.) and cannot be trivially fixed without a full
    // reformatter post-processing step.
    let suppressions = vec![
        "ktlint:standard:trailing-comma-on-call-site",
        "ktlint:standard:trailing-comma-on-declaration-site",
        "ktlint:standard:spacing-between-declarations-with-comments",
        "ktlint:standard:spacing-between-declarations-with-annotations",
        "ktlint:standard:when-entry-bracing",
        "ktlint:standard:blank-line-between-when-conditions",
        "ktlint:standard:blank-line-before-declaration",
        "ktlint:standard:chain-method-continuation",
        "ktlint:standard:annotation",
        "ktlint:standard:max-line-length",
        "ktlint:standard:no-semi",
        "ktlint:standard:statement-wrapping",
        "MaxLineLength",
        "TooManyFunctions",
        "FunctionParameterNaming",
        "LongParameterList",
        "CyclomaticComplexMethod",
        "LongMethod",
        "MagicNumber",
        // Jackson deserializer for heterogeneous-default sealed enums nests
        // when-blocks past detekt's NestedBlockDepth threshold (introduced in
        // the deserializer added by 2bdbb0db8). Generated code; restructuring
        // would obscure the readNode → match → readNode flow.
        "NestedBlockDepth",
    ];
    let imports = imports.iter().cloned().collect::<Vec<_>>();
    template_env::render(
        "kt_file.jinja",
        minijinja::context! {
            package => package,
            imports => imports,
            suppressions => suppressions,
            body => body,
        },
    )
}
