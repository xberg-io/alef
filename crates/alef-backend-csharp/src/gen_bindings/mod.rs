use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, FieldDef, TypeRef};
use heck::ToPascalCase;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Metadata for a streaming adapter, used to drive emission of an
/// `IAsyncEnumerable<Item>` method over the FFI iterator-handle protocol
/// (`_start` / `_next` / `_free`).
#[derive(Debug, Clone)]
pub(super) struct StreamingMethodMeta {
    /// Owner type (e.g. `DefaultClient`). Retained for future routing decisions even when the
    /// current emitter derives the receiver type from the enclosing class.
    #[allow(dead_code)]
    pub owner_type: String,
    pub item_type: String,
}

pub(super) mod enums;
pub(super) mod errors;
pub(super) mod functions;
pub(super) mod methods;
pub(super) mod types;

pub struct CsharpBackend;

impl CsharpBackend {
    // lib_name comes from config.ffi_lib_name()
}

impl Backend for CsharpBackend {
    fn name(&self) -> &str {
        "csharp"
    }

    fn language(&self) -> Language {
        Language::Csharp
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

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let namespace = config.csharp_namespace();
        let prefix = config.ffi_prefix();
        let lib_name = config.ffi_lib_name();

        // Collect bridge param names and type aliases from trait_bridges config so we can strip
        // them from generated function signatures and emit ConvertWithVisitor instead.
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
        // Only emit ConvertWithVisitor method if visitor_callbacks is explicitly enabled in FFI config
        let has_visitor_callbacks = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false);
        let bridge_associated_types = config.bridge_associated_types();

        // Streaming adapter methods are emitted via the iterator-handle FFI protocol
        // (`{prefix}_{owner}_{name}_start` / `_next` / `_free`) — not as direct P/Invoke calls
        // of the callback-based variant. The set is still used to skip the default
        // method-emission path; the parallel meta map drives the `IAsyncEnumerable` emitters.
        let streaming_methods: HashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .map(|a| a.name.clone())
            .collect();
        let streaming_methods_meta: HashMap<String, StreamingMethodMeta> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .filter_map(|a| {
                let owner_type = a.owner_type.clone()?;
                let item_type = a.item_type.clone()?;
                Some((a.name.clone(), StreamingMethodMeta { owner_type, item_type }))
            })
            .collect();

        // Functions explicitly excluded from C# bindings (e.g., not present in the C FFI layer).
        let exclude_functions: HashSet<String> = config
            .csharp
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let output_dir = resolve_output_dir(config.output_paths.get("csharp"), &config.name, "packages/csharp/");

        let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

        let mut files = Vec::new();

        // Fallback generic exception class name (used by GetLastError and as base for typed errors)
        let exception_class_name = format!("{}Exception", api.crate_name.to_pascal_case());

        // 1. Generate NativeMethods.cs
        files.push(GeneratedFile {
            path: base_path.join("NativeMethods.cs"),
            content: strip_trailing_whitespace(&functions::gen_native_methods(
                api,
                &namespace,
                &lib_name,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_callbacks,
                &config.trait_bridges,
                &streaming_methods,
                &streaming_methods_meta,
                &exclude_functions,
            )),
            generated_header: true,
        });

        // 2. Generate error types from thiserror enums (if any), otherwise generic exception
        if !api.errors.is_empty() {
            for error in &api.errors {
                let error_files =
                    alef_codegen::error_gen::gen_csharp_error_types(error, &namespace, Some(&exception_class_name));
                for (class_name, content) in error_files {
                    files.push(GeneratedFile {
                        path: base_path.join(format!("{}.cs", class_name)),
                        content: strip_trailing_whitespace(&content),
                        generated_header: false, // already has header
                    });
                }
            }
        }

