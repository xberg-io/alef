use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, FieldDef, FunctionDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;
use std::path::PathBuf;

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

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let namespace = config.csharp_namespace();
        let prefix = config.ffi_prefix();
        let lib_name = config.ffi_lib_name();

        let output_dir = resolve_output_dir(
            config.output.csharp.as_ref(),
            &config.crate_config.name,
            "packages/csharp/",
        );

        let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

        let mut files = Vec::new();

        // 1. Generate NativeMethods.cs
        files.push(GeneratedFile {
            path: base_path.join("NativeMethods.cs"),
            content: strip_trailing_whitespace(&gen_native_methods(api, &namespace, &lib_name, &prefix)),
            generated_header: true,
        });

        // 2. Generate error types from thiserror enums (if any), otherwise generic exception
        if !api.errors.is_empty() {
            for error in &api.errors {
                let error_files = alef_codegen::error_gen::gen_csharp_error_types(error, &namespace);
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
        let exception_class_name = format!("{}Exception", api.crate_name.to_pascal_case());
        if api.errors.is_empty()
            || !api
                .errors
                .iter()
                .any(|e| format!("{}Exception", e.name) == exception_class_name)
        {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", exception_class_name)),
                content: strip_trailing_whitespace(&gen_exception_class(&namespace, &exception_class_name)),
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
            content: strip_trailing_whitespace(&gen_wrapper_class(
                api,
                &namespace,
                &wrapper_class_name,
                &exception_class_name,
                &prefix,
            )),
            generated_header: true,
        });

        // 4. Generate opaque handle classes
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                let type_filename = typ.name.to_pascal_case();
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&gen_opaque_handle(typ, &namespace)),
                    generated_header: true,
                });
            }
        }

        // Collect enum names so record generation can distinguish enum fields from class fields.
        let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.to_pascal_case()).collect();

        // Collect complex enums (enums with data variants and no serde tag) — these can't be
        // simple C# enums and should be represented as JsonElement for flexible deserialization.
        // Tagged unions (serde_tag is set) are now generated as proper abstract records
        // and can be deserialized as their concrete types, so they are NOT complex_enums.
        let complex_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_none() && e.variants.iter().any(|v| !v.fields.is_empty()))
            .map(|e| e.name.to_pascal_case())
            .collect();

        // Collect enums that require a custom JsonConverter (non-standard serialized names or
        // tagged unions). When a property has this enum as its type, we must emit a property-level
        // [JsonConverter] attribute so the custom converter wins over the global JsonStringEnumConverter.
        let custom_converter_enums: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                // Tagged unions always use a custom converter
                (e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
                // Enums with non-standard variant names need a custom converter
                || e.variants.iter().any(|v| {
                    if let Some(ref rename) = v.serde_rename {
                        let snake = apply_rename_all(&v.name, e.serde_rename_all.as_deref());
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

                let type_filename = typ.name.to_pascal_case();
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.cs", type_filename)),
                    content: strip_trailing_whitespace(&gen_record_type(
                        typ,
                        &namespace,
                        &enum_names,
                        &complex_enums,
                        &custom_converter_enums,
                        &lang_rename_all,
                    )),
                    generated_header: true,
                });
            }
        }

        // 6. Generate enums
        for enum_def in &api.enums {
            let enum_filename = enum_def.name.to_pascal_case();
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.cs", enum_filename)),
                content: strip_trailing_whitespace(&gen_enum(enum_def, &namespace)),
                generated_header: true,
            });
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Csharp)?;

        Ok(files)
    }

    /// C# wrapper class is already the public API.
    /// The `gen_wrapper_class` (generated in `generate_bindings`) provides high-level public methods
    /// that wrap NativeMethods (P/Invoke), marshal types, and handle errors.
    /// No additional facade is needed.
    fn generate_public_api(&self, _api: &ApiSurface, _config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // C#'s wrapper class IS the public API — no additional wrapper needed.
        Ok(vec![])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            depends_on_ffi: true,
            post_build: vec![],
        })
    }
}

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
fn strip_trailing_whitespace(content: &str) -> String {
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

// ---------------------------------------------------------------------------
// Helpers: P/Invoke return type mapping
// ---------------------------------------------------------------------------

/// Returns the C# type to use in a `[DllImport]` declaration for the given return type.
///
/// Key differences from the high-level `csharp_type`:
/// - Bool is marshalled as `int` (C FFI convention) — the wrapper compares != 0.
/// - String / Named / Vec / Map / Path / Json / Bytes all come back as `IntPtr`.
/// - Numeric primitives use their natural C# types (`nuint`, `int`, etc.).
fn pinvoke_return_type(ty: &TypeRef) -> &'static str {
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

/// Does the return type need IntPtr→string marshalling in the wrapper?
fn returns_string(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json)
}

/// Does the return type come back as a C int that should be converted to bool?
fn returns_bool_via_int(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(PrimitiveType::Bool))
}

/// Does the return type need JSON deserialization from an IntPtr string?
fn returns_json_object(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) | TypeRef::Bytes | TypeRef::Optional(_)
    )
}

/// Does this return type represent an opaque handle (Named struct type) that needs special marshalling?
///
/// Opaque handles are returned as `IntPtr` from P/Invoke.  The wrapper must call
/// `{prefix}_{type_snake}_to_json(ptr)` to obtain a JSON string, then deserialise it,
/// Returns the C# type to use for a parameter in a `[DllImport]` declaration.
///
/// Managed reference types (Named structs, Vec, Map, Bytes, Optional of Named, etc.)
/// cannot be directly marshalled by P/Invoke.  They must be passed as `IntPtr` (opaque
/// handle or JSON-string pointer).  Primitive types and plain strings use their natural
/// types.
fn pinvoke_param_type(ty: &TypeRef) -> &'static str {
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

// ---------------------------------------------------------------------------
// Code generation functions
// ---------------------------------------------------------------------------

