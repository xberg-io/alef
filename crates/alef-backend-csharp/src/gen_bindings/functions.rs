//! C# NativeMethods (P/Invoke) code generation.

use super::{is_bridge_param, pinvoke_param_type, pinvoke_return_type};
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
    use crate::template_env::render;
    use minijinja::Value;

    let mut out = render(
        "native_methods_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "lib_name": lib_name,
        })),
    );
    out.push('\n');

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

    // Opaque handle classes own native pointers via SafeHandle, so every true
    // opaque type needs a matching free declaration even if no public wrapper
    // function currently accepts or returns that handle.
    let mut sorted_true_opaque_types: Vec<&String> = true_opaque_types.iter().collect();
    sorted_true_opaque_types.sort();
    for type_name in sorted_true_opaque_types {
        let snake = type_name.to_snake_case();
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", type_name.to_pascal_case());
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

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
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &from_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_from_json.jinja",
                    minijinja::context! { cs_name => &from_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", type_name.to_pascal_case());
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
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
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &to_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_to_json.jinja",
                    minijinja::context! { cs_name => &to_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", type_name.to_pascal_case());
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
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
    let last_error_code_entry = format!("{prefix}_last_error_code");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_code_entry },
    ));
    out.push_str("    internal static extern int LastErrorCode();\n\n");

    let last_error_context_entry = format!("{prefix}_last_error_context");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_context_entry },
    ));
    out.push_str("    internal static extern IntPtr LastErrorContext();\n\n");

    let free_string_entry = format!("{prefix}_free_string");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &free_string_entry },
    ));
    out.push_str("    internal static extern void FreeString(IntPtr ptr);\n\n");

    let free_bytes_entry = format!("{prefix}_free_bytes");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &free_bytes_entry },
    ));
    out.push_str("    internal static extern void FreeBytes(IntPtr ptr, UIntPtr len, UIntPtr cap);\n");

    // Inject visitor create/free/convert P/Invoke declarations when a bridge is configured.
    if has_visitor_callbacks {
        out.push('\n');
        // Find the visitor trait bridge config to determine trait name and options field
        let visitor_bridge = trait_bridges
            .iter()
            .find(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField);

        if let Some(bridge) = visitor_bridge {
            out.push_str(&crate::gen_visitor::gen_native_methods_visitor(
                namespace,
                lib_name,
                prefix,
                &bridge.trait_name,
                bridge.options_field.as_deref().unwrap_or("visitor"),
            ));
        }
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

/// Returns true when a function returns `Result<Vec<u8>>` — uses the out-param
/// convention: `(args..., out IntPtr, out UIntPtr, out UIntPtr) -> int`.
pub(super) fn is_bytes_result_func(func: &FunctionDef) -> bool {
    func.error_type.is_some() && matches!(func.return_type, TypeRef::Bytes)
}

/// Same check for MethodDef.
pub(super) fn is_bytes_result_method(method: &MethodDef) -> bool {
    method.error_type.is_some() && matches!(method.return_type, TypeRef::Bytes)
}

pub(super) fn gen_pinvoke_for_func(
    c_name: &str,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    use crate::template_env::render;

    let cs_name = to_csharp_name(&func.name);
    let is_bytes_result = is_bytes_result_func(func);

    let mut out = render("dll_import_attr.jinja", minijinja::context! { entry_point => c_name });
    out.push_str("    internal static extern ");

    // Result<Vec<u8>> returns an i32 error code; output bytes come via out-params.
    if is_bytes_result {
        out.push_str("int");
    } else {
        out.push_str(pinvoke_return_type(&func.return_type));
    }

    out.push(' ');
    out.push_str(&cs_name);
    out.push('(');

    // Filter bridge params — they are not visible in P/Invoke declarations.
    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .collect();

    // For bytes_result: always need params block for the three out-params.
    if visible_params.is_empty() && !is_bytes_result {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        for param in visible_params.iter() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
        }
        if is_bytes_result {
            // Three trailing out-params for the byte-buffer out-param convention.
            out.push_str("        out IntPtr outPtr,\n");
            out.push_str("        out UIntPtr outLen,\n");
            out.push_str("        out UIntPtr outCap\n");
        } else {
            // Remove trailing comma from the last regular param.
            let trim_len = ",\n".len();
            out.truncate(out.len() - trim_len);
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

pub(super) fn gen_pinvoke_for_method(c_name: &str, cs_name: &str, method: &MethodDef) -> String {
    use crate::template_env::render;

    let is_bytes_result = is_bytes_result_method(method);

    let mut out = render("dll_import_attr.jinja", minijinja::context! { entry_point => c_name });
    out.push_str("    internal static extern ");

    // Result<Vec<u8>> returns an i32 error code; output bytes come via out-params.
    if is_bytes_result {
        out.push_str("int");
    } else {
        out.push_str(pinvoke_return_type(&method.return_type));
    }

    out.push(' ');
    out.push_str(cs_name);
    out.push('(');

    // Non-static methods take the receiver as the first FFI parameter.
    let has_receiver = !method.is_static && method.receiver.is_some();

    let needs_params = has_receiver || !method.params.is_empty() || is_bytes_result;
    if !needs_params {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        if has_receiver {
            out.push_str("        IntPtr handle,\n");
        }
        for param in method.params.iter() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPStr)] ");
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
        }
        if is_bytes_result {
            // Three trailing out-params for the byte-buffer out-param convention.
            out.push_str("        out IntPtr outPtr,\n");
            out.push_str("        out UIntPtr outLen,\n");
            out.push_str("        out UIntPtr outCap\n");
        } else {
            // Remove trailing comma from the last param.
            let trim_len = ",\n".len();
            out.truncate(out.len() - trim_len);
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}
