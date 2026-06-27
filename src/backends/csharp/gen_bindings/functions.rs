//! C# NativeMethods (P/Invoke) code generation.

use super::{StreamingMethodMeta, is_bridge_param, pinvoke_param_type, pinvoke_return_type};
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, PrimitiveType, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Map a Rust FFI type string to the C# P/Invoke parameter declaration.
///
/// String parameters use explicit UTF-8 marshalling to match the C `const char*` ABI.
fn ffi_ty_to_pinvoke_param(rust_ty: &str, param_name: &str) -> String {
    let normalized = rust_ty.trim();
    let cs_name = param_name.to_lower_camel_case();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return format!("[MarshalAs(UnmanagedType.LPUTF8Str)] string {cs_name}");
    }
    let cs_type = match normalized {
        "bool" => "bool",
        "u8" | "uint8_t" => "byte",
        "u16" | "uint16_t" => "ushort",
        "u32" | "uint32_t" => "uint",
        "u64" | "uint64_t" | "usize" => "ulong",
        "i8" | "int8_t" => "sbyte",
        "i16" | "int16_t" => "short",
        "i32" | "int32_t" | "c_int" => "int",
        "i64" | "int64_t" | "isize" => "long",
        "f32" | "float" => "float",
        "f64" | "double" => "double",
        _ => "IntPtr",
    };
    format!("{cs_type} {cs_name}")
}

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
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    exclude_functions: &HashSet<String>,
    client_constructors: &HashMap<String, ClientConstructorConfig>,
    adapters: &[crate::core::config::AdapterConfig],
) -> String {
    use crate::backends::csharp::template_env::render;
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
    //
    // Walk through Optional/Vec wrappers when collecting returned Named types — the FFI
    // surfaces Option<T> and Vec<T> Named returns as `*mut T` and still emits matching
    // `_to_json` / `_free` exports, so the C# side needs the DllImport declarations.
    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        for param in &func.params {
            if let TypeRef::Named(name) = &param.ty {
                opaque_param_types.insert(name.clone());
            }
        }
        if let Some(name) = inner_named(&func.return_type) {
            if !enum_names.contains(name) {
                opaque_return_types.insert(name.to_string());
            }
        }
    }
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                // Streaming methods have no plain P/Invoke entry but the request payload still
                // needs `from_json` / `free`, and the item type needs `to_json` / `free`, so
                // the wrapper can serialize the request and deserialize each chunk.
                for param in &method.params {
                    if let TypeRef::Named(name) = &param.ty {
                        opaque_param_types.insert(name.clone());
                    }
                }
                if let Some(meta) = streaming_methods_meta.get(&method.name) {
                    // Streaming item types — whether enum or struct — flow through the
                    // FFI's `<type>_to_json` and `<type>_free` exports (the FFI backend
                    // emits them for both shapes), so the C# DllImport declarations are
                    // needed regardless of variant kind.  Without registering the type
                    // here, the C# streaming wrapper calls
                    // `NativeMethods.<ItemType>ToJson` against a non-existent extern and
                    // fails to compile (CS0117).
                    opaque_return_types.insert(meta.item_type.clone());
                }
                continue;
            }
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    opaque_param_types.insert(name.clone());
                }
            }
            if let Some(name) = inner_named(&method.return_type) {
                if !enum_names.contains(name) {
                    opaque_return_types.insert(name.to_string());
                }
            }
            // Instance methods (`fn with_*(self, ...)`) get a C# wrapper that
            // serialises `this` via `NativeMethods.{Type}FromJson(selfJson)` and
            // calls `{Type}ToJson` / `{Type}Free` on the returned handle. Those
            // calls need matching DllImport declarations, so register the owner
            // type as both a param and return type. Without this, types whose
            // only self-consuming methods take no Named params (e.g.
            // `ProblemDetails.with_detail(self, detail: String) -> Self`) never
            // appear in opaque_param_types / opaque_return_types and the
            // declarations are omitted.
            if method.receiver.is_some() {
                opaque_param_types.insert(typ.name.clone());
                if !enum_names.contains(&typ.name) {
                    opaque_return_types.insert(typ.name.clone());
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
    opaque_param_types.retain(|name| !bridge_type_aliases.contains(name));
    opaque_return_types.retain(|name| !bridge_type_aliases.contains(name));

    // Opaque handle classes own native pointers via SafeHandle, so every true
    // opaque type needs a matching free declaration even if no public wrapper
    // function currently accepts or returns that handle.
    let mut sorted_true_opaque_types: Vec<&String> = true_opaque_types.iter().collect();
    sorted_true_opaque_types.sort();
    for type_name in sorted_true_opaque_types {
        let snake = type_name.to_snake_case();
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
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

    // Emit _new P/Invoke declarations for opaque types that have a configured client_constructor.
    // These are paired with the _free entries emitted above.
    let mut sorted_ctor_types: Vec<&String> = client_constructors.keys().collect();
    sorted_ctor_types.sort();
    for type_name in sorted_ctor_types {
        let ctor = &client_constructors[type_name];
        let snake = type_name.to_snake_case();
        let new_entry = format!("{prefix}_{snake}_new");
        let new_cs = format!("{}New", csharp_type_name(type_name));
        if emitted.insert(new_entry.clone()) {
            // Build the P/Invoke param list with appropriate marshalling attributes.
            let params_str: String = ctor
                .params
                .iter()
                .map(|p| ffi_ty_to_pinvoke_param(&p.ty, &p.name))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &new_entry },
            ));
            out.push_str(&render(
                "client_constructor_pinvoke.jinja",
                minijinja::context! { new_cs, params_str },
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
            let from_json_cs = format!("{}FromJson", csharp_type_name(type_name));
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
        let free_cs = format!("{}Free", csharp_type_name(type_name));
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
            let to_json_cs = format!("{}ToJson", csharp_type_name(type_name));
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
        let free_cs = format!("{}Free", csharp_type_name(type_name));
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

    // Generate P/Invoke declarations for functions.
    // Skip trait-bridge lifecycle functions: the FFI layer exposes the trait-bridge
    // entry points separately in TraitBridges.cs. Emitting regular P/Invokes here
    // would duplicate or shadow those binding-safe APIs.
    for func in api.functions.iter().filter(|f| {
        !exclude_functions.contains(&f.name)
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, trait_bridges)
    }) {
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
    // Skip streaming adapter methods — the callback-based variant cannot be called from
    // P/Invoke; streaming is exposed instead via the iterator-handle entry points emitted below.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
            // A static method returning a borrowed reference to its own opaque type (e.g.
            // `Registry::global() -> &'static Registry`) has no FFI symbol — the FFI backend
            // cannot box a borrow into an owned `*mut T` handle. Emitting a `[DllImport]`
            // here would fail with EntryPointNotFoundException on first call.
            if method.returns_ref_to_owner(&typ.name) {
                continue;
            }
            let c_method_name = format!("{}_{}_{}", prefix, type_snake, method.name.to_lowercase());
            // Use a type-prefixed C# method name to avoid collisions when different types
            // share a method with the same name (e.g. BrowserConfig::default and CrawlConfig::default
            // would both produce "Default" without the prefix, but have different FFI entry points).
            let cs_method_name = format!("{}{}", csharp_type_name(&typ.name), to_csharp_name(&method.name));
            if emitted.insert(c_method_name.clone()) {
                out.push_str(&gen_pinvoke_for_method(&c_method_name, &cs_method_name, method));
            }
        }
    }

    // Emit P/Invoke declarations for streaming iterator-handle entry points:
    //   {prefix}_{owner_snake}_{name}_start(client*, req*) -> stream handle (IntPtr)
    //   {prefix}_{owner_snake}_{name}_next(handle*)        -> chunk pointer or IntPtr.Zero
    //   {prefix}_{owner_snake}_{name}_free(handle*)        -> void
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if !streaming_methods.contains(&method.name) {
                continue;
            }
            let cs_type = csharp_type_name(&typ.name);
            let cs_method = to_csharp_name(&method.name);

            let start_entry = format!("{}_{}_{}_start", prefix, type_snake, method.name.to_lowercase());
            let start_cs = format!("{cs_type}{cs_method}Start");
            if emitted.insert(start_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &start_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &start_cs,
                        params => "IntPtr client, IntPtr req",
                    },
                ));
                out.push('\n');
            }

            let next_entry = format!("{}_{}_{}_next", prefix, type_snake, method.name.to_lowercase());
            let next_cs = format!("{cs_type}{cs_method}Next");
            if emitted.insert(next_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &next_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &next_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }

            let free_entry = format!("{}_{}_{}_free", prefix, type_snake, method.name.to_lowercase());
            let free_cs = format!("{cs_type}{cs_method}Free");
            if emitted.insert(free_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &free_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "void",
                        cs_name => &free_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }
        }
    }

    // Emit P/Invoke declarations for streaming adapters that are configured
    // without a corresponding opaque owner method in the extracted API:
    //   {prefix}_{owner_snake}_{adapter_snake}_start(engine*, req*) -> stream handle (IntPtr)
    //   {prefix}_{owner_snake}_{adapter_snake}_next(handle*)        -> chunk pointer or IntPtr.Zero
    //   {prefix}_{owner_snake}_{adapter_snake}_free(handle*)        -> void
    for adapter in adapters {
        if matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
            let Some(owner_type) = adapter.owner_type.as_deref() else {
                continue;
            };
            let owner_snake = owner_type.to_snake_case();
            let owner_cs = csharp_type_name(owner_type);
            let adapter_snake = adapter.name.to_snake_case();
            let adapter_cs = to_csharp_name(&adapter.name);

            let start_entry = format!("{}_{}_{}_start", prefix, owner_snake, adapter_snake);
            let start_cs = format!("{owner_cs}{adapter_cs}Start");
            if emitted.insert(start_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &start_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &start_cs,
                        params => "IntPtr engine, IntPtr req",
                    },
                ));
                out.push('\n');
            }

            let next_entry = format!("{}_{}_{}_next", prefix, owner_snake, adapter_snake);
            let next_cs = format!("{owner_cs}{adapter_cs}Next");
            if emitted.insert(next_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &next_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &next_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }

            let free_entry = format!("{}_{}_{}_free", prefix, owner_snake, adapter_snake);
            let free_cs = format!("{owner_cs}{adapter_cs}Free");
            if emitted.insert(free_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &free_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "void",
                        cs_name => &free_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
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
            .find(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField);

        if let Some(bridge) = visitor_bridge {
            out.push_str(&crate::backends::csharp::gen_visitor::gen_native_methods_visitor(
                namespace,
                lib_name,
                prefix,
                &bridge.trait_name,
                bridge.options_type.as_deref().unwrap_or("Options"),
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
            // Collect visible type names (non-trait types that have C# bindings)
            let visible_type_names: std::collections::HashSet<&str> = api
                .types
                .iter()
                .filter(|t| !t.is_trait)
                .map(|t| t.name.as_str())
                .collect();
            out.push('\n');
            out.push_str(
                &crate::backends::csharp::trait_bridge::gen_native_methods_trait_bridges(
                    namespace,
                    prefix,
                    &bridges,
                    &visible_type_names,
                ),
            );
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
    use crate::backends::csharp::template_env::render;

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
            // Emit [MarshalAs(...)] attributes for types that need them.
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPUTF8Str)] ");
            } else if pinvoke_ty == "int" && matches!(param.ty, TypeRef::Primitive(PrimitiveType::Bool)) {
                // bool cross FFI as C int (0/1) — use U1 to marshal as single byte
                out.push_str("[MarshalAs(UnmanagedType.U1)] bool ");
                let param_name = param.name.to_lower_camel_case();
                out.push_str(&param_name);
                out.push_str(",\n");
                continue;
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
            // For byte-slice input parameters, emit the length parameter immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                let len_param_name = format!("{param_name}Len");
                out.push_str(&render(
                    "pinvoke_bytes_len_param.jinja",
                    minijinja::context! { len_param_name },
                ));
            }
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
    use crate::backends::csharp::template_env::render;

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
            // Emit [MarshalAs(...)] attributes for types that need them.
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPUTF8Str)] ");
            } else if pinvoke_ty == "int" && matches!(param.ty, TypeRef::Primitive(PrimitiveType::Bool)) {
                // bool cross FFI as C int (0/1) — use U1 to marshal as single byte
                out.push_str("[MarshalAs(UnmanagedType.U1)] bool ");
                let param_name = param.name.to_lower_camel_case();
                out.push_str(&param_name);
                out.push_str(",\n");
                continue;
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
            // For byte-slice input parameters, emit the length parameter immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                let len_param_name = format!("{param_name}Len");
                out.push_str(&render(
                    "pinvoke_bytes_len_param.jinja",
                    minijinja::context! { len_param_name },
                ));
            }
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