fn gen_native_methods(api: &ApiSurface, namespace: &str, lib_name: &str, prefix: &str) -> String {
    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\
         using System.Runtime.InteropServices;\n\n",
    );

    out.push_str(&format!("namespace {};\n\n", namespace));

    out.push_str("internal static partial class NativeMethods\n{\n");
    out.push_str(&format!("    private const string LibName = \"{}\";\n\n", lib_name));

    // Track emitted C entry-point names to avoid duplicates when the same FFI
    // function appears both as a free function and as a type method.
    let mut emitted: HashSet<String> = HashSet::new();

    // Enum type names — these are NOT opaque handles and must not have from_json / to_json / free
    // helpers emitted for them.
    let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Collect opaque struct type names that appear as parameters or return types so we can
    // emit their from_json / to_json / free P/Invoke helpers.
    // Enum types are excluded.
    let mut opaque_param_types: HashSet<String> = HashSet::new();
    let mut opaque_return_types: HashSet<String> = HashSet::new();

    for func in &api.functions {
        for param in &func.params {
            if let TypeRef::Named(name) = &param.ty {
                if !enum_names.contains(name) {
                    opaque_param_types.insert(name.clone());
                }
            }
        }
        if let TypeRef::Named(name) = &func.return_type {
            if !enum_names.contains(name) {
                opaque_return_types.insert(name.clone());
            }
        }
    }
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    if !enum_names.contains(name) {
                        opaque_param_types.insert(name.clone());
                    }
                }
            }
            if let TypeRef::Named(name) = &method.return_type {
                if !enum_names.contains(name) {
                    opaque_return_types.insert(name.clone());
                }
            }
        }
    }

    // Collect truly opaque types (is_opaque = true in IR) — these have no to_json/from_json FFI.
    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Emit from_json + free helpers for opaque types used as parameters.
    // Truly opaque handles (is_opaque = true) have no from_json — only free.
    // E.g. `htm_conversion_options_from_json(const char *json) -> HTMConversionOptions*`
    for type_name in &opaque_param_types {
        let snake = type_name.to_snake_case();
        if !true_opaque_types.contains(type_name) {
            let from_json_entry = format!("{prefix}_{snake}_from_json");
            let from_json_cs = format!("{}FromJson", type_name.to_pascal_case());
            if emitted.insert(from_json_entry.clone()) {
                out.push_str(&format!(
                    "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{from_json_entry}\")]\n"
                ));
                out.push_str(&format!(
                    "    internal static extern IntPtr {from_json_cs}([MarshalAs(UnmanagedType.LPStr)] string json);\n\n"
                ));
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", type_name.to_pascal_case());
        if emitted.insert(free_entry.clone()) {
            out.push_str(&format!(
                "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{free_entry}\")]\n"
            ));
            out.push_str(&format!("    internal static extern void {free_cs}(IntPtr ptr);\n\n"));
        }
    }

    // Emit to_json + free helpers for opaque types returned from functions.
    // Truly opaque handles (is_opaque = true) have no to_json — only free.
    for type_name in &opaque_return_types {
        let snake = type_name.to_snake_case();
        if !true_opaque_types.contains(type_name) {
            let to_json_entry = format!("{prefix}_{snake}_to_json");
            let to_json_cs = format!("{}ToJson", type_name.to_pascal_case());
            if emitted.insert(to_json_entry.clone()) {
                out.push_str(&format!(
                    "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{to_json_entry}\")]\n"
                ));
                out.push_str(&format!(
                    "    internal static extern IntPtr {to_json_cs}(IntPtr ptr);\n\n"
                ));
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", type_name.to_pascal_case());
        if emitted.insert(free_entry.clone()) {
            out.push_str(&format!(
                "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{free_entry}\")]\n"
            ));
            out.push_str(&format!("    internal static extern void {free_cs}(IntPtr ptr);\n\n"));
        }
    }

    // Generate P/Invoke declarations for functions
    for func in &api.functions {
        let c_func_name = format!("{}_{}", prefix, func.name.to_lowercase());
        if emitted.insert(c_func_name.clone()) {
            out.push_str(&gen_pinvoke_for_func(&c_func_name, func));
        }
    }

    // Generate P/Invoke declarations for type methods
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            let c_method_name = format!("{}_{}_{}", prefix, type_snake, method.name.to_lowercase());
            // Use a type-prefixed C# method name to avoid collisions when different types
            // share a method with the same name (e.g. BrowserConfig::default and CrawlConfig::default
            // would both produce "Default" without the prefix, but have different FFI entry points).
            let cs_method_name = format!("{}{}", typ.name.to_pascal_case(), to_csharp_name(&method.name));
            if emitted.insert(c_method_name.clone()) {
                out.push_str(&gen_pinvoke_for_method(&c_method_name, &cs_method_name, method));
            }
        }
    }

    // Add error handling functions with PascalCase names
    out.push_str(&format!(
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_last_error_code\")]\n"
    ));
    out.push_str("    internal static extern int LastErrorCode();\n\n");

    out.push_str(&format!(
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_last_error_context\")]\n"
    ));
    out.push_str("    internal static extern IntPtr LastErrorContext();\n\n");

    out.push_str(&format!(
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_free_string\")]\n"
    ));
    out.push_str("    internal static extern void FreeString(IntPtr ptr);\n");

    out.push_str("}\n");

    out
}

fn gen_pinvoke_for_func(c_name: &str, func: &FunctionDef) -> String {
    let cs_name = to_csharp_name(&func.name);
    let mut out =
        format!("    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{c_name}\")]\n");
    out.push_str("    internal static extern ");

    // Return type — use the correct P/Invoke type for each kind.
    out.push_str(pinvoke_return_type(&func.return_type));

    out.push_str(&format!(" {}(", cs_name));

    if func.params.is_empty() {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        for (i, param) in func.params.iter().enumerate() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("{pinvoke_ty} {param_name}"));

            if i < func.params.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

fn gen_pinvoke_for_method(c_name: &str, cs_name: &str, method: &MethodDef) -> String {
    let mut out =
        format!("    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{c_name}\")]\n");
    out.push_str("    internal static extern ");

    // Return type — use the correct P/Invoke type for each kind.
    out.push_str(pinvoke_return_type(&method.return_type));

    out.push_str(&format!(" {}(", cs_name));

    if method.params.is_empty() {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        for (i, param) in method.params.iter().enumerate() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("{pinvoke_ty} {param_name}"));

            if i < method.params.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

fn gen_exception_class(namespace: &str, class_name: &str) -> String {
    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\n",
    );

    out.push_str(&format!("namespace {};\n\n", namespace));

    out.push_str(&format!("public class {} : Exception\n", class_name));
    out.push_str("{\n");
    out.push_str("    public int Code { get; }\n\n");
    out.push_str(&format!(
        "    public {}(int code, string message) : base(message)\n",
        class_name
    ));
    out.push_str("    {\n");
    out.push_str("        Code = code;\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}

fn gen_wrapper_class(
    api: &ApiSurface,
    namespace: &str,
    class_name: &str,
    exception_name: &str,
    prefix: &str,
) -> String {
    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\
         using System.Collections.Generic;\n\
         using System.Runtime.InteropServices;\n\
         using System.Text.Json;\n\
         using System.Text.Json.Serialization;\n\
         using System.Threading.Tasks;\n\n",
    );

    out.push_str(&format!("namespace {};\n\n", namespace));

    out.push_str(&format!("public static class {}\n", class_name));
    out.push_str("{\n");
    out.push_str("    private static readonly JsonSerializerOptions JsonOptions = new()\n");
    out.push_str("    {\n");
    out.push_str("        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },\n");
    out.push_str("        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingDefault\n");
    out.push_str("    };\n\n");

    // Enum names: used to distinguish opaque struct handles from enum return types.
    let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.to_pascal_case()).collect();

    // Truly opaque types (is_opaque = true) — returned/passed as handles, no JSON serialization.
    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Generate wrapper methods for functions
    for func in &api.functions {
        out.push_str(&gen_wrapper_function(
            func,
            exception_name,
            prefix,
            &enum_names,
            &true_opaque_types,
        ));
    }

    // Generate wrapper methods for type methods (prefixed with type name to avoid collisions)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        // Skip opaque types — their methods belong on the opaque handle class, not the static wrapper
        if typ.is_opaque {
            continue;
        }
        for method in &typ.methods {
            out.push_str(&gen_wrapper_method(
                method,
                exception_name,
                prefix,
                &typ.name,
                &enum_names,
                &true_opaque_types,
            ));
        }
    }

    // Add error handling helper
    out.push_str("    private static ");
    out.push_str(&format!("{} GetLastError()\n", exception_name));
    out.push_str("    {\n");
    out.push_str("        var code = NativeMethods.LastErrorCode();\n");
    out.push_str("        var ctxPtr = NativeMethods.LastErrorContext();\n");
    out.push_str("        var message = Marshal.PtrToStringAnsi(ctxPtr) ?? \"Unknown error\";\n");
    out.push_str(&format!("        return new {}(code, message);\n", exception_name));
    out.push_str("    }\n");

    out.push_str("}\n");

    out
}

// ---------------------------------------------------------------------------
// Helpers: Named-param setup/teardown for opaque handle marshalling
// ---------------------------------------------------------------------------

/// For each `Named` parameter, emit code to serialise it to JSON and obtain a native handle.
///
/// For truly opaque types (is_opaque = true), the C# class already wraps the native handle, so
/// we pass `param.Handle` directly without any JSON serialisation.
///
/// ```text
/// // Data struct (has from_json):
/// var optionsJson = JsonSerializer.Serialize(options);
/// var optionsHandle = NativeMethods.ConversionOptionsFromJson(optionsJson);
///
/// // Truly opaque handle: passed as engineHandle.Handle directly — no setup needed.
/// ```
fn emit_named_param_setup(
    out: &mut String,
    params: &[alef_core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
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
                if param.optional {
                    out.push_str(&format!(
                        "{indent}var {json_var} = {param_name} != null ? JsonSerializer.Serialize({param_name}, JsonOptions) : \"null\";\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "{indent}var {json_var} = JsonSerializer.Serialize({param_name}, JsonOptions);\n"
                    ));
                }
                out.push_str(&format!(
                    "{indent}var {handle_var} = NativeMethods.{from_json_method}({json_var});\n"
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Vec/Map: serialize to JSON string, marshal to native pointer
                out.push_str(&format!(
                    "{indent}var {json_var} = JsonSerializer.Serialize({param_name}, JsonOptions);\n"
                ));
                out.push_str(&format!(
                    "{indent}var {handle_var} = Marshal.StringToHGlobalAnsi({json_var});\n"
                ));
            }
            _ => {}
        }
    }
}

/// Returns the argument expression to pass to the native method for a given parameter.
///
/// For truly opaque types (is_opaque = true), the C# class wraps an IntPtr; pass `.Handle`.
/// For data-struct `Named` types this is the handle variable (e.g. `optionsHandle`).
/// For everything else it is the parameter name (with `!` for optional).
fn native_call_arg(ty: &TypeRef, param_name: &str, optional: bool, true_opaque_types: &HashSet<String>) -> String {
    match ty {
        TypeRef::Named(type_name) if true_opaque_types.contains(type_name) => {
            // Truly opaque: unwrap the IntPtr from the C# handle class.
            let bang = if optional { "!" } else { "" };
            format!("{param_name}{bang}.Handle")
        }
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{param_name}Handle")
        }
        ty => {
            if optional {
                // For optional primitive types (e.g. ulong?, uint?), the `!` null-forgiving
                // operator does not coerce a nullable to non-nullable at the type level —
                // only `.Value` does. String/Char/Path/Json are reference types so `!` is
                // correct for those; all value-type primitives need `.Value`.
                let needs_value_unwrap = matches!(
                    ty,
                    TypeRef::Primitive(_) | TypeRef::Duration
                );
                if needs_value_unwrap {
                    format!("{param_name}.Value")
                } else {
                    format!("{param_name}!")
                }
            } else {
                param_name.to_string()
            }
        }
    }
}