        // Fallback generic exception class (always generated for GetLastError)
        if api.errors.is_empty()
            || !api
                .errors
                .iter()
                .any(|e| format!("{}Exception", e.name) == exception_class_name)
        {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", exception_class_name)),
                content: strip_trailing_whitespace(&errors::gen_exception_class(&namespace, &exception_class_name)),
                generated_header: true,
            });
        }

        // 3. Generate main wrapper class
        let base_class_name = api.crate_name.to_pascal_case();
        let wrapper_class_name = if namespace == base_class_name {
            format!("{}Lib", base_class_name)
        } else {
            base_class_name
        };
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.cs", wrapper_class_name)),
            content: strip_trailing_whitespace(&methods::gen_wrapper_class(
                api,
                &namespace,
                &wrapper_class_name,
                &exception_class_name,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_callbacks,
                &streaming_methods,
                &streaming_methods_meta,
                &exclude_functions,
                &config.trait_bridges,
            )),
            generated_header: true,
        });

        // 3b. Generate visitor support files when a bridge is configured.
        if has_visitor_callbacks {
            // Look up the visitor trait def from the IR via TraitBridgeConfig.trait_name,
            // mirroring the Go backend's pattern so that gen_visitor_files is IR-driven.
            let visitor_bridge_cfg = config
                .trait_bridges
                .iter()
                .find(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField);
            let trait_map: std::collections::HashMap<&str, &alef_core::ir::TypeDef> = api
                .types
                .iter()
                .filter(|t| t.is_trait)
                .map(|t| (t.name.as_str(), t))
                .collect();
            let visitor_trait = visitor_bridge_cfg.and_then(|b| trait_map.get(b.trait_name.as_str()).copied());

            if let Some(trait_def) = visitor_trait {
                for (filename, content) in crate::gen_visitor::gen_visitor_files(&namespace, trait_def) {
                    files.push(GeneratedFile {
                        path: base_path.join(filename),
                        content: strip_trailing_whitespace(&content),
                        generated_header: true,
                    });
                }
            } else {
                // Trait not in IR (e.g. parsed separately); fall back to a minimal placeholder.
                let placeholder = alef_core::ir::TypeDef {
                    name: String::new(),
                    rust_path: String::new(),
                    original_rust_path: String::new(),
                    fields: vec![],
                    methods: vec![],
                    is_opaque: false,
                    is_clone: false,
                    is_copy: false,
                    is_trait: true,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    serde_rename_all: None,
                    has_serde: false,
                    super_traits: vec![],
                    doc: String::new(),
                    cfg: None,
                };
                for (filename, content) in crate::gen_visitor::gen_visitor_files(&namespace, &placeholder) {
                    files.push(GeneratedFile {
                        path: base_path.join(filename),
                        content: strip_trailing_whitespace(&content),
                        generated_header: true,
                    });
                }
            }
            // IVisitor.cs and VisitorCallbacks.cs were removed from gen_visitor_files() in favour
            // of the HtmlVisitorBridge path in TraitBridges.cs.  Delete any stale copies left
            // over from earlier generator runs.
            delete_superseded_visitor_files(&base_path)?;
        } else {
            // When visitor_callbacks is disabled, delete stale files from prior runs
            // to prevent CS8632 warnings (nullable context not enabled).
            delete_stale_visitor_files(&base_path)?;
        }

        // 3c. Generate trait bridge classes when configured.
        if !config.trait_bridges.is_empty() {
            let trait_defs: Vec<_> = api.types.iter().filter(|t| t.is_trait).collect();
            let bridges: Vec<_> = config
                .trait_bridges
                .iter()
                .filter_map(|cfg| {
                    let trait_name = cfg.trait_name.clone();
                    trait_defs
                        .iter()
                        .find(|t| t.name == trait_name)
                        .map(|trait_def| (trait_name, cfg, *trait_def))
                })
                .collect();

            if !bridges.is_empty() {
                // Collect visible type names (non-trait types that have C# bindings)
                let visible_type_names: HashSet<&str> = api
                    .types
                    .iter()
                    .filter(|t| !t.is_trait)
                    .map(|t| t.name.as_str())
                    .collect();
                let (filename, content) =
                    crate::trait_bridge::gen_trait_bridges_file(&namespace, &prefix, &bridges, &visible_type_names);
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content: strip_trailing_whitespace(&content),
                    generated_header: true,
                });
            }
        }

        // Collect enum names so record generation can distinguish enum fields from class fields.
        let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.to_pascal_case()).collect();

        // Collect all opaque type names (pascal-cased) so methods on one opaque type that
        // return another opaque type are wrapped correctly rather than JSON-serialized.
        let all_opaque_type_names: HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.to_pascal_case())
            .collect();

        // 4. Generate opaque handle classes
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                let type_filename = typ.name.to_pascal_case();
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&types::gen_opaque_handle(
                        typ,
                        &namespace,
                        &exception_class_name,
                        &enum_names,
                        &streaming_methods,
                        &streaming_methods_meta,
                        &all_opaque_type_names,
                    )),
                    generated_header: true,
                });
            }
        }

        // Untagged unions with data variants now emit as JsonElement-wrapper classes
        // (see gen_untagged_wrapper). The set is intentionally empty so record fields
        // keep their wrapper-class type instead of being downcast to JsonElement.
        let complex_enums: HashSet<String> = HashSet::new();

        // Collect enums that require a custom JsonConverter (non-standard serialized names only).
        // Tagged unions are generated as abstract records with [JsonPolymorphic] and do NOT need
        // a custom converter — the attribute on the type itself handles polymorphic deserialization.
        // When a property has a custom-converter enum as its type, emit a property-level
        // [JsonConverter] attribute so the custom converter wins over the global JsonStringEnumConverter.
        let custom_converter_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                // Skip tagged unions — they use [JsonPolymorphic] instead
                let is_tagged_union = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
                if is_tagged_union {
                    return false;
                }
                // Enums whose `serde_rename_all` is something other than snake_case
                // (e.g. "kebab-case" for `FilePurpose::FineTune` → `"fine-tune"`)
                // need a custom converter — `JsonStringEnumConverter(SnakeCaseLower)`
                // would write `"fine_tune"` instead.
                let rename_all_differs = matches!(
                    e.serde_rename_all.as_deref(),
                    Some("kebab-case") | Some("SCREAMING-KEBAB-CASE") | Some("camelCase") | Some("PascalCase")
                );
                if rename_all_differs {
                    return true;
                }
                // Enums with non-standard variant names need a custom converter
                e.variants.iter().any(|v| {
                    if let Some(ref rename) = v.serde_rename {
                        let snake = enums::apply_rename_all(&v.name, e.serde_rename_all.as_deref());
                        rename != &snake
                    } else {
                        false
                    }
                })
            })
            .map(|e| e.name.to_pascal_case())
            .collect();

        // Resolve the language-level serde rename_all strategy (always wins over IR type-level).
        let lang_rename_all = config.serde_rename_all_for_language(Language::Csharp);

        // 5. Generate record types (structs)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !typ.is_opaque {
                // Skip types where all fields are unnamed tuple positions — they have no
                // meaningful properties to expose in C#.
                let has_named_fields = typ.fields.iter().any(|f| !is_tuple_field(f));
                if !typ.fields.is_empty() && !has_named_fields {
                    continue;
                }
                // Skip types that gen_visitor handles with richer visitor-specific versions
                if has_visitor_callbacks && bridge_associated_types.contains(typ.name.as_str()) {
                    continue;
                }

                let type_filename = typ.name.to_pascal_case();
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&types::gen_record_type(
                        typ,
                        &namespace,
                        &enum_names,
                        &complex_enums,
                        &custom_converter_enums,
                        &lang_rename_all,
                        &bridge_type_aliases,
                        &exception_class_name,
                    )),
                    generated_header: true,
                });
            }
        }

        // 6. Generate enums
        for enum_def in &api.enums {
            // Skip enums that gen_visitor handles with richer visitor-specific versions
            if has_visitor_callbacks && bridge_associated_types.contains(enum_def.name.as_str()) {
                continue;
            }
            let enum_filename = enum_def.name.to_pascal_case();
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", enum_filename)),
                content: strip_trailing_whitespace(&enums::gen_enum(enum_def, &namespace)),
                generated_header: true,
            });
        }

        // 7. Generate ByteArrayToIntArrayConverter if any non-opaque type has non-optional Bytes fields.
        // Non-optional byte[] fields must be serialized as JSON int arrays, not base64 strings.
        let needs_byte_array_converter = api
            .types
            .iter()
            .any(|t| !t.is_opaque && t.fields.iter().any(|f| !f.optional && matches!(f.ty, TypeRef::Bytes)));
        if needs_byte_array_converter {
            files.push(GeneratedFile {
                path: base_path.join("ByteArrayToIntArrayConverter.cs"),
                content: types::gen_byte_array_to_int_array_converter(&namespace),
                generated_header: true,
            });
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Csharp)?;

        // 8. Generate Directory.Build.props at the package root (always overwritten).
        // This file enables Nullable=enable and latest LangVersion for all C# projects
        // in the packages/csharp hierarchy without requiring per-csproj configuration.
        files.push(GeneratedFile {
            path: PathBuf::from("packages/csharp/Directory.Build.props"),
            content: gen_directory_build_props(),
            generated_header: true,
        });

        Ok(files)
    }

    /// C# wrapper class is already the public API.
    /// The `gen_wrapper_class` (generated in `generate_bindings`) provides high-level public methods
    /// that wrap NativeMethods (P/Invoke), marshal types, and handle errors.
    /// No additional facade is needed.
    fn generate_public_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // C#'s wrapper class IS the public API — no additional wrapper needed.
        Ok(vec![])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
