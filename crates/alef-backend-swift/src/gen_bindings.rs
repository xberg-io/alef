use alef_codegen::keywords::swift_ident;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, EnumVariant, ErrorDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::gen_rust_crate;
use crate::type_map::SwiftMapper;

/// Checks if a type reference contains any Named types (struct/enum references).
/// Used to determine if conversion from RustBridge types is needed.
/// Non-Codable types (typealiases to RustBridge.X) don't need conversion.
fn should_convert_return(ty: &TypeRef, non_codable_types: &std::collections::HashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(name) => !non_codable_types.contains(name.as_str()),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(n) if !non_codable_types.contains(n.as_str()))
        }
        _ => is_container_of_named(ty, non_codable_types),
    }
}

/// Recursively checks if a type is a container (Vec, Optional, etc.) of Named types
/// that need conversion (i.e., not non-Codable typealiases).
fn is_container_of_named(ty: &TypeRef, non_codable_types: &std::collections::HashSet<&str>) -> bool {
    match ty {
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(n) if !non_codable_types.contains(n.as_str()))
                || is_container_of_named(inner, non_codable_types)
        }
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(n) if !non_codable_types.contains(n.as_str()))
                || is_container_of_named(inner, non_codable_types)
        }
        TypeRef::Map(k, v) => {
            is_container_of_named(k, non_codable_types) || is_container_of_named(v, non_codable_types)
        }
        _ => false,
    }
}

