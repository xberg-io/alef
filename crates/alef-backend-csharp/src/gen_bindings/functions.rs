//! C# NativeMethods (P/Invoke) code generation.

use super::{csharp_file_header, is_bridge_param, pinvoke_param_type, pinvoke_return_type};
use alef_codegen::naming::to_csharp_name;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_native_methods(
    api: &ApiSurface,
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
    trait_bridges: &[TraitBridgeConfig],
    streaming_methods: &HashSet<String>,
    exclude_functions: &HashSet<String>,
) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Runtime.InteropServices;\n\n");

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

    // Enums passed as parameters in any FFI function flow through *_from_json + *_free
    // (the alef-backend-ffi side now emits these for param-passed enums). Treat them
    // like opaque struct params so the DllImport entries get generated.
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        for param in &func.params {
            if let TypeRef::Named(name) = &param.ty {
                opaque_param_types.insert(name.clone());
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
            if streaming_methods.contains(&method.name) {
                continue;
            }
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    opaque_param_types.insert(name.clone());
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
    let mut sorted_param_types: Vec<&String> = opaque_param_types.iter().collect();
    sorted_param_types.sort();
    for type_name in sorted_param_types {
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
    let mut sorted_return_types: Vec<&String> = opaque_return_types.iter().collect();
    sorted_return_types.sort();
    for type_name in sorted_return_types {
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
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        let c_func_name = format!("{}_{}", prefix, func.name.to_lowercase());
        if emitted.insert(c_func_name.clone()) {
            out.push_str(&gen_pinvoke_for_func(
                &c_func_name,
                func,
                bridge_param_names,
                bridge_type_aliases,
            ));
        }
    }

    // Generate P/Invoke declarations for type methods.
    // Skip streaming adapter methods — their FFI signature uses callbacks that P/Invoke can't call.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
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

    // Inject visitor create/free/convert P/Invoke declarations when a bridge is configured.
    if has_visitor_callbacks {
        out.push('\n');
        out.push_str(&crate::gen_visitor::gen_native_methods_visitor(
            namespace, lib_name, prefix,
        ));
    }

    // Inject trait bridge registration/unregistration P/Invoke declarations.
    if !trait_bridges.is_empty() {
        // Collect trait definitions from api.types (by name) to match with trait_bridges config
        let trait_defs: Vec<_> = api.types.iter().filter(|t| t.is_trait).collect();

        // Build a list of (trait_name, bridge_config, trait_def) tuples for trait bridges
        let bridges: Vec<_> = trait_bridges
            .iter()
            .filter_map(|config| {
                let trait_name = config.trait_name.clone();
                trait_defs
                    .iter()
                    .find(|t| t.name == trait_name)
                    .map(|trait_def| (trait_name, config, *trait_def))
            })
            .collect();

        if !bridges.is_empty() {
            out.push('\n');
            out.push_str(&crate::trait_bridge::gen_native_methods_trait_bridges(
                namespace, prefix, &bridges,
            ));
        }
    }

    out.push_str("}\n");

    out
}

pub(super) fn gen_pinvoke_for_func(
    c_name: &str,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    let cs_name = to_csharp_name(&func.name);
    let mut out =
        format!("    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{c_name}\")]\n");
    out.push_str("    internal static extern ");

    // Return type — use the correct P/Invoke type for each kind.
    out.push_str(pinvoke_return_type(&func.return_type));

    out.push_str(&format!(" {}(", cs_name));

    // Filter bridge params — they are not visible in P/Invoke declarations; the wrapper
    // passes IntPtr.Zero directly when calling the visitor-less FFI entry point.
    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .collect();

    if visible_params.is_empty() {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        for (i, param) in visible_params.iter().enumerate() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("{pinvoke_ty} {param_name}"));

            if i < visible_params.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

pub(super) fn gen_pinvoke_for_method(c_name: &str, cs_name: &str, method: &MethodDef) -> String {
    let mut out =
        format!("    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{c_name}\")]\n");
    out.push_str("    internal static extern ");

    // Return type — use the correct P/Invoke type for each kind.
    out.push_str(pinvoke_return_type(&method.return_type));

    out.push_str(&format!(" {}(", cs_name));

    // Non-static methods take the receiver as the first FFI parameter (the
    // generated extern "C" fn signature is `fn (this: *const T, ...)`). Prepend
    // an `IntPtr handle` here so the P/Invoke signature matches; without this
    // the C# wrapper falls one argument short and the runtime throws
    // EntryPointNotFoundException / the C# compiler rejects the call site.
    let has_receiver = !method.is_static && method.receiver.is_some();

    if !has_receiver && method.params.is_empty() {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        let total = if has_receiver {
            method.params.len() + 1
        } else {
            method.params.len()
        };
        let mut idx = 0usize;
        if has_receiver {
            out.push_str("        IntPtr handle");
            if total > 1 {
                out.push(',');
            }
            out.push('\n');
            idx += 1;
        }
        for param in method.params.iter() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("{pinvoke_ty} {param_name}"));

            if idx < total - 1 {
                out.push(',');
            }
            out.push('\n');
            idx += 1;
        }
        out.push_str("    );\n\n");
    }

    out
}