pub(super) fn strip_trailing_whitespace(content: &str) -> String {
    let mut result: String = content
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Generate C# file header with hash and nullable-enable pragma.
pub(super) fn csharp_file_header() -> String {
    let mut out = hash::header(CommentStyle::DoubleSlash);
    out.push_str("#nullable enable\n\n");
    out
}

/// Generate Directory.Build.props with Nullable=enable and LangVersion=latest.
/// This is auto-generated (overwritten on each build) so it doesn't require user maintenance.
fn gen_directory_build_props() -> String {
    "<!-- auto-generated by alef (generate_bindings) -->\n\
<Project>\n  \
<PropertyGroup>\n    \
<Nullable>enable</Nullable>\n    \
<LangVersion>latest</LangVersion>\n    \
<TreatWarningsAsErrors>true</TreatWarningsAsErrors>\n  \
</PropertyGroup>\n\
</Project>\n"
        .to_string()
}

/// Delete `IVisitor.cs` and `VisitorCallbacks.cs` when visitor_callbacks is enabled but the
/// modern `HtmlVisitorBridge` / `TraitBridges.cs` path supersedes them.
/// These files are no longer emitted by `gen_visitor_files()` but may exist on disk from older
/// generator runs.
fn delete_superseded_visitor_files(base_path: &std::path::Path) -> anyhow::Result<()> {
    let superseded = ["IVisitor.cs", "VisitorCallbacks.cs"];
    for filename in superseded {
        let path = base_path.join(filename);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("Failed to delete superseded visitor file {}: {}", path.display(), e))?;
        }
    }
    Ok(())
}