/// Emit cleanup code to free native handles allocated for `Named` parameters.
///
/// Truly opaque handles (is_opaque = true) are NOT freed here — their lifetime is managed by
/// the C# wrapper class (IDisposable). Only data-struct handles (from_json-allocated) are freed.
fn emit_named_param_teardown(
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
                out.push_str(&format!("        NativeMethods.{free_method}({handle_var});\n"));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&format!("        Marshal.FreeHGlobal({handle_var});\n"));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code with configurable indentation (used inside `Task.Run` lambdas).
fn emit_named_param_teardown_indented(
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
                out.push_str(&format!("{indent}NativeMethods.{free_method}({handle_var});\n"));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&format!("{indent}Marshal.FreeHGlobal({handle_var});\n"));
            }
            _ => {}
        }
    }
}

fn gen_wrapper_function(
    func: &FunctionDef,
    _exception_name: &str,
    _prefix: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(1024);

    // XML doc comment
    if !func.doc.is_empty() {
        out.push_str("    /// <summary>\n");
        for line in func.doc.lines() {
            out.push_str(&format!("    /// {}\n", line));
        }
        out.push_str("    /// </summary>\n");
        for param in &func.params {
            out.push_str(&format!(
                "    /// <param name=\"{}\">{}</param>\n",
                param.name.to_lower_camel_case(),
                if param.optional { "Optional." } else { "" }
            ));
        }
    }

    out.push_str("    public static ");

    // Return type — use async Task<T> for async methods
    if func.is_async {
        if func.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            out.push_str(&format!("async Task<{}>", csharp_type(&func.return_type)));
        }
    } else if func.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&func.return_type));
    }

    out.push_str(&format!(" {}", to_csharp_name(&func.name)));
    out.push('(');

    // Parameters
    for (i, param) in func.params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let mapped = csharp_type(&param.ty);
        if param.optional && !mapped.ends_with('?') {
            out.push_str(&format!("{mapped}? {param_name}"));
        } else {
            out.push_str(&format!("{mapped} {param_name}"));
        }

        if i < func.params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    // Null checks for required string/object parameters
    for param in &func.params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("        ArgumentNullException.ThrowIfNull({param_name});\n"));
        }
    }

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(&mut out, &func.params, "        ", true_opaque_types);

    // Method body - delegation to native method with proper marshalling
    let cs_native_name = to_csharp_name(&func.name);

    if func.is_async {
        // Async: wrap in Task.Run for non-blocking execution
        out.push_str("        return await Task.Run(() =>\n        {\n");

        if func.return_type != TypeRef::Unit {
            out.push_str("            var result = ");
        } else {
            out.push_str("            ");
        }

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if func.params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in func.params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("                {arg}"));
                if i < func.params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        // Check for FFI error (null result means the call failed).
        if func.return_type != TypeRef::Unit {
            out.push_str(
                "            if (result == IntPtr.Zero) { var err = GetLastError(); if (err.Code != 0) throw err; }\n",
            );
        }

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            "            ",
            enum_names,
            true_opaque_types,
        );
        emit_named_param_teardown_indented(&mut out, &func.params, "            ", true_opaque_types);
        emit_return_statement_indented(&mut out, &func.return_type, "            ");
        out.push_str("        });\n");
    } else {
        if func.return_type != TypeRef::Unit {
            out.push_str("        var result = ");
        } else {
            out.push_str("        ");
        }

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if func.params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in func.params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("            {arg}"));
                if i < func.params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("        );\n");
        }

        // Check for FFI error (null result means the call failed).
        if func.return_type != TypeRef::Unit {
            out.push_str(
                "        if (result == IntPtr.Zero) { var err = GetLastError(); if (err.Code != 0) throw err; }\n",
            );
        }

        emit_return_marshalling(&mut out, &func.return_type, enum_names, true_opaque_types);
        emit_named_param_teardown(&mut out, &func.params, true_opaque_types);
        emit_return_statement(&mut out, &func.return_type);
    }

    out.push_str("    }\n\n");

    out
}

