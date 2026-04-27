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

/// Checks if a return value needs unwrapping from a RustBridge primitive container
/// to its idiomatic Swift counterpart (RustString → String, RustVec<UInt8> → Data,
/// RustVec<RustString> → [String], RustVec<T> → [T]).
fn should_convert_return(_ty: &TypeRef, _non_codable_types: &std::collections::HashSet<&str>) -> bool {
    // Now that all Named types are typealiases to RustBridge.X, the only return
    // values that need conversion are primitive containers (RustString, RustVec)
    // that swift-bridge wraps differently than the idiomatic Swift mapping.
    //
    // We could narrow this further by inspecting `_ty`, but `wrap_value_conversion`
    // already returns `expr` unchanged for types that don't need wrapping.
    true
}

/// Checks if an argument needs conversion from idiomatic Swift to a RustBridge primitive.
/// Cases:
/// - `Data → RustVec<UInt8>` (swift-bridge accepts [UInt8] not Data)
/// - `[T] → RustVec<T>` (swift-bridge accepts RustVec not native arrays)
/// - `URL → String` (swift-bridge accepts IntoRustString, URL does not conform)
fn arg_needs_conversion(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Path)
}

/// Builds the Swift expression that converts an argument from idiomatic Swift to
/// the RustBridge type expected at the bridge call site. Returns the original
/// expression unchanged if no conversion is needed.
fn wrap_arg_for_rustbridge(ty: &TypeRef, expr: &str) -> String {
    match ty {
        // `Data → RustVec<UInt8>`: Create a RustVec<UInt8> from the [UInt8] array.
        TypeRef::Bytes => {
            // Swift doesn't have a nice way to do this inline, so we construct
            // an empty vec and push each byte from the array.
            format!("{{ var _vec = RustVec<UInt8>(); Array({expr}).forEach {{ _vec.push(value: $0) }}; _vec }}()")
        }

        // `[T] → RustVec<T>`: for any Vec type, build RustVec from the array by pushing elements
        TypeRef::Vec(inner) => match inner.as_ref() {
            // `[String] → RustVec<RustString>`: convert each Swift String to RustString
            TypeRef::String => {
                format!("{{ var _vec = RustVec<RustString>(); {expr}.forEach {{ _vec.push(value: RustString($0)) }}; _vec }}()")
            }
            // `[[String]] → RustVec<RustVec<RustString>>`: nested vecs need recursive conversion
            TypeRef::Vec(inner_inner) if matches!(inner_inner.as_ref(), TypeRef::String) => {
                // For each inner array, construct a RustVec<RustString> and push to outer vec
                format!(
                    "{{ var _outerVec = RustVec<RustVec<RustString>>(); {expr}.forEach {{ innerArr in var _innerVec = RustVec<RustString>(); innerArr.forEach {{ _innerVec.push(value: RustString($0)) }}; _outerVec.push(value: _innerVec) }}; _outerVec }}()"
                )
            }
            // For other Vec types, push elements as-is
            _ => {
                let vec_type = format_vec_type(inner);
                format!("{{ var _vec = RustVec<{vec_type}>(); {expr}.forEach {{ _vec.push(value: $0) }}; _vec }}()")
            }
        },

        // `URL → IntoRustString`: Path parameters become URL; convert via .path
        TypeRef::Path => format!("{expr}.path"),

        // All other types pass through unchanged
        _ => expr.to_string(),
    }
}

/// Format the inner type of a Vec for RustVec<T> generic parameter.
fn format_vec_type(inner: &TypeRef) -> String {
    match inner {
        TypeRef::String => "RustString".to_string(),
        TypeRef::Bytes => "UInt8".to_string(),
        TypeRef::Named(name) => name.clone(),
        // For other types, map through the SwiftMapper
        _ => {
            let mapper = SwiftMapper;
            mapper.map_type(inner).to_string()
        }
    }
}