/// Delete stale visitor-related files when visitor_callbacks is disabled.
/// When visitor_callbacks transitions from true → false, these files remain on disk
/// and cause CS8632 warnings (nullable context not enabled in these files).
fn delete_stale_visitor_files(base_path: &std::path::Path) -> anyhow::Result<()> {
    let stale_files = vec!["IVisitor.cs", "VisitorCallbacks.cs", "NodeContext.cs", "VisitResult.cs"];

    for filename in stale_files {
        let path = base_path.join(filename);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("Failed to delete stale visitor file {}: {}", path.display(), e))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers: P/Invoke return type mapping
// ---------------------------------------------------------------------------

use alef_core::ir::PrimitiveType;

/// Returns the C# type to use in a `[DllImport]` declaration for the given return type.
///
/// Key differences from the high-level `csharp_type`:
/// - Bool is marshalled as `int` (C FFI convention) — the wrapper compares != 0.
/// - String / Named / Vec / Map / Path / Json / Bytes all come back as `IntPtr`.
/// - Numeric primitives use their natural C# types (`nuint`, `int`, etc.).
pub(super) fn pinvoke_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "void",
        // Bool over FFI is a C int (0/1).
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        // Numeric primitives — use their real C# types.
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        // Duration as u64
        TypeRef::Duration => "ulong",
        // Everything else is a pointer that needs manual marshalling.
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Optional(_)
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Named(_)
        | TypeRef::Path
        | TypeRef::Json => "IntPtr",
    }
}