fn gen_wrapper_method(
    method: &MethodDef,
    _exception_name: &str,
    _prefix: &str,
    type_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(1024);

    // XML doc comment
    if !method.doc.is_empty() {
        out.push_str("    /// <summary>\n");
        for line in method.doc.lines() {
            out.push_str(&format!("    /// {}\n", line));
        }
        out.push_str("    /// </summary>\n");
        for param in &method.params {
            out.push_str(&format!(
                "    /// <param name=\"{}\">{}</param>\n",
                param.name.to_lower_camel_case(),
                if param.optional { "Optional." } else { "" }
            ));
        }
    }

    // The wrapper class is always `static class`, so all methods must be static.
    out.push_str("    public static ");

    // Return type — use async Task<T> for async methods
    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            out.push_str(&format!("async Task<{}>", csharp_type(&method.return_type)));
        }
    } else if method.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&method.return_type));
    }

    // Prefix method name with type name to avoid collisions (e.g., MetadataConfigDefault)
    let method_cs_name = format!("{}{}", type_name, to_csharp_name(&method.name));
    out.push_str(&format!(" {method_cs_name}"));
    out.push('(');

    // Parameters
    for (i, param) in method.params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let mapped = csharp_type(&param.ty);
        if param.optional && !mapped.ends_with('?') {
            out.push_str(&format!("{mapped}? {param_name}"));
        } else {
            out.push_str(&format!("{mapped} {param_name}"));
        }

        if i < method.params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    // Null checks for required string/object parameters
    for param in &method.params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("        ArgumentNullException.ThrowIfNull({param_name});\n"));
        }
    }

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(&mut out, &method.params, "        ", true_opaque_types);

    // Method body - delegation to native method with proper marshalling.
    // Use the type-prefixed name to match the P/Invoke declaration, which includes the type
    // name to avoid collisions between different types with identically-named methods
    // (e.g. BrowserConfig::default and CrawlConfig::default).
    let cs_native_name = format!("{}{}", type_name.to_pascal_case(), to_csharp_name(&method.name));

    if method.is_async {
        // Async: wrap in Task.Run for non-blocking execution
        out.push_str("        return await Task.Run(() =>\n        {\n");

        if method.return_type != TypeRef::Unit {
            out.push_str("            var result = ");
        } else {
            out.push_str("            ");
        }

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if method.params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in method.params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("                {arg}"));
                if i < method.params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "            ",
            enum_names,
            true_opaque_types,
        );
        emit_named_param_teardown_indented(&mut out, &method.params, "            ", true_opaque_types);
        emit_return_statement_indented(&mut out, &method.return_type, "            ");
        out.push_str("        });\n");
    } else {
        if method.return_type != TypeRef::Unit {
            out.push_str("        var result = ");
        } else {
            out.push_str("        ");
        }

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if method.params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in method.params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("            {arg}"));
                if i < method.params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("        );\n");
        }

        emit_return_marshalling(&mut out, &method.return_type, enum_names, true_opaque_types);
        emit_named_param_teardown(&mut out, &method.params, true_opaque_types);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n\n");

    out
}