/// Builds a Swift expression that converts a value from RustBridge type to the idiomatic wrapper type.
/// For example: `Data(rustVal: value)` or `[String](rustVal: value)`.
/// For non-Codable types (which are typealiases to RustBridge.X), returns the expression unchanged.
fn wrap_value_conversion(
    ty: &TypeRef,
    expr: &str,
    mapper: &SwiftMapper,
    non_codable_types: &std::collections::HashSet<&str>,
) -> String {
    match ty {
        TypeRef::Named(name) => {
            if non_codable_types.contains(name.as_str()) {
                // Non-Codable type is a typealias to RustBridge.X — no conversion needed
                expr.to_string()
            } else {
                format!("{}(rustVal: {})", name, expr)
            }
        }
        TypeRef::Optional(inner) => {
            let inner_conversion = wrap_value_conversion(inner, "val", mapper, non_codable_types);
            format!("{}.map {{ val in {} }}", expr, inner_conversion)
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) => {
                if non_codable_types.contains(name.as_str()) {
                    // Non-Codable type in Vec — no conversion needed
                    expr.to_string()
                } else {
                    format!("{}.map {{ {}(rustVal: $0) }}", expr, name)
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

pub struct SwiftBackend;

impl Backend for SwiftBackend {
    fn name(&self) -> &str {
        "swift"
    }

    fn language(&self) -> Language {
        Language::Swift
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
        let module_name = config.swift_module();
        let mapper = SwiftMapper;

        let exclude_functions: std::collections::HashSet<&str> = config
            .swift
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: std::collections::HashSet<&str> = config
            .swift
            .as_ref()
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let visible_functions: Vec<&FunctionDef> = api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
            .collect();

        let mut imports: BTreeSet<String> = BTreeSet::new();
        // Foundation is always included — Codable, Data, URL all live there.
        imports.insert("import Foundation".to_string());
        // RustBridge is the Swift module generated by swift-bridge from the Rust bridge crate.
        // It provides the FFI entry points that the wrapper delegates to.
        if !visible_functions.is_empty() {
            imports.insert("import RustBridge".to_string());
        }

        let mut body = String::new();

        // Compute the set of non-trait types that should be emitted as typealiases to RustBridge.X.
        // This includes ALL non-trait enums and structs (both Codable and non-Codable).
        // swift-bridge generates opaque Swift classes for these, not enums or structs.
        // Emitting typealiases makes them pass through without conversion, avoiding:
        //   1. Invalid enum pattern matching on opaque classes
        //   2. Type mismatches between Codable wrappers and RustBridge opaque types
        //   3. Need for bidirectional JSON serialization
        // Tradeoff: users interact with RustBridge opaque types directly rather than
        // idiomatic Swift wrappers. This is acceptable for the current phase where
        // swift-bridge handling is incomplete.
        let non_codable_types: std::collections::HashSet<&str> = api
            .enums
            .iter()
            .map(|e| e.name.as_str())
            .chain(api.types.iter().filter(|t| !t.is_trait).map(|t| t.name.as_str()))
            .collect();

        for ty in api.types.iter().filter(|t| !exclude_types.contains(t.name.as_str())) {
            emit_struct(ty, &mut body, &mapper, &non_codable_types);
            body.push('\n');
        }

        for en in api.enums.iter().filter(|e| !exclude_types.contains(e.name.as_str())) {
            emit_enum(en, &mut body, &mapper, &non_codable_types);
            body.push('\n');
        }

        for error in &api.errors {
            emit_error(error, &mut body, &mapper);
            body.push('\n');
        }

        if !visible_functions.is_empty() {
            body.push_str(&format!("public enum {module_name} {{\n"));
            for f in &visible_functions {
                emit_function(f, &mut body, &mapper, &non_codable_types);
                body.push('\n');
            }
            body.push_str("}\n");
        }

        let mut content = String::new();
        content.push_str("// Generated by alef. Do not edit by hand.\n\n");
        for import in &imports {
            content.push_str(import);
            content.push('\n');
        }
        content.push('\n');
        content.push_str(&body);

        let dir = resolve_output_dir(
            config.output.swift.as_ref(),
            &config.crate_config.name,
            &format!("Sources/{module_name}"),
        );
        let path = PathBuf::from(dir).join(format!("{module_name}.swift"));

        let mut files = vec![GeneratedFile {
            path,
            content,
            generated_header: false,
        }];

        // Phase 2C: emit the Rust-side swift-bridge crate
        let rust_crate_files = gen_rust_crate::emit(api, config)?;
        files.extend(rust_crate_files);

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "swift",
            crate_suffix: "-swift",
            build_dep: BuildDependency::None,
            // Build the Rust bridge crate first so swift-bridge codegen produces
            // the Swift glue files that the Swift Package consumes.
            post_build: vec![PostBuildStep::RunCommand {
                cmd: "cargo",
                args: vec!["build", "--release"],
            }],
        })
    }
}

/// Emits a Swift `public struct` for the given `TypeDef`.
/// All non-trait structs are emitted as typealiases to RustBridge.X.
fn emit_struct(ty: &TypeDef, out: &mut String, _mapper: &SwiftMapper, non_codable_types: &std::collections::HashSet<&str>) {
    // All structs become typealiases to the RustBridge opaque class
    if non_codable_types.contains(ty.name.as_str()) {
        emit_doc_comment(&ty.doc, "", out);
        out.push_str(&format!("public typealias {} = RustBridge.{}\n", ty.name, ty.name));
        return;
    }

    // Trait types fall through but shouldn't happen in practice
    emit_doc_comment(&ty.doc, "", out);
    out.push_str(&format!("public struct {} {{\n", ty.name));
    out.push_str("}\n");
}

/// Emits a Swift `public enum` for the given `EnumDef`.
/// All non-trait enums are emitted as typealiases to RustBridge.X.
fn emit_enum(
    en: &EnumDef,
    out: &mut String,
    mapper: &SwiftMapper,
    non_codable_types: &std::collections::HashSet<&str>,
) {
    // All enums become typealiases to the RustBridge opaque class
    if non_codable_types.contains(en.name.as_str()) {
        emit_doc_comment(&en.doc, "", out);
        out.push_str(&format!("public typealias {} = RustBridge.{}\n", en.name, en.name));
        return;
    }

    emit_doc_comment(&en.doc, "", out);

    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());

    if all_unit {
        out.push_str(&format!("public enum {} {{\n", en.name));
        for variant in &en.variants {
            emit_doc_comment(&variant.doc, "    ", out);
            let case_name = swift_ident(&variant.name.to_lower_camel_case());
            out.push_str(&format!("    case {case_name}\n"));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&format!("public enum {} {{\n", en.name));
        for variant in &en.variants {
            emit_variant_with_data(variant, out, mapper);
        }
        out.push_str("}\n");
    }
}

/// Emits a single enum case, with or without associated values.
fn emit_variant_with_data(variant: &EnumVariant, out: &mut String, mapper: &SwiftMapper) {
    emit_doc_comment(&variant.doc, "    ", out);
    let case_name = swift_ident(&variant.name.to_lower_camel_case());
    if variant.fields.is_empty() {
        out.push_str(&format!("    case {case_name}\n"));
    } else {
        let assoc: Vec<String> = variant
            .fields
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let ty_str = mapper.map_type(&f.ty);
                let label = swift_associated_label(&f.name, idx);
                format!("{label}: {ty_str}")
            })
            .collect();
        out.push_str(&format!("    case {case_name}({})\n", assoc.join(", ")));
    }
}