/// Returns the C# type to use for a parameter in a `[DllImport]` declaration.
///
/// Managed reference types (Named structs, Vec, Map, Bytes, Optional of Named, etc.)
/// cannot be directly marshalled by P/Invoke.  They must be passed as `IntPtr` (opaque
/// handle or JSON-string pointer).  Primitive types and plain strings use their natural
/// types.
pub(super) fn pinvoke_param_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "string",
        // Managed objects — pass as opaque IntPtr (serialised to handle before call)
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes | TypeRef::Optional(_) => "IntPtr",
        TypeRef::Unit => "void",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        TypeRef::Duration => "ulong",
    }
}

/// Returns true if a parameter should be hidden from the public API because it is a
/// trait-bridge param (e.g. the FFI visitor handle).
pub(super) fn is_bridge_param(
    param: &alef_core::ir::ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    bridge_param_names.contains(&param.name)
        || matches!(&param.ty, alef_core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n))
}

/// Does the return type need IntPtr→string marshalling in the wrapper?
pub(super) fn returns_string(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json)
}

/// Does the return type come back as a C int that should be converted to bool?
pub(super) fn returns_bool_via_int(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(PrimitiveType::Bool))
}

/// Does the return type need JSON deserialization from an IntPtr string?
pub(super) fn returns_json_object(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) | TypeRef::Bytes | TypeRef::Optional(_)
    )
}

/// Returns true if the FFI return type is a pointer (IntPtr), as opposed to a numeric value.
/// Only pointer-returning functions use `IntPtr.Zero` as an error sentinel.
pub(super) fn returns_ptr(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Named(_)
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _)
            | TypeRef::Bytes
            | TypeRef::Optional(_)
    )
}

/// Returns the argument expression to pass to the native method for a given parameter.
///
/// For truly opaque types (is_opaque = true), the C# class wraps an IntPtr; pass `.Handle`.
/// For data-struct `Named` types this is the handle variable (e.g. `optionsHandle`).
/// For everything else it is the parameter name (with `!` for optional).
pub(super) fn native_call_arg(
    ty: &TypeRef,
    param_name: &str,
    optional: bool,
    true_opaque_types: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(type_name) if true_opaque_types.contains(type_name) => {
            // Truly opaque: unwrap the IntPtr from the C# handle class.
            let bang = if optional { "!" } else { "" };
            format!("{param_name}{bang}.Handle")
        }
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{param_name}Handle")
        }
        TypeRef::Bytes => {
            format!("{param_name}Handle.AddrOfPinnedObject()")
        }
        TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool) => {
            // FFI convention: bool marshalled as int (0 = false, non-zero = true)
            if optional {
                format!("({param_name}?.Value ? 1 : 0)")
            } else {
                format!("({param_name} ? 1 : 0)")
            }
        }
        ty => {
            if optional {
                // For optional primitive types (e.g. ulong?, uint?), pass the FFI's
                // None sentinel when the value is null. The FFI shim decodes
                // `{prim}::MAX` (and NAN for floats) as None — passing 0 collides with
                // a legitimate zero from the caller, e.g. timeout_secs=0 = "no timeout"
                // would be silently treated as "unset" without this. Mirrors the
                // `alef-backend-ffi` `param_optional_numeric_conversion` decoder.
                // String/Char/Path/Json are reference types so `!` is correct for those.
                if let TypeRef::Primitive(prim) = ty {
                    use alef_core::ir::PrimitiveType;
                    let sentinel = match prim {
                        PrimitiveType::U8 => "byte.MaxValue",
                        PrimitiveType::U16 => "ushort.MaxValue",
                        PrimitiveType::U32 => "uint.MaxValue",
                        PrimitiveType::U64 | PrimitiveType::Usize => "ulong.MaxValue",
                        PrimitiveType::I8 => "sbyte.MaxValue",
                        PrimitiveType::I16 => "short.MaxValue",
                        PrimitiveType::I32 => "int.MaxValue",
                        PrimitiveType::I64 | PrimitiveType::Isize => "long.MaxValue",
                        PrimitiveType::F32 => "float.NaN",
                        PrimitiveType::F64 => "double.NaN",
                        PrimitiveType::Bool => unreachable!("handled above"),
                    };
                    format!("{param_name} ?? {sentinel}")
                } else if matches!(ty, TypeRef::Duration) {
                    format!("{param_name}.GetValueOrDefault()")
                } else {
                    format!("{param_name}!")
                }
            } else {
                param_name.to_string()
            }
        }
    }
}