/// Emit the return-value marshalling code shared by both function and method wrappers.
///
/// This function emits the code to convert the raw P/Invoke `result` into the managed return
/// type and store it in a local variable `returnValue`.  It intentionally does **not** emit
/// the `return` statement so that callers can interpose cleanup (param handle teardown) between
/// the value computation and the return.
///
/// `enum_names`: the set of C# type names that are enums (not opaque handles).
/// `true_opaque_types`: types with `is_opaque = true` — wrapped in `new CsType(result)`.
///
/// Callers must invoke `emit_return_statement` after their cleanup to complete the method body.
fn emit_return_marshalling(
    out: &mut String,
    return_type: &TypeRef,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) {
    if *return_type == TypeRef::Unit {
        // void — nothing to return
        return;
    }

    if returns_string(return_type) {
        // IntPtr → string, then free the native buffer.
        out.push_str("        var returnValue = Marshal.PtrToStringUTF8(result) ?? string.Empty;\n");
        out.push_str("        NativeMethods.FreeString(result);\n");
    } else if returns_bool_via_int(return_type) {
        // C int → bool
        out.push_str("        var returnValue = result != 0;\n");
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = type_name.to_pascal_case();
        if true_opaque_types.contains(type_name) {
            // Truly opaque handle: wrap the IntPtr in the C# handle class.
            out.push_str(&format!("        var returnValue = new {pascal}(result);\n"));
        } else if !enum_names.contains(&pascal) {
            // Data struct with to_json: call to_json, deserialise, then free both.
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!(
                "        var jsonPtr = NativeMethods.{to_json_method}(result);\n"
            ));
            out.push_str("        var json = Marshal.PtrToStringUTF8(jsonPtr);\n");
            out.push_str("        NativeMethods.FreeString(jsonPtr);\n");
            out.push_str(&format!("        NativeMethods.{free_method}(result);\n"));
            out.push_str(&format!(
                "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        } else {
            // Enum returned as JSON string IntPtr.
            let cs_ty = csharp_type(return_type);
            out.push_str("        var json = Marshal.PtrToStringUTF8(result);\n");
            out.push_str("        NativeMethods.FreeString(result);\n");
            out.push_str(&format!(
                "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        }
    } else if returns_json_object(return_type) {
        // IntPtr → JSON string → deserialized object, then free the native buffer.
        let cs_ty = csharp_type(return_type);
        out.push_str("        var json = Marshal.PtrToStringUTF8(result);\n");
        out.push_str("        NativeMethods.FreeString(result);\n");
        out.push_str(&format!(
            "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
            cs_ty
        ));
    } else {
        // Numeric primitives — direct return.
        out.push_str("        var returnValue = result;\n");
    }
}

/// Emit the final `return returnValue;` statement after cleanup.
fn emit_return_statement(out: &mut String, return_type: &TypeRef) {
    if *return_type != TypeRef::Unit {
        out.push_str("        return returnValue;\n");
    }
}

/// Emit the return-value marshalling code with configurable indentation.
///
/// Like `emit_return_marshalling` this stores the value in `returnValue` without emitting
/// the final `return` statement.  Callers must call `emit_return_statement_indented` after.
fn emit_return_marshalling_indented(
    out: &mut String,
    return_type: &TypeRef,
    indent: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) {
    if *return_type == TypeRef::Unit {
        return;
    }

    if returns_string(return_type) {
        out.push_str(&format!(
            "{indent}var returnValue = Marshal.PtrToStringUTF8(result) ?? string.Empty;\n"
        ));
        out.push_str(&format!("{indent}NativeMethods.FreeString(result);\n"));
    } else if returns_bool_via_int(return_type) {
        out.push_str(&format!("{indent}var returnValue = result != 0;\n"));
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = type_name.to_pascal_case();
        if true_opaque_types.contains(type_name) {
            // Truly opaque handle: wrap the IntPtr in the C# handle class.
            out.push_str(&format!("{indent}var returnValue = new {pascal}(result);\n"));
        } else if !enum_names.contains(&pascal) {
            // Data struct with to_json: call to_json, deserialise, then free both.
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!(
                "{indent}var jsonPtr = NativeMethods.{to_json_method}(result);\n"
            ));
            out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(jsonPtr);\n"));
            out.push_str(&format!("{indent}NativeMethods.FreeString(jsonPtr);\n"));
            out.push_str(&format!("{indent}NativeMethods.{free_method}(result);\n"));
            out.push_str(&format!(
                "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        } else {
            // Enum returned as JSON string IntPtr.
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(result);\n"));
            out.push_str(&format!("{indent}NativeMethods.FreeString(result);\n"));
            out.push_str(&format!(
                "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        }
    } else if returns_json_object(return_type) {
        let cs_ty = csharp_type(return_type);
        out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(result);\n"));
        out.push_str(&format!("{indent}NativeMethods.FreeString(result);\n"));
        out.push_str(&format!(
            "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
            cs_ty
        ));
    } else {
        out.push_str(&format!("{indent}var returnValue = result;\n"));
    }
}

/// Emit the final `return returnValue;` with configurable indentation.
fn emit_return_statement_indented(out: &mut String, return_type: &TypeRef, indent: &str) {
    if *return_type != TypeRef::Unit {
        out.push_str(&format!("{indent}return returnValue;\n"));
    }
}

fn gen_opaque_handle(typ: &TypeDef, namespace: &str) -> String {
    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\n",
    );

    out.push_str(&format!("namespace {};\n\n", namespace));

    // Generate doc comment if available
    if !typ.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in typ.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    let class_name = typ.name.to_pascal_case();
    out.push_str(&format!("public sealed class {} : IDisposable\n", class_name));
    out.push_str("{\n");
    out.push_str("    internal IntPtr Handle { get; }\n\n");
    out.push_str(&format!("    internal {}(IntPtr handle)\n", class_name));
    out.push_str("    {\n");
    out.push_str("        Handle = handle;\n");
    out.push_str("    }\n\n");
    out.push_str("    public void Dispose()\n");
    out.push_str("    {\n");
    out.push_str("        // Native free will be called by the runtime\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}

fn gen_record_type(
    typ: &TypeDef,
    namespace: &str,
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    custom_converter_enums: &HashSet<String>,
    _lang_rename_all: &str,
) -> String {
    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\
         using System.Collections.Generic;\n\
         using System.Text.Json;\n\
         using System.Text.Json.Serialization;\n\n",
    );

    out.push_str(&format!("namespace {};\n\n", namespace));

    // Generate doc comment if available
    if !typ.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in typ.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    out.push_str(&format!("public sealed class {}\n", typ.name.to_pascal_case()));
    out.push_str("{\n");

    for field in &typ.fields {
        // Skip unnamed tuple struct fields (e.g., _0, _1, 0, 1, etc.)
        if is_tuple_field(field) {
            continue;
        }

        // Doc comment for field
        if !field.doc.is_empty() {
            out.push_str("    /// <summary>\n");
            for line in field.doc.lines() {
                out.push_str(&format!("    /// {}\n", line));
            }
            out.push_str("    /// </summary>\n");
        }

        // If the field's type is an enum with a custom converter, emit a property-level
        // [JsonConverter] attribute. This ensures the custom converter takes precedence
        // over the global JsonStringEnumConverter registered in JsonSerializerOptions.
        let field_base_type = match &field.ty {
            TypeRef::Named(n) => Some(n.to_pascal_case()),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.to_pascal_case()),
                _ => None,
            },
            _ => None,
        };
        if let Some(ref base) = field_base_type {
            if custom_converter_enums.contains(base) {
                out.push_str(&format!("    [JsonConverter(typeof({base}JsonConverter))]\n"));
            }
        }

        // [JsonPropertyName("json_name")]
        // FFI-based languages serialize to JSON that Rust serde deserializes.
        // Since Rust uses default snake_case, JSON property names must be snake_case.
        let json_name = field.name.clone();
        out.push_str(&format!("    [JsonPropertyName(\"{}\")]\n", json_name));

        let cs_name = to_csharp_name(&field.name);

        // Check if field type is a complex enum (tagged enum with data variants).
        // These can't be simple C# enums — use JsonElement for flexible deserialization.
        let is_complex = matches!(&field.ty, TypeRef::Named(n) if complex_enums.contains(&n.to_pascal_case()));

        if field.optional {
            // Optional fields: nullable type, no `required`, default = null
            let mapped = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };
            let field_type = if mapped.ends_with('?') {
                mapped
            } else {
                format!("{mapped}?")
            };
            out.push_str(&format!("    public {} {} {{ get; set; }}", field_type, cs_name));
            out.push_str(" = null;\n");
        } else if typ.has_default || field.default.is_some() {
            // Field with an explicit default value or part of a type with defaults.
            // Use typed_default from IR to get Rust-compatible defaults.
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };
            out.push_str(&format!("    public {} {} {{ get; set; }}", field_type, cs_name));
            use alef_core::ir::DefaultValue;
            // Duration fields are mapped to ulong? so that 0 is distinguishable from
            // "not set". Always default to null here; Rust has its own default.
            if matches!(&field.ty, TypeRef::Duration) {
                out.push_str(" = null;\n");
                out.push('\n');
                continue;
            }
            let default_val = match &field.typed_default {
                Some(DefaultValue::BoolLiteral(b)) => b.to_string(),
                Some(DefaultValue::IntLiteral(n)) => n.to_string(),
                Some(DefaultValue::FloatLiteral(f)) => {
                    let s = f.to_string();
                    if s.contains('.') { s } else { format!("{s}.0") }
                }
                Some(DefaultValue::StringLiteral(s)) => format!("\"{}\"", s.replace('"', "\\\"")),
                Some(DefaultValue::EnumVariant(v)) => format!("{}.{}", field_type, v.to_pascal_case()),
                Some(DefaultValue::None) => "null".to_string(),
                Some(DefaultValue::Empty) | None => match &field.ty {
                    TypeRef::Vec(_) => "[]".to_string(),
                    TypeRef::Map(k, v) => format!("new Dictionary<{}, {}>()", csharp_type(k), csharp_type(v)),
                    TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "Array.Empty<byte>()".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => "false".to_string(),
                        PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
                        _ => "0".to_string(),
                    },
                    TypeRef::Named(name) => {
                        let pascal = name.to_pascal_case();
                        if enum_names.contains(&pascal) {
                            "default".to_string()
                        } else {
                            "default!".to_string()
                        }
                    }
                    _ => "default!".to_string(),
                },
            };
            out.push_str(&format!(" = {};\n", default_val));
        } else {
            // Non-optional field without explicit default.
            // Use type-appropriate zero values instead of `required` to avoid
            // JSON deserialization failures when fields are omitted via serde skip_serializing_if.
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };
            // Duration is mapped to ulong? so null is the correct "not set" default.
            if matches!(&field.ty, TypeRef::Duration) {
                out.push_str(&format!(
                    "    public {} {} {{ get; set; }} = null;\n",
                    field_type, cs_name
                ));
            } else {
                let default_val = match &field.ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"",
                    TypeRef::Vec(_) => "[]",
                    TypeRef::Bytes => "Array.Empty<byte>()",
                    TypeRef::Primitive(PrimitiveType::Bool) => "false",
                    TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "0.0",
                    TypeRef::Primitive(_) => "0",
                    _ => "default!",
                };
                out.push_str(&format!(
                    "    public {} {} {{ get; set; }} = {};\n",
                    field_type, cs_name, default_val
                ));
            }
        }

        out.push('\n');
    }

    out.push_str("}\n");

    out
}

/// Apply a serde `rename_all` strategy to a variant name.
fn apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => name.to_snake_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => name.to_snake_case().to_uppercase(),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        _ => name.to_lowercase(),
    }
}