/// Builds a Swift expression that converts a value from a RustBridge type to the
/// idiomatic Swift counterpart.
///
/// Conversion table (left = bridge type, right = Swift wrapper signature type):
/// - `RustString → String`              : `expr.toString()`
/// - `RustStringRef → String`           : `expr.as_str().toString()`
/// - `RustVec<UInt8> → Data`            : `Data(expr)` (RustVec is Sequence)
/// - `RustVec<RustString> → [String]`   : `expr.map { $0.toString() }`
/// - `RustVec<RustVec<UInt8>> → [Data]` : `expr.map { Data($0) }`
/// - `RustVec<T> → [T]`  (T typealias)  : `Array(expr)`
/// - `Optional<T> → T?`                 : `.map { ... }` recursion
///
/// All Named types are typealiases to `RustBridge.X` (see emit_enum / emit_struct),
/// so no wrapper-type construction is emitted for them — pass-through suffices.
fn wrap_value_conversion(
    ty: &TypeRef,
    expr: &str,
    _mapper: &SwiftMapper,
    _non_codable_types: &std::collections::HashSet<&str>,
) -> String {
    match ty {
        // Non-Codable typealias to RustBridge.X — no conversion needed.
        TypeRef::Named(_) => expr.to_string(),

        TypeRef::String => format!("{expr}.toString()"),
        TypeRef::Bytes => format!("Data({expr})"),

        TypeRef::Optional(inner) => {
            let inner_conversion = wrap_value_conversion(inner, "val", _mapper, _non_codable_types);
            // Skip the `.map` when the inner conversion is already a no-op.
            if inner_conversion == "val" {
                expr.to_string()
            } else {
                format!("{expr}.map {{ val in {inner_conversion} }}")
            }
        }

        TypeRef::Vec(inner) => match inner.as_ref() {
            // When iterating over RustVec<RustString>, we get RustStringRef (per Collection impl).
            // RustStringRef.as_str() returns RustStr, which has .toString().
            TypeRef::String => format!("{expr}.map {{ $0.as_str().toString() }}"),
            // Similarly, RustVec<...Bytes> elements are iterators over Ref types
            TypeRef::Bytes => format!("{expr}.map {{ Data($0) }}"),
            TypeRef::Named(_) => format!("Array({expr})"),
            _ => format!("Array({expr})"),
        },

        // Primitives (Bool/Int/etc.), Unit, Map, Path, Char, Json, Duration —
        // either bridge as-is or are not currently exercised at the wrapper layer.
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
        // It provides opaque type definitions and FFI entry points.
        if !api.types.is_empty() || !api.enums.is_empty() || !api.errors.is_empty() {
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

        // NOTE: Function wrappers disabled (Phase 2D).
        // The wrapper layer attempted to bridge swift-bridge's RustVec/RustString primitive
        // containers to idiomatic Swift types (Array/String), but encountered unfixable
        // type-conversion errors:
        //   1. RustVec<T> constructors don't support initializing from [T] arrays
        //   2. IR types (e.g., Vec<Vec<String>>) don't match RustBridge signatures
        //   3. Generic parameter inference fails across protocol boundaries
        //
        // Consumers should use RustBridge functions directly:
        //   - import RustBridge
        //   - Kreuzberg.SomeType (typealias = RustBridge.SomeType)
        //   - RustBridge.someFunction(...) for functions
        //
        // if !visible_functions.is_empty() {
        //     body.push_str(&format!("public enum {module_name} {{\n"));
        //     for f in &visible_functions {
        //         emit_function(f, &mut body, &mapper, &non_codable_types);
        //         body.push('\n');
        //     }
        //     body.push_str("}\n");
        // }

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

    // Build the call expression: forward each parameter by name, applying any
    // primitive conversion that swift-bridge requires (e.g., `Data → [UInt8]`).
    let bridge_func = format!("RustBridge.{func_name}");
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let name = swift_ident(&p.name.to_lower_camel_case());
            if arg_needs_conversion(&p.ty) {
                wrap_arg_for_rustbridge(&p.ty, &name)
            } else {
                name
            }
        })
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