/// For each `Named` parameter, emit code to serialise it to JSON and obtain a native handle.
///
/// For truly opaque types (is_opaque = true), the C# class already wraps the native handle, so
/// we pass `param.Handle` directly without any JSON serialisation.
pub(super) fn emit_named_param_setup(
    out: &mut String,
    params: &[alef_core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
    exception_name: &str,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let json_var = format!("{param_name}Json");
        let handle_var = format!("{param_name}Handle");

        match &param.ty {
            TypeRef::Named(type_name) => {
                // Truly opaque handles: the C# wrapper class holds the IntPtr directly.
                // No from_json round-trip needed — pass .Handle directly in native_call_arg.
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                let from_json_method = format!("{}FromJson", type_name.to_pascal_case());

                // Config parameters: always treat as optional and default null to new instance
                let is_config_param = param.name == "config";
                let param_to_serialize = if is_config_param {
                    let type_pascal = type_name.to_pascal_case();
                    format!("({} ?? new {}())", param_name, type_pascal)
                } else {
                    param_name.to_string()
                };

                if param.optional && !is_config_param {
                    // Optional Named param: pass IntPtr.Zero through to native when the
                    // C# arg is null instead of round-tripping `"null"` through FromJson
                    // which would error with "invalid type: null, expected struct T".
                    out.push_str(&crate::template_env::render(
                        "named_param_handle_from_json_optional.jinja",
                        minijinja::context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            param_name => &param_name,
                            exception_name => exception_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "named_param_json_serialize.jinja",
                        minijinja::context! { indent, json_var => &json_var, param_name => &param_to_serialize },
                    ));
                    out.push_str(&crate::template_env::render(
                        "named_param_handle_from_json.jinja",
                        minijinja::context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            exception_name => exception_name,
                        },
                    ));
                }
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Vec/Map: serialize to JSON string, marshal to native pointer
                out.push_str(&crate::template_env::render(
                    "named_param_json_serialize.jinja",
                    minijinja::context! { indent, json_var => &json_var, param_name => &param_name },
                ));
                out.push_str(&crate::template_env::render(
                    "named_param_handle_string.jinja",
                    minijinja::context! { indent, handle_var => &handle_var, json_var => &json_var },
                ));
            }
            TypeRef::Bytes => {
                // byte[]: pin the managed array and pass pointer to native
                out.push_str(&crate::template_env::render(
                    "named_param_handle_pin.jinja",
                    minijinja::context! { indent, handle_var => &handle_var, param_name => &param_name },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code to free native handles allocated for `Named` parameters.
///
/// Truly opaque handles (is_opaque = true) are NOT freed here — their lifetime is managed by
/// the C# wrapper class (IDisposable). Only data-struct handles (from_json-allocated) are freed.
pub(super) fn emit_named_param_teardown(
    out: &mut String,
    params: &[alef_core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    // Caller owns the opaque handle — do not free it here.
                    continue;
                }
                let free_method = format!("{}Free", type_name.to_pascal_case());
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_free.jinja",
                    minijinja::context! { indent => "        ", free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    minijinja::context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    minijinja::context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code with configurable indentation (used inside `Task.Run` lambdas).
pub(super) fn emit_named_param_teardown_indented(
    out: &mut String,
    params: &[alef_core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    // Caller owns the opaque handle — do not free it here.
                    continue;
                }
                let free_method = format!("{}Free", type_name.to_pascal_case());
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_free.jinja",
                    minijinja::context! { indent, free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    minijinja::context! { indent, handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    minijinja::context! { indent, handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}

use heck::ToLowerCamelCase;