fn gen_enum(enum_def: &EnumDef, namespace: &str) -> String {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate abstract record hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_tagged_union(enum_def, namespace);
    }

    // If any variant has an explicit serde_rename whose value differs from what
    // SnakeCaseLower would produce (e.g. "og:image" vs "og_image"), the global
    // JsonStringEnumConverter(SnakeCaseLower) in KreuzcrawlLib.JsonOptions would
    // ignore [JsonPropertyName] and use the naming policy instead.
    // Also, the non-generic JsonStringEnumConverter does NOT support [JsonPropertyName]
    // on enum members at all. For these cases we generate a custom JsonConverter<T>
    // that explicitly maps each variant name.
    let needs_custom_converter = enum_def.variants.iter().any(|v| {
        if let Some(ref rename) = v.serde_rename {
            let snake = apply_rename_all(&v.name, enum_def.serde_rename_all.as_deref());
            rename != &snake
        } else {
            false
        }
    });

    let enum_pascal = enum_def.name.to_pascal_case();

    // Collect (json_name, pascal_name) pairs
    let variants: Vec<(String, String)> = enum_def
        .variants
        .iter()
        .map(|v| {
            let json_name = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_rename_all(&v.name, enum_def.serde_rename_all.as_deref()));
            let pascal_name = v.name.to_pascal_case();
            (json_name, pascal_name)
        })
        .collect();

    let mut out = String::from("// This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str("using System;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&format!("namespace {};\n\n", namespace));

    // Generate doc comment if available
    if !enum_def.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in enum_def.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    if needs_custom_converter {
        out.push_str(&format!("[JsonConverter(typeof({enum_pascal}JsonConverter))]\n"));
    }
    out.push_str(&format!("public enum {enum_pascal}\n"));
    out.push_str("{\n");

    for (json_name, pascal_name) in &variants {
        // Find doc for this variant
        if let Some(v) = enum_def
            .variants
            .iter()
            .find(|v| v.name.to_pascal_case() == *pascal_name)
        {
            if !v.doc.is_empty() {
                out.push_str("    /// <summary>\n");
                for line in v.doc.lines() {
                    out.push_str(&format!("    /// {}\n", line));
                }
                out.push_str("    /// </summary>\n");
            }
        }
        out.push_str(&format!("    [JsonPropertyName(\"{json_name}\")]\n"));
        out.push_str(&format!("    {pascal_name},\n"));
    }

    out.push_str("}\n");

    // Generate custom converter class after the enum when needed
    if needs_custom_converter {
        out.push('\n');
        out.push_str(&format!(
            "/// <summary>Custom JSON converter for <see cref=\"{enum_pascal}\"/> that respects explicit variant names.</summary>\n"
        ));
        out.push_str(&format!(
            "internal sealed class {enum_pascal}JsonConverter : JsonConverter<{enum_pascal}>\n"
        ));
        out.push_str("{\n");

        // Read
        out.push_str(&format!(
            "    public override {enum_pascal} Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)\n"
        ));
        out.push_str("    {\n");
        out.push_str("        var value = reader.GetString();\n");
        out.push_str("        return value switch\n");
        out.push_str("        {\n");
        for (json_name, pascal_name) in &variants {
            out.push_str(&format!(
                "            \"{json_name}\" => {enum_pascal}.{pascal_name},\n"
            ));
        }
        out.push_str(&format!(
            "            _ => throw new JsonException($\"Unknown {enum_pascal} value: {{value}}\")\n"
        ));
        out.push_str("        };\n");
        out.push_str("    }\n\n");

        // Write
        out.push_str(&format!(
            "    public override void Write(Utf8JsonWriter writer, {enum_pascal} value, JsonSerializerOptions options)\n"
        ));
        out.push_str("    {\n");
        out.push_str("        var str = value switch\n");
        out.push_str("        {\n");
        for (json_name, pascal_name) in &variants {
            out.push_str(&format!(
                "            {enum_pascal}.{pascal_name} => \"{json_name}\",\n"
            ));
        }
        out.push_str(&format!(
            "            _ => throw new JsonException($\"Unknown {enum_pascal} value: {{value}}\")\n"
        ));
        out.push_str("        };\n");
        out.push_str("        writer.WriteStringValue(str);\n");
        out.push_str("    }\n");
        out.push_str("}\n");
    }

    out
}