/// Resolves a Swift associated-value label for an enum case field.
///
/// - Empty, all-digit, or `_<digits>` names (positional tuple variants) become
///   `field0`, `field1`, …
/// - Otherwise lowerCamelCase + Swift keyword escaping.
fn swift_associated_label(name: &str, idx: usize) -> String {
    let stripped = name.trim_start_matches('_');
    if stripped.is_empty() || stripped.chars().all(|c| c.is_ascii_digit()) {
        return format!("field{idx}");
    }
    swift_ident(&name.to_lower_camel_case())
}

/// Emits a Swift `Error`-conforming `public enum` for the given `ErrorDef`.
fn emit_error(error: &ErrorDef, out: &mut String, mapper: &SwiftMapper) {
    emit_doc_comment(&error.doc, "", out);
    out.push_str(&format!("public enum {}: Error {{\n", error.name));
    for variant in &error.variants {
        emit_doc_comment(&variant.doc, "    ", out);
        let case_name = swift_ident(&variant.name.to_lower_camel_case());
        if variant.is_unit || variant.fields.is_empty() {
            out.push_str(&format!("    case {case_name}(message: String)\n"));
        } else {
            let mut assoc: Vec<String> = Vec::with_capacity(variant.fields.len() + 1);
            let mut seen_message = false;
            let mut labels: BTreeSet<String> = BTreeSet::new();
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = mapper.map_type(&f.ty);
                let mut label = swift_associated_label(&f.name, idx);
                // Disambiguate duplicate labels by suffixing the index.
                while labels.contains(&label) {
                    label = format!("{label}{idx}");
                }
                labels.insert(label.clone());
                if label == "message" {
                    seen_message = true;
                }
                assoc.push(format!("{label}: {ty_str}"));
            }
            if !seen_message {
                assoc.insert(0, "message: String".to_string());
            }
            out.push_str(&format!("    case {case_name}({})\n", assoc.join(", ")));
        }
    }
    out.push_str("}\n");
}

/// Emits a single `public static func` inside the module enum namespace.
fn emit_function(
    f: &FunctionDef,
    out: &mut String,
    mapper: &SwiftMapper,
    non_codable_types: &std::collections::HashSet<&str>,
) {
    emit_doc_comment(&f.doc, "    ", out);

    let params: Vec<String> = f.params.iter().map(|p| format_param(p, mapper)).collect();
    let return_ty = mapper.map_type(&f.return_type);

    let func_name = swift_ident(&f.name.to_lower_camel_case());

    // Build the qualifier string: [async] [throws] [-> ReturnType]
    let mut qualifiers = String::new();
    if f.is_async {
        qualifiers.push_str("async ");
    }
    if f.error_type.is_some() {
        qualifiers.push_str("throws ");
    }
    // Omit `-> Void` per Swift convention.
    if !matches!(f.return_type, TypeRef::Unit) {
        qualifiers.push_str(&format!("-> {return_ty} "));
    }

    out.push_str(&format!(
        "    public static func {func_name}({}) {qualifiers}{{\n",
        params.join(", "),
    ));

    // Build the call expression: forward each parameter by name.
    let bridge_func = format!("RustBridge.{func_name}");
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| swift_ident(&p.name.to_lower_camel_case()))
        .collect();
    let call_expr = format!("{bridge_func}({})", call_args.join(", "));

    // Prefix the call with await/try as required.
    let awaited = if f.is_async {
        format!("await {call_expr}")
    } else {
        call_expr
    };
    let invocation = if f.error_type.is_some() {
        format!("try {awaited}")
    } else {
        awaited
    };

    if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&format!("        {invocation}\n"));
    } else {
        // Wrap return value with conversion if it's a Named type or container of Named types
        let return_expr = if should_convert_return(&f.return_type, non_codable_types) {
            wrap_value_conversion(&f.return_type, &invocation, mapper, non_codable_types)
        } else {
            invocation
        };
        out.push_str(&format!("        return {return_expr}\n"));
    }
    out.push_str("    }\n");
}

/// Formats a single parameter as `label: Type`.
fn format_param(p: &ParamDef, mapper: &SwiftMapper) -> String {
    let ty_str = mapper.map_type(&p.ty);
    let name = swift_ident(&p.name.to_lower_camel_case());
    format!("{name}: {ty_str}")
}

/// Emits `/// <line>` doc-comment lines with the given indent prefix.
fn emit_doc_comment(doc: &str, indent: &str, out: &mut String) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