/// Generate a C# abstract record hierarchy for internally tagged enums.
///
/// Maps `#[serde(tag = "type_field", rename_all = "snake_case")]` Rust enums to
/// a custom `JsonConverter<T>` that buffers all JSON properties before resolving
/// the discriminator. This is more robust than `[JsonPolymorphic]` which requires
/// the discriminator to be the first property in the JSON object.
fn gen_tagged_union(enum_def: &EnumDef, namespace: &str) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let enum_pascal = enum_def.name.to_pascal_case();
    let converter_name = format!("{enum_pascal}JsonConverter");
    // Namespace prefix used to fully-qualify inner types when their short name is shadowed
    // by a nested record of the same name (e.g. ContentPart.ImageUrl shadows ImageUrl).
    let ns = namespace;

    let mut out = String::from(
        "// This file is auto-generated by alef. DO NOT EDIT.\n\
         using System;\n\
         using System.Collections.Generic;\n\
         using System.Text.Json;\n\
         using System.Text.Json.Serialization;\n\n",
    );
    out.push_str(&format!("namespace {};\n\n", namespace));

    // Doc comment
    if !enum_def.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in enum_def.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    // Use custom converter instead of [JsonPolymorphic] to handle discriminator in any position
    out.push_str(&format!("[JsonConverter(typeof({converter_name}))]\n"));
    out.push_str(&format!("public abstract record {enum_pascal}\n"));
    out.push_str("{\n");

    // Nested sealed records for each variant
    for variant in &enum_def.variants {
        let pascal = variant.name.to_pascal_case();

        if !variant.doc.is_empty() {
            out.push_str("    /// <summary>\n");
            for line in variant.doc.lines() {
                out.push_str(&format!("    /// {}\n", line));
            }
            out.push_str("    /// </summary>\n");
        }

        if variant.fields.is_empty() {
            // Unit variant → sealed record with no fields
            out.push_str(&format!("    public sealed record {pascal}() : {enum_pascal};\n\n"));
        } else {
            // CS8910: when a single-field variant has a parameter whose TYPE equals the record name
            // (e.g., record ImageUrl(ImageUrl Value)), the primary constructor conflicts with the
            // synthesized copy constructor. Use a property-based record body instead.
            // This applies to both tuple fields and named fields that get renamed to "Value".
            let is_copy_ctor_clash = variant.fields.len() == 1 && {
                let field_cs_type = csharp_type(&variant.fields[0].ty);
                field_cs_type.as_ref() == pascal
            };

            if is_copy_ctor_clash {
                let cs_type = csharp_type(&variant.fields[0].ty);
                // Fully qualify the inner type to avoid the nested record shadowing the
                // standalone type of the same name (e.g. `ContentPart.ImageUrl` would shadow
                // `LiterLlm.ImageUrl` within the `ContentPart` abstract record body).
                let qualified_cs_type = format!("global::{ns}.{cs_type}");
                out.push_str(&format!("    public sealed record {pascal} : {enum_pascal}\n"));
                out.push_str("    {\n");
                out.push_str(&format!(
                    "        public required {qualified_cs_type} Value {{ get; init; }}\n"
                ));
                out.push_str("    }\n\n");
            } else {
                // Data variant → sealed record with fields as constructor params
                out.push_str(&format!("    public sealed record {pascal}(\n"));
                for (i, field) in variant.fields.iter().enumerate() {
                    let cs_type = csharp_type(&field.ty);
                    let cs_type = if field.optional && !cs_type.ends_with('?') {
                        format!("{cs_type}?")
                    } else {
                        cs_type.to_string()
                    };
                    let comma = if i < variant.fields.len() - 1 { "," } else { "" };
                    if is_tuple_field(field) {
                        out.push_str(&format!("        {cs_type} Value{comma}\n"));
                    } else {
                        let json_name = field.name.trim_start_matches('_');
                        let cs_name = to_csharp_name(json_name);
                        let clashes = cs_name == pascal || cs_name == cs_type;
                        if clashes {
                            out.push_str(&format!("        {cs_type} Value{comma}\n"));
                        } else {
                            out.push_str(&format!(
                                "        [property: JsonPropertyName(\"{json_name}\")] {cs_type} {cs_name}{comma}\n"
                            ));
                        }
                    }
                }
                out.push_str(&format!("    ) : {enum_pascal};\n\n"));
            }
        }
    }

    out.push_str("}\n\n");

    // Generate custom converter that buffers the JSON document before dispatching
    out.push_str(&format!(
        "/// <summary>Custom JSON converter for <see cref=\"{enum_pascal}\"/> that reads the \"{tag_field}\" discriminator from any position.</summary>\n"
    ));
    out.push_str(&format!(
        "internal sealed class {converter_name} : JsonConverter<{enum_pascal}>\n"
    ));
    out.push_str("{\n");

    // Read method
    out.push_str(&format!(
        "    public override {enum_pascal} Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)\n"
    ));
    out.push_str("    {\n");
    out.push_str("        using var doc = JsonDocument.ParseValue(ref reader);\n");
    out.push_str("        var root = doc.RootElement;\n");
    out.push_str(&format!(
        "        if (!root.TryGetProperty(\"{tag_field}\", out var tagEl))\n"
    ));
    out.push_str(&format!(
        "            throw new JsonException(\"{enum_pascal}: missing \\\"{tag_field}\\\" discriminator\");\n"
    ));
    out.push_str("        var tag = tagEl.GetString();\n");
    out.push_str("        var json = root.GetRawText();\n");
    out.push_str("        return tag switch\n");
    out.push_str("        {\n");

    for variant in &enum_def.variants {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        let pascal = variant.name.to_pascal_case();
        // Newtype/tuple variants have their inner type's fields inlined alongside the tag in JSON.
        // Deserialize the inner type from the full JSON object and wrap it in the record constructor.
        // Also treat single named-field variants whose parameter was renamed to "Value" (clash with
        // the variant name or the field's own type name) the same way.
        let is_tuple_newtype = variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]);
        let is_named_clash_newtype = variant.fields.len() == 1 && !is_tuple_field(&variant.fields[0]) && {
            let f = &variant.fields[0];
            let cs_type = csharp_type(&f.ty);
            let cs_name = to_csharp_name(f.name.trim_start_matches('_'));
            cs_name == pascal || cs_name == cs_type
        };
        let is_newtype = is_tuple_newtype || is_named_clash_newtype;
        if is_newtype {
            let inner_cs_type = csharp_type(&variant.fields[0].ty);
            // CS8910: when inner type name equals variant name, use object initializer
            // (no primary constructor exists — property-based record was emitted)
            if inner_cs_type == pascal {
                out.push_str(&format!(
                    "            \"{discriminator}\" => new {enum_pascal}.{pascal} {{ Value = JsonSerializer.Deserialize<{inner_cs_type}>(json, options)!\n"
                ));
                out.push_str(&format!(
                    "                ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}.Value\") }},\n"
                ));
            } else {
                out.push_str(&format!(
                    "            \"{discriminator}\" => new {enum_pascal}.{pascal}(\n"
                ));
                out.push_str(&format!(
                    "                JsonSerializer.Deserialize<{inner_cs_type}>(json, options)!\n"
                ));
                out.push_str(&format!(
                    "                    ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}.Value\")),\n"
                ));
            }
        } else {
            out.push_str(&format!(
                "            \"{discriminator}\" => JsonSerializer.Deserialize<{enum_pascal}.{pascal}>(json, options)!\n"
            ));
            out.push_str(&format!(
                "                ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}\"),\n"
            ));
        }
    }

    out.push_str(&format!(
        "            _ => throw new JsonException($\"Unknown {enum_pascal} discriminator: {{tag}}\")\n"
    ));
    out.push_str("        };\n");
    out.push_str("    }\n\n");

    // Write method
    out.push_str(&format!(
        "    public override void Write(Utf8JsonWriter writer, {enum_pascal} value, JsonSerializerOptions options)\n"
    ));
    out.push_str("    {\n");

    // Build options without this converter to avoid infinite recursion
    out.push_str("        // Serialize the concrete type, then inject the discriminator\n");
    out.push_str("        switch (value)\n");
    out.push_str("        {\n");

    for variant in &enum_def.variants {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        let pascal = variant.name.to_pascal_case();
        // Newtype/tuple variants: serialize the inner Value's fields inline alongside the tag.
        // Also applies to single named-field variants whose parameter was renamed to "Value" due
        // to a clash with the variant name or the field's own type name.
        let is_tuple_newtype = variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]);
        let is_named_clash_newtype = variant.fields.len() == 1 && !is_tuple_field(&variant.fields[0]) && {
            let f = &variant.fields[0];
            let cs_type = csharp_type(&f.ty);
            let cs_name = to_csharp_name(f.name.trim_start_matches('_'));
            cs_name == pascal || cs_name == cs_type
        };
        let is_newtype = is_tuple_newtype || is_named_clash_newtype;
        out.push_str(&format!("            case {enum_pascal}.{pascal} v:\n"));
        out.push_str("            {\n");
        if is_newtype {
            out.push_str("                var doc = JsonSerializer.SerializeToDocument(v.Value, options);\n");
        } else {
            out.push_str("                var doc = JsonSerializer.SerializeToDocument(v, options);\n");
        }
        out.push_str("                writer.WriteStartObject();\n");
        out.push_str(&format!(
            "                writer.WriteString(\"{tag_field}\", \"{discriminator}\");\n"
        ));
        out.push_str("                foreach (var prop in doc.RootElement.EnumerateObject())\n");
        out.push_str(&format!(
            "                    if (prop.Name != \"{tag_field}\") prop.WriteTo(writer);\n"
        ));
        out.push_str("                writer.WriteEndObject();\n");
        out.push_str("                break;\n");
        out.push_str("            }\n");
    }

    out.push_str(&format!(
        "            default: throw new JsonException($\"Unknown {enum_pascal} subtype: {{value.GetType().Name}}\");\n"
    ));
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}
