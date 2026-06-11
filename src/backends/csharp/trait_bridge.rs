//! C# trait bridge support via P/Invoke and managed delegates.
//!
//! For C# backends that use C FFI (FFI dependency), this module generates:
//! 1. P/Invoke declarations for trait bridge registration/unregistration functions
//! 2. Managed `interface I{TraitName}` with Plugin lifecycle + trait methods
//! 3. Bridge class `{TraitName}Bridge` implementing marshal helpers, delegate rooting, and vtable construction
//! 4. Static registration helpers: `RegisterTextBackend(ITextBackend impl)`, `UnregisterTextBackend(string name)`

use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;

/// Maps a TypeRef to its C# representation, substituting non-visible Named types with string.
/// This prevents internal types like `PrivatePayload` or `SyncExtractor` from appearing
/// in the generated trait interface signatures.
fn csharp_type_visible(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) => {
            if visible_type_names.contains(name.as_str()) {
                csharp_type(ty).into_owned()
            } else {
                "string".to_string()
            }
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::Named(name) if !visible_type_names.contains(name.as_str()) => {
                    // Optional<NonApiType> becomes string?
                    "string?".to_string()
                }
                _ => {
                    // Optional<ApiType> or other types: recurse and add ?
                    let inner_type = csharp_type_visible(inner, visible_type_names);
                    format!("{}?", inner_type)
                }
            }
        }
        TypeRef::Vec(inner) => {
            let inner_type = csharp_type_visible(inner, visible_type_names);
            format!("List<{}>", inner_type)
        }
        TypeRef::Map(k, v) => {
            let key_type = csharp_type_visible(k, visible_type_names);
            let val_type = csharp_type_visible(v, visible_type_names);
            format!("Dictionary<{}, {}>", key_type, val_type)
        }
        _ => csharp_type(ty).into_owned(),
    }
}

/// Maps a TypeRef to its unmanaged C# type for use in [UnmanagedFunctionPointer] delegates.
/// Managed types (arrays, classes, strings) become IntPtr; primitives remain as-is.
fn csharp_unmanaged_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "int".to_string(),
        TypeRef::Primitive(_) => csharp_type(ty).to_string(),
        TypeRef::Unit => csharp_type(ty).to_string(),
        // All managed types (String, Bytes, Vec, Optional containing managed, Named classes, etc.) become IntPtr
        _ => "IntPtr".to_string(),
    }
}

/// Generate P/Invoke trait bridge declarations for NativeMethods.cs.
///
/// For each trait bridge in the config, returns a C# P/Invoke declaration
/// for the register and unregister functions.
pub fn gen_native_methods_trait_bridges(
    _namespace: &str,
    prefix: &str,
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
    _visible_type_names: &HashSet<&str>,
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    if bridges.is_empty() {
        return String::new();
    }

    let bridge_data: Vec<_> = bridges
        .iter()
        .map(|(trait_name, config, _trait_def)| {
            let trait_snake = trait_name.to_snake_case();
            // The FFI layer always exports the register/unregister/clear functions as
            // `{prefix}_register_{trait_snake}` / `{prefix}_unregister_{trait_snake}` /
            // `{prefix}_clear_{trait_snake}` (see alef-backend-ffi trait_bridge::registration),
            // deliberately ignoring the alef.toml `register_fn` / `unregister_fn` / `clear_fn`
            // aliases (which only name the host-language wrappers, and may be plural or
            // unprefixed, e.g. `clear_text_backends`). The P/Invoke EntryPoint must match the
            // actual FFI symbol, not the alias. go/java derive these identically.
            let register_fn = format!("{prefix}_register_{trait_snake}");
            let has_unregister = config.unregister_fn.is_some();
            let unregister_fn = format!("{prefix}_unregister_{trait_snake}");
            let has_clear = config.clear_fn.is_some();
            let clear_fn = format!("{prefix}_clear_{trait_snake}");
            Value::from_serialize(serde_json::json!({
                "trait_name": trait_name,
                "register_fn": register_fn,
                "has_unregister": has_unregister,
                "unregister_fn": unregister_fn,
                "has_clear": has_clear,
                "clear_fn": clear_fn,
            }))
        })
        .collect();

    let ctx = Value::from_serialize(serde_json::json!({
        "bridges": bridge_data,
    }));

    render("native_methods_trait_bridges.jinja", ctx)
}

/// Generate the complete TraitBridges.cs file for all configured trait bridges.
///
/// For each bridge in the config:
/// - Generates a managed `interface I{TraitName}` with Plugin lifecycle methods (when super_trait set)
/// - Generates a `{TraitName}Bridge` class with delegate rooting, GCHandle management, and vtable construction
/// - Generates static registration helpers for `Register{TraitName}` / `Unregister{TraitName}`
///
/// Returns a tuple of (filename, content) ready for GeneratedFile emission.
pub fn gen_trait_bridges_file(
    namespace: &str,
    prefix: &str,
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
    visible_type_names: &HashSet<&str>,
) -> (String, String) {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let mut out = render(
        "trait_bridges_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
        })),
    );

    // Generate each trait bridge
    for (trait_name, bridge_cfg, trait_def) in bridges {
        // Skip if csharp is in exclude_languages
        if bridge_cfg.exclude_languages.iter().any(|lang| lang == "csharp") {
            continue;
        }

        gen_single_trait_bridge(&mut out, trait_name, bridge_cfg, trait_def, prefix, visible_type_names);
        out.push('\n');
    }

    // Generate extension helper class for JSON serialization across trait bridges
    out.push_str(&render(
        "ffi_json_extensions.jinja",
        Value::from_serialize(serde_json::json!({})),
    ));

    ("TraitBridges.cs".to_string(), out)
}

/// Generate the BridgeAdapters.cs file with sealed adapter classes for each trait.
///
/// For each trait bridge in the config that uses `FunctionParam` binding:
/// - Generates a sealed `_{TraitName}BridgeAdapter` class that implements `I{TraitName}`
/// - The adapter wraps a user-provided `I{TraitName}` implementation and delegates to it
/// - This follows the "Path A" pattern from Swift, enabling e2e tests to register user impls
///
/// Returns Option<(filename, content)> if there are any trait bridges to emit, None otherwise.
pub fn gen_bridge_adapters_file(
    namespace: &str,
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
    visible_type_names: &HashSet<&str>,
) -> Option<(String, String)> {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    // Filter to function_param bridges only (those where user provides implementation instance)
    let adapter_bridges: Vec<_> = bridges
        .iter()
        .filter(|(_, bridge_cfg, _)| {
            !bridge_cfg.exclude_languages.iter().any(|lang| lang == "csharp")
                && matches!(bridge_cfg.bind_via, BridgeBinding::FunctionParam)
        })
        .collect();

    if adapter_bridges.is_empty() {
        return None;
    }

    let mut trait_data = Vec::new();

    for (trait_name, bridge_cfg, trait_def) in &adapter_bridges {
        let trait_pascal = csharp_type_name(trait_name);
        let trait_snake = trait_name.to_snake_case();
        let has_super_trait = bridge_cfg.super_trait.is_some();

        // Build method signatures for delegation
        let methods: Vec<_> = trait_def
            .methods
            .iter()
            .map(|method| {
                let return_type = if method.is_async {
                    match &method.return_type {
                        TypeRef::Unit => "void".to_string(),
                        _ => csharp_type_visible(&method.return_type, visible_type_names),
                    }
                } else {
                    csharp_type_visible(&method.return_type, visible_type_names)
                };

                let params_sig = method
                    .params
                    .iter()
                    .map(|p| {
                        format!(
                            "{} {}",
                            csharp_type_visible(&p.ty, visible_type_names),
                            to_csharp_name(&p.name)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                let params_pass = method
                    .params
                    .iter()
                    .map(|p| to_csharp_name(&p.name))
                    .collect::<Vec<_>>()
                    .join(", ");

                // Zero-parameter methods with non-void return are properties
                let is_property = method.params.is_empty() && return_type != "void";

                serde_json::json!({
                    "pascal_name": to_csharp_name(&method.name),
                    "return_type": return_type,
                    "params_sig": params_sig,
                    "params_pass": params_pass,
                    "is_void_return": return_type == "void",
                    "is_property": is_property,
                })
            })
            .collect();

        trait_data.push(serde_json::json!({
            "pascal_name": trait_pascal,
            "snake_name": trait_snake,
            "has_super_trait": has_super_trait,
            "methods": methods,
        }));
    }

    let content = render(
        "trait_bridge_adapters.csharp.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "traits": trait_data,
        })),
    );

    Some(("BridgeAdapters.cs".to_string(), content))
}

fn gen_single_trait_bridge(
    out: &mut String,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    _prefix: &str,
    visible_type_names: &HashSet<&str>,
) {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let trait_pascal = csharp_type_name(trait_name);
    let _trait_snake = trait_name.to_snake_case();
    let has_super_trait = bridge_cfg.super_trait.is_some();
    let has_bytes_param = trait_def
        .methods
        .iter()
        .flat_map(|m| m.params.iter())
        .any(|p| matches!(&p.ty, TypeRef::Bytes));

    // --- Public Interface ---
    let methods: Vec<_> = trait_def
        .methods
        .iter()
        .map(|method| {
            // Unwrap async Task<T> to T. C# trait bridge interfaces expose synchronous methods
            // even though the Rust trait methods are async. The bridge implementation blocks
            // on the async Rust call.
            let return_type = if method.is_async {
                // async Task -> void, async Task<T> -> T
                match &method.return_type {
                    TypeRef::Unit => "void".to_string(),
                    _ => csharp_type_visible(&method.return_type, visible_type_names),
                }
            } else {
                csharp_type_visible(&method.return_type, visible_type_names)
            };
            let params = method
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        csharp_type_visible(&p.ty, visible_type_names),
                        to_csharp_name(&p.name)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            serde_json::json!({
                "name": method.name,
                "method_name": to_csharp_name(&method.name),
                "return_type": return_type,
                "params_sig": params,
                "params_empty": method.params.is_empty(),
            })
        })
        .collect();

    out.push_str(&render(
        "trait_interface.jinja",
        Value::from_serialize(serde_json::json!({
            "trait_pascal": trait_pascal,
            "has_super_trait": has_super_trait,
            "methods": methods,
        })),
    ));
    out.push('\n');

    // --- Bridge Class ---
    let num_methods = trait_def.methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    let num_vtable_fields = num_super_slots + num_methods + 2;
    let is_options_field = bridge_cfg.bind_via == BridgeBinding::OptionsField;

    // Build method data for template
    let template_methods: Vec<_> = trait_def
        .methods
        .iter()
        .map(|method| {
            let mut parts: Vec<String> = Vec::new();
            for p in &method.params {
                let p_camel = p.name.to_lower_camel_case();
                // Use camelCase for delegate parameters (idiomatic C# P/Invoke convention).
                parts.push(format!("{} {}", csharp_unmanaged_type(&p.ty), p_camel));
                // Bytes params carry a companion length so callers can read the full buffer
                // without NUL-truncation (mirrors the vtable.rs and call_body.rs pattern).
                if matches!(p.ty, TypeRef::Bytes) {
                    let len_name = format!("{p_camel}Len");
                    parts.push(format!("UIntPtr {len_name}"));
                }
            }
            let unmanaged_params = parts.join(", ");
            let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_) | TypeRef::Unit);
            let delegate_return_type = if is_primitive_return {
                match &method.return_type {
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::I8 => "sbyte",
                        PrimitiveType::I16 => "short",
                        PrimitiveType::I32 => "int",
                        PrimitiveType::I64 => "long",
                        PrimitiveType::U8 => "byte",
                        PrimitiveType::U16 => "ushort",
                        PrimitiveType::U32 => "uint",
                        PrimitiveType::U64 => "ulong",
                        PrimitiveType::Usize => "ulong", // usize maps to ulong
                        PrimitiveType::Isize => "long",  // isize maps to long
                        PrimitiveType::F32 => "float",
                        PrimitiveType::F64 => "double",
                        PrimitiveType::Bool => "int", // bool marshalled as int
                    },
                    TypeRef::Unit => "int",
                    _ => "int",
                }
            } else {
                "int"
            };
            serde_json::json!({
                "pascal_name": to_csharp_name(&method.name),
                "params_empty": method.params.is_empty(),
                "unmanaged_params": unmanaged_params,
                "is_primitive_return": is_primitive_return,
                "delegate_return_type": delegate_return_type,
            })
        })
        .collect();

    // Build vtable slots code
    let mut vtable_slots = String::with_capacity(1024);
    let mut offset = 0usize;
    let ptr_size = std::mem::size_of::<*const ()>();

    // Plugin lifecycle slots
    if has_super_trait {
        vtable_slots.push_str(&render(
            "vtable_slot_comment.jinja",
            minijinja::context! { slot_idx => offset, slot_name => "name_fn" },
        ));
        vtable_slots.push_str("        var nameFn = new NameFn(NameFnCallback);\n");
        vtable_slots.push_str(&render(
            "vtable_slot_assign.jinja",
            minijinja::context! { slot_idx => offset, fn_var => "name" },
        ));
        vtable_slots.push_str(&render(
            "vtable_write_intptr.jinja",
            minijinja::context! { byte_offset => offset * ptr_size, fn_var => "name" },
        ));
        vtable_slots.push('\n');
        offset += 1;

        vtable_slots.push_str(&render(
            "vtable_slot_comment.jinja",
            minijinja::context! { slot_idx => offset, slot_name => "version_fn" },
        ));
        vtable_slots.push_str("        var versionFn = new VersionFn(VersionFnCallback);\n");
        vtable_slots.push_str(&render(
            "vtable_slot_assign.jinja",
            minijinja::context! { slot_idx => offset, fn_var => "version" },
        ));
        vtable_slots.push_str(&render(
            "vtable_write_intptr.jinja",
            minijinja::context! { byte_offset => offset * ptr_size, fn_var => "version" },
        ));
        vtable_slots.push('\n');
        offset += 1;

        vtable_slots.push_str(&render(
            "vtable_slot_comment.jinja",
            minijinja::context! { slot_idx => offset, slot_name => "initialize_fn" },
        ));
        vtable_slots.push_str("        var initFn = new InitializeFn(InitializeFnCallback);\n");
        vtable_slots.push_str(&render(
            "vtable_slot_assign.jinja",
            minijinja::context! { slot_idx => offset, fn_var => "init" },
        ));
        vtable_slots.push_str(&render(
            "vtable_write_intptr.jinja",
            minijinja::context! { byte_offset => offset * ptr_size, fn_var => "init" },
        ));
        vtable_slots.push('\n');
        offset += 1;

        vtable_slots.push_str(&render(
            "vtable_slot_comment.jinja",
            minijinja::context! { slot_idx => offset, slot_name => "shutdown_fn" },
        ));
        vtable_slots.push_str("        var shutdownFn = new ShutdownFn(ShutdownFnCallback);\n");
        vtable_slots.push_str(&render(
            "vtable_slot_assign.jinja",
            minijinja::context! { slot_idx => offset, fn_var => "shutdown" },
        ));
        vtable_slots.push_str(&render(
            "vtable_write_intptr.jinja",
            minijinja::context! { byte_offset => offset * ptr_size, fn_var => "shutdown" },
        ));
        vtable_slots.push('\n');
        offset += 1;
    }

    // Trait method slots
    for method in &trait_def.methods {
        let method_pascal = to_csharp_name(&method.name);
        let method_camel = method.name.to_lower_camel_case();
        let slot_name = format!("{}_fn", method.name);
        vtable_slots.push_str(&render(
            "vtable_slot_comment.jinja",
            minijinja::context! { slot_idx => offset, slot_name },
        ));
        vtable_slots.push_str(&render(
            "vtable_method_fn_new.jinja",
            minijinja::context! { method_camel, method_pascal },
        ));
        vtable_slots.push_str(&render(
            "vtable_slot_assign.jinja",
            minijinja::context! { slot_idx => offset, fn_var => &method_camel },
        ));
        vtable_slots.push_str(&render(
            "vtable_write_intptr.jinja",
            minijinja::context! { byte_offset => offset * ptr_size, fn_var => &method_camel },
        ));
        vtable_slots.push('\n');
        offset += 1;
    }

    // free_string slot
    vtable_slots.push_str(&render(
        "vtable_slot_comment.jinja",
        minijinja::context! { slot_idx => offset, slot_name => "free_string" },
    ));
    vtable_slots.push_str("        var freeStringFn = new FreeStringFn(FreeStringCallback);\n");
    vtable_slots.push_str(&render(
        "vtable_slot_assign.jinja",
        minijinja::context! { slot_idx => offset, fn_var => "freeString" },
    ));
    vtable_slots.push_str(&render(
        "vtable_write_intptr.jinja",
        minijinja::context! { byte_offset => offset * ptr_size, fn_var => "freeString" },
    ));
    vtable_slots.push('\n');
    offset += 1;

    // free_user_data slot
    vtable_slots.push_str(&render(
        "vtable_slot_comment.jinja",
        minijinja::context! { slot_idx => offset, slot_name => "free_user_data" },
    ));
    vtable_slots.push_str("        var freeFn = new FreeUserDataFn(FreeUserDataCallback);\n");
    vtable_slots.push_str(&render(
        "vtable_slot_assign.jinja",
        minijinja::context! { slot_idx => offset, fn_var => "free" },
    ));
    vtable_slots.push_str(&render(
        "vtable_write_intptr.jinja",
        minijinja::context! { byte_offset => offset * ptr_size, fn_var => "free" },
    ));

    // Generate callbacks
    let mut callbacks = String::with_capacity(4096);

    // Plugin lifecycle callbacks
    if has_super_trait {
        callbacks.push_str(&render(
            "plugin_string_callback.jinja",
            minijinja::context! {
                callback_name => "NameFnCallback",
                out_name => "outName",
                local_name => "_name",
                trait_pascal,
                property_name => "Name",
            },
        ));
        callbacks.push('\n');

        callbacks.push_str(&render(
            "plugin_string_callback.jinja",
            minijinja::context! {
                callback_name => "VersionFnCallback",
                out_name => "outVersion",
                local_name => "_version",
                trait_pascal,
                property_name => "Version",
            },
        ));
        callbacks.push('\n');

        callbacks.push_str(&render(
            "plugin_void_callback.jinja",
            minijinja::context! {
                callback_name => "InitializeFnCallback",
                trait_pascal,
                method_name => "Initialize",
            },
        ));
        callbacks.push('\n');

        callbacks.push_str(&render(
            "plugin_void_callback.jinja",
            minijinja::context! {
                callback_name => "ShutdownFnCallback",
                trait_pascal,
                method_name => "Shutdown",
            },
        ));
        callbacks.push('\n');
    }

    // Trait method callbacks
    for method in &trait_def.methods {
        let method_pascal = to_csharp_name(&method.name);

        // Check if return type is a primitive (non-complex type)
        let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_) | TypeRef::Unit);

        // Build parameter signature for unmanaged delegate (what we receive).
        // Bytes params carry a companion UIntPtr {name}Len so the callback can
        // copy the full buffer without NUL-truncation.
        // Use camelCase for all delegate parameters (idiomatic C# P/Invoke convention).
        let mut sig_parts: Vec<String> = Vec::new();
        for p in &method.params {
            let p_camel = p.name.to_lower_camel_case();
            sig_parts.push(format!("{} {}", csharp_unmanaged_type(&p.ty), p_camel));
            if matches!(p.ty, TypeRef::Bytes) {
                let len_name = format!("{p_camel}Len");
                sig_parts.push(format!("UIntPtr {len_name}"));
            }
        }
        let unmanaged_param_sig = sig_parts.join(", ");

        // params_decl is the comma-separated parameter list WITHOUT trailing
        // comma. Each callback header template that consumes it is responsible
        // for emitting the `, ` separator between userData and params_decl (and
        // between params_decl and the out params). This avoids double-commas
        // when params_decl is non-empty.
        let params_decl = unmanaged_param_sig.clone();
        let params_decl_no_trailing = params_decl.clone();

        if method.return_type == TypeRef::Unit {
            // void return: no out params
            let params_with_userdata = if params_decl_no_trailing.is_empty() {
                "IntPtr userData".to_string()
            } else {
                format!("IntPtr userData, {}", params_decl_no_trailing)
            };
            callbacks.push_str(&render(
                "callback_header_void.jinja",
                minijinja::context! { method_pascal, params_with_userdata },
            ));
        } else if is_primitive_return {
            // Primitive return: return directly (no out params)
            let return_c_type = match &method.return_type {
                TypeRef::Primitive(p) => match p {
                    PrimitiveType::I8 => "sbyte",
                    PrimitiveType::I16 => "short",
                    PrimitiveType::I32 => "int",
                    PrimitiveType::I64 => "long",
                    PrimitiveType::U8 => "byte",
                    PrimitiveType::U16 => "ushort",
                    PrimitiveType::U32 => "uint",
                    PrimitiveType::U64 => "ulong",
                    PrimitiveType::Usize => "ulong", // usize maps to ulong
                    PrimitiveType::Isize => "long",  // isize maps to long
                    PrimitiveType::F32 => "float",
                    PrimitiveType::F64 => "double",
                    PrimitiveType::Bool => "int", // bool marshalled as int for C ABI
                },
                _ => "int",
            };
            let params_with_userdata = if params_decl_no_trailing.is_empty() {
                "IntPtr userData".to_string()
            } else {
                format!("IntPtr userData, {}", params_decl_no_trailing)
            };
            callbacks.push_str(&render(
                "callback_header_primitive.jinja",
                minijinja::context! { return_c_type, method_pascal, params_with_userdata },
            ));
        } else {
            // Complex return: use out params
            if is_options_field {
                callbacks.push_str(&render(
                    "callback_header_options.jinja",
                    minijinja::context! { method_pascal, params_decl },
                ));
            } else {
                callbacks.push_str(&render(
                    "callback_header_full.jinja",
                    minijinja::context! { method_pascal, params_decl },
                ));
            }
        }
        // Recover the bridge instance from the registry using userData as key
        callbacks.push_str(&render(
            "callback_registry_acquire.jinja",
            minijinja::context! { trait_pascal },
        ));

        // If bridge not found, return error gracefully
        callbacks.push_str("        if (_bridgeFromRegistry == null) {\n");
        if is_primitive_return {
            callbacks.push_str("            return 0;\n");
        } else if is_options_field {
            callbacks.push_str("            outResult = IntPtr.Zero;\n");
            callbacks.push_str("            return 1;\n");
        } else {
            callbacks.push_str("            outResult = IntPtr.Zero;\n");
            callbacks.push_str("            outError = global::System.Runtime.InteropServices.Marshal.StringToCoTaskMemUTF8($\"Bridge not found for userData (likely unregistered): {userData}\");\n");
            callbacks.push_str("            return 1;\n");
        }
        callbacks.push_str("        }\n");

        // Outer try-finally to ensure refcount is decremented (only for successful bridge acquisition)
        callbacks.push_str("        try {\n");
        callbacks.push_str("            var bridge = _bridgeFromRegistry!;\n");

        // Marshal parameters from IntPtr to managed types
        let mut param_call_parts = Vec::new();
        for param in &method.params {
            // Use camelCase so the variable name matches the delegate signature.
            let param_name = param.name.to_lower_camel_case();
            let managed_type = csharp_type_visible(&param.ty, visible_type_names);
            let is_non_api = matches!(&param.ty, TypeRef::Named(n) if !visible_type_names.contains(n.as_str()));

            match &param.ty {
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    // Bool comes from unmanaged side as int; convert to bool
                    param_call_parts.push(format!("({} != 0)", param_name));
                }
                TypeRef::Primitive(_) | TypeRef::Unit => {
                    // Other primitives don't need conversion
                    param_call_parts.push(param_name);
                }
                TypeRef::String | TypeRef::Char => {
                    callbacks.push_str(&render(
                        "callback_string_param.jinja",
                        minijinja::context! { param_name },
                    ));
                    param_call_parts.push(format!("managed_{param_name}"));
                }
                TypeRef::Bytes => {
                    let len_name = format!("{param_name}Len");
                    callbacks.push_str(&render(
                        "callback_bytes_param.jinja",
                        minijinja::context! { param_name, len_name },
                    ));
                    param_call_parts.push(format!("managed_{param_name}"));
                }
                _ => {
                    // For complex types (including non-API types), assume JSON deserialization
                    // Non-API types like PrivatePayload are marshalled as strings (JSON)
                    callbacks.push_str(&render(
                        "callback_json_from_ptr.jinja",
                        minijinja::context! { param_name },
                    ));
                    if is_non_api {
                        // Non-API types: keep as string (JSON), don't deserialize
                        // callback_json_from_ptr declares json_{param_name}
                        param_call_parts.push(format!("json_{param_name}"));
                    } else {
                        // API types: deserialize to the actual type
                        callbacks.push_str(&render(
                            "callback_json_deserialize.jinja",
                            minijinja::context! { param_name, managed_type },
                        ));
                        param_call_parts.push(format!("managed_{param_name}"));
                    }
                }
            }
        }

        let param_call = param_call_parts.join(", ");

        if method.return_type == TypeRef::Unit {
            callbacks.push_str(&render(
                "callback_void_call.jinja",
                minijinja::context! { method_pascal, param_call },
            ));
            callbacks.push_str("            return 0;\n");
        } else if is_primitive_return {
            // Primitive return: call method and return directly
            // Use methodResult to avoid variable shadowing with parameters
            // Zero-parameter non-void methods are emitted as properties in C#
            let method_call_syntax = if method.params.is_empty() {
                format!("bridge._impl.{}", method_pascal)
            } else {
                format!("bridge._impl.{}({})", method_pascal, param_call)
            };
            callbacks.push_str(&render(
                "callback_primitive_call.jinja",
                minijinja::context! { method_call_syntax },
            ));
            // Convert return value based on method return type
            if matches!(&method.return_type, TypeRef::Primitive(PrimitiveType::Bool)) {
                // bool → int (0 or 1)
                callbacks.push_str("            return methodResult ? 1 : 0;\n");
            } else {
                // Other primitives: cast to the delegate return type (no cast needed if already matching type)
                // Just return as-is since the delegate return type already matches
                callbacks.push_str("            return methodResult;\n");
            }
        } else {
            // Complex return: use out params
            // Zero-parameter non-void methods are emitted as properties in C#
            let is_property = method.params.is_empty();
            callbacks.push_str(&render(
                "callback_result_call.jinja",
                minijinja::context! { method_pascal, param_call, result_var => "methodResult", is_property },
            ));
            // Check if return type is a Named type (struct or enum) that's visible
            let is_named_visible =
                matches!(&method.return_type, TypeRef::Named(n) if visible_type_names.contains(n.as_str()));
            // All Named types (both enums and struct types) that are visible have ToFfiJson() extension methods
            let serialize_expr = if is_named_visible {
                // Named types (enums or structs): use ToFfiJson()
                "methodResult.ToFfiJson()".to_string()
            } else {
                "ToJsonString(methodResult)".to_string()
            };
            callbacks.push_str(&render(
                "callback_result_serialize.jinja",
                minijinja::context! { serialize_expr },
            ));
            if !is_options_field {
                callbacks.push_str("            outError = IntPtr.Zero;\n");
            }
            callbacks.push_str("            return 0;\n");
        }

        if is_primitive_return {
            callbacks.push_str("        } catch (Exception) {\n");
        } else if !is_options_field {
            // Only bind ex for non-primitive, non-options-field returns where we log it
            callbacks.push_str("        } catch (Exception ex) {\n");
        } else {
            // Options-field binding doesn't use exception details
            callbacks.push_str("        } catch (Exception) {\n");
        }

        if !is_primitive_return {
            callbacks.push_str("            outResult = IntPtr.Zero;\n");
        }
        if !is_options_field && !is_primitive_return {
            // Defensive error handling: if exception occurs, try to log the message,
            // but if that fails for ANY reason (including p/invoke stack corruption),
            // just set outError to IntPtr.Zero and let Rust see the 1 return code.
            // The issue is that StringToCoTaskMemUTF8 itself can crash if the stack is corrupted.
            callbacks.push_str("            outError = IntPtr.Zero;\n");
            callbacks.push_str(
                "            // Attempt to marshal exception message, but on ANY failure just leave outError null\n",
            );
            callbacks.push_str("            try {\n");
            callbacks.push_str("                string _errMsg = null!;\n");
            callbacks.push_str("                try {\n");
            callbacks.push_str(
                "                    _errMsg = ex?.Message ?? ex?.GetType()?.Name ?? \"Unknown exception\";\n",
            );
            callbacks.push_str("                } catch {\n");
            callbacks.push_str("                    _errMsg = \"Callback failed\";\n");
            callbacks.push_str("                }\n");
            callbacks.push_str("                if (!string.IsNullOrEmpty(_errMsg)) {\n");
            callbacks.push_str("                    outError = global::System.Runtime.InteropServices.Marshal.StringToCoTaskMemUTF8(_errMsg);\n");
            callbacks.push_str("                }\n");
            callbacks.push_str("            } catch {\n");
            callbacks
                .push_str("                // Marshalling failed; outError stays null — Rust will see return code 1\n");
            callbacks.push_str("            }\n");
        }
        if !is_primitive_return {
            callbacks.push_str("            return 1;\n");
        } else {
            callbacks.push_str("            return 0;\n");
        }
        callbacks.push_str("        } finally {\n");
        callbacks.push_str("            if (_bridgeFromRegistry != null) {\n");
        callbacks.push_str("                try { _bridgeFromRegistry.DecrementCallbackRef(); } catch { /* Bridge already removed from registry */ }\n");
        callbacks.push_str("            }\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');
    }

    // free_string callback
    callbacks.push_str("    private void FreeStringCallback(IntPtr ptr) {\n");
    callbacks.push_str("        if (ptr != IntPtr.Zero) {\n");
    callbacks.push_str("            global::System.Runtime.InteropServices.Marshal.FreeCoTaskMem(ptr);\n");
    callbacks.push_str("        }\n");
    callbacks.push_str("    }\n");
    callbacks.push('\n');

    // free_user_data callback
    callbacks.push_str(&render(
        "free_user_data_callback.jinja",
        minijinja::context! { trait_pascal },
    ));
    callbacks.push('\n');

    // Render the bridge class with callbacks
    out.push_str(&render(
        "trait_bridge_class.jinja",
        Value::from_serialize(serde_json::json!({
            "trait_pascal": trait_pascal,
            "has_super_trait": has_super_trait,
            "num_vtable_fields": num_vtable_fields,
            "methods": template_methods,
            "is_options_field": is_options_field,
            "vtable_slots": vtable_slots,
            "has_bytes_param": has_bytes_param,
            "callbacks": callbacks,
        })),
    ));
    out.push('\n');

    // --- Registry Class ---
    let has_unregister = bridge_cfg.unregister_fn.is_some();
    let has_clear = bridge_cfg.clear_fn.is_some();
    let clear_fn = bridge_cfg.clear_fn.as_deref().unwrap_or("").to_string();
    out.push_str(&render(
        "trait_registry_class.jinja",
        Value::from_serialize(serde_json::json!({
            "trait_pascal": trait_pascal,
            "has_super_trait": has_super_trait,
            "has_unregister": has_unregister,
            "has_clear": has_clear,
            "clear_fn": clear_fn,
        })),
    ));
}

// Placeholder for JSON serialization — in production, would use System.Text.Json
fn _to_json_string(_obj: &dyn std::any::Any) -> String {
    "null".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trait_def(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample_crate::{}", name),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            param_name: None,
            type_alias: None,
            exclude_languages: vec![],
            super_trait: super_trait.map(|s| s.to_string()),
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
            register_extra_args: None,
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    #[test]
    fn test_interface_contains_lifecycle_when_super_trait_set() {
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public interface ITextBackend"));
        assert!(content.contains("string Name { get; }"));
        assert!(content.contains("string Version { get; }"));
        assert!(content.contains("void Initialize();"));
        assert!(content.contains("void Shutdown();"));
    }

    #[test]
    fn test_interface_omits_lifecycle_when_super_trait_empty() {
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public interface ITextBackend"));
        assert!(!content.contains("string Name { get; }"));
    }

    #[test]
    fn test_bridge_class_exists() {
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
        assert!(content.contains("private delegate void FreeStringFn(IntPtr ptr);"));
        assert!(content.contains("FreeStringCallback"));
        assert!(content.contains("Marshal.FreeCoTaskMem(ptr);"));
    }

    #[test]
    fn test_bool_callback_param_uses_int_boundary_type() {
        let mut trait_def = make_trait_def("Checker");
        trait_def.methods = vec![crate::core::ir::MethodDef {
            name: "check".to_string(),
            params: vec![crate::core::ir::ParamDef {
                name: "enabled".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];
        let bridge_cfg = make_bridge_cfg("Checker", None);
        let bridges = vec![("Checker".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["Checker"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("private delegate int CheckFn(IntPtr userData, int enabled);"));
    }

    #[test]
    fn test_registry_no_super_trait_requires_explicit_name_param() {
        // Without super_trait, the interface has no Name property, so Register must
        // accept an explicit string name from the caller.
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public static class TextBackendRegistry"));
        assert!(content.contains("public static IntPtr Register(ITextBackend impl, string name)"));
        // unregister_fn is None — Unregister must not be emitted
        assert!(!content.contains("public static void Unregister(string name)"));
        // No impl.Name reference when interface lacks it
        assert!(!content.contains("impl.Name"));
    }

    #[test]
    fn test_registry_with_super_trait_reads_name_from_impl() {
        // With super_trait, interface declares Name property; Register reads it from impl.
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public static class TextBackendRegistry"));
        assert!(content.contains("public static IntPtr Register(ITextBackend impl)"));
        assert!(!content.contains("Register(ITextBackend impl, string name)"));
        assert!(content.contains("impl.Name"));
    }

    #[test]
    fn test_exclude_languages_skips_csharp() {
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
        bridge_cfg.exclude_languages = vec!["csharp".to_string()];
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(!content.contains("interface ITextBackend"));
        assert!(!content.contains("class TextBackendBridge"));
    }

    #[test]
    fn test_native_methods_declarations_without_unregister() {
        // unregister_fn is None — only the register P/Invoke should be emitted.
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("RegisterTextBackend"));
        assert!(!content.contains("UnregisterTextBackend"));
        assert!(content.contains("[DllImport"));
        assert!(content.contains("sample_crate_register_text_backend"));
        assert!(!content.contains("sample_crate_unregister_text_backend"));
    }

    #[test]
    fn test_native_methods_declarations_with_configured_unregister() {
        // When unregister_fn is set, both register and unregister P/Invokes are emitted.
        // The EntryPoints are derived from `{prefix}_{register,unregister}_{trait_snake}`,
        // not from the alias values.
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
        bridge_cfg.register_fn = Some("sample_crate_register_text_backend".to_string());
        bridge_cfg.unregister_fn = Some("sample_crate_unregister_text_backend".to_string());
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("RegisterTextBackend"));
        assert!(content.contains("UnregisterTextBackend"));
        assert!(content.contains("[DllImport"));
        assert!(content.contains("sample_crate_register_text_backend"));
        assert!(content.contains("sample_crate_unregister_text_backend"));
    }

    #[test]
    fn test_native_methods_register_unregister_use_derived_ffi_symbol_not_alias() {
        // The alef.toml `register_fn` / `unregister_fn` aliases name the host-language
        // wrappers and are typically unprefixed (e.g. `register_renderer`). The P/Invoke
        // EntryPoint must match the actual FFI symbol `{prefix}_register_{trait_snake}`,
        // never the bare alias.
        let trait_def = make_trait_def("Renderer");
        let mut bridge_cfg = make_bridge_cfg("Renderer", None);
        bridge_cfg.register_fn = Some("register_renderer".to_string());
        bridge_cfg.unregister_fn = Some("unregister_renderer".to_string());
        bridge_cfg.clear_fn = Some("clear_renderers".to_string());
        let bridges = vec![("Renderer".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["Renderer"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("EntryPoint = \"sample_crate_register_renderer\""));
        assert!(content.contains("EntryPoint = \"sample_crate_unregister_renderer\""));
        assert!(content.contains("EntryPoint = \"sample_crate_clear_renderer\""));
        assert!(!content.contains("EntryPoint = \"register_renderer\""));
        assert!(!content.contains("EntryPoint = \"unregister_renderer\""));
        assert!(!content.contains("EntryPoint = \"clear_renderers\""));
        assert!(!content.contains("sample_crate_clear_renderers"));
    }

    #[test]
    fn test_native_methods_clear_uses_derived_ffi_symbol_not_alias() {
        // The FFI layer exports the clear function as `{prefix}_clear_{trait_snake}`,
        // ignoring the alef.toml `clear_fn` alias (which may be plural, e.g.
        // `clear_text_backends`). The P/Invoke EntryPoint must match the actual FFI
        // symbol `sample_core_clear_text_backend`, not the alias.
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
        bridge_cfg.clear_fn = Some("clear_text_backends".to_string());
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("EntryPoint = \"sample_crate_clear_text_backend\""));
        assert!(!content.contains("sample_crate_clear_text_backends"));
        assert!(!content.contains("EntryPoint = \"clear_text_backends\""));
        assert!(content.contains("ClearTextBackend("));
    }

    #[test]
    fn test_native_methods_omits_clear_when_not_configured() {
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(!content.contains("sample_crate_clear_text_backend"));
        assert!(!content.contains("ClearTextBackend("));
    }

    #[test]
    fn test_registry_emits_clear_when_configured() {
        // When clear_fn is set, the registry class should contain a Clear method.
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
        bridge_cfg.clear_fn = Some("clear_text_backends".to_string());
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public static void Clear()"));
        assert!(content.contains("NativeMethods.ClearTextBackend(out var outError)"));
    }

    #[test]
    fn test_registry_omits_clear_when_not_configured() {
        // When clear_fn is None, the registry class must not emit a Clear method.
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(!content.contains("NativeMethods.ClearTextBackend("));
    }

    #[test]
    fn test_registry_emits_unregister_when_configured() {
        // When unregister_fn is set, the registry class should contain an Unregister method.
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
        bridge_cfg.unregister_fn = Some("sample_crate_unregister_text_backend".to_string());
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public static class TextBackendRegistry"));
        assert!(content.contains("public static void Unregister(string name)"));
        assert!(content.contains("NativeMethods.UnregisterTextBackend(name, out var outError)"));
    }

    #[test]
    fn test_registry_omits_unregister_when_not_configured() {
        // When unregister_fn is None, the registry class must not emit an Unregister method.
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public static class TextBackendRegistry"));
        assert!(!content.contains("public static void Unregister(string name)"));
        assert!(!content.contains("NativeMethods.UnregisterTextBackend"));
    }

    /// Regression (#114): the `[UnmanagedFunctionPointer]` delegate type for a Bytes parameter
    /// must include `UIntPtr {name}Len` immediately after the `IntPtr {name}` field.
    /// The callback marshalling must use `Marshal.Copy(ptr, dst, 0, len)` rather than reading
    /// bytes as a NUL-terminated JSON string, which silently truncates payloads containing 0x00.
    #[test]
    fn test_bridge_delegate_bytes_param_includes_len_companion() {
        let mut trait_def = make_trait_def("Processor");
        trait_def.methods.push(crate::core::ir::MethodDef {
            name: "ingest".to_string(),
            params: vec![crate::core::ir::ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        });
        let bridge_cfg = make_bridge_cfg("Processor", None);
        let bridges = vec![("Processor".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["Processor"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        // Delegate type must carry the length companion parameter.
        assert!(
            content.contains("UIntPtr payloadLen"),
            "delegate signature must include `UIntPtr payloadLen` for Bytes param;\nactual:\n{content}"
        );
        // Callback body must use Marshal.Copy for bounded binary copy, not string deserialization.
        assert!(
            content.contains("Marshal.Copy(payload"),
            "callback must use Marshal.Copy for Bytes param;\nactual:\n{content}"
        );
        // Must not revert to the old JSON/base64 string path.
        assert!(
            !content.contains("MarshalBytesFromIntPtr"),
            "callback must not use MarshalBytesFromIntPtr;\nactual:\n{content}"
        );
    }

    #[test]
    fn test_bridge_class_has_register_static_method_with_super_trait() {
        // Bridge class should have a static Register method that takes the impl and optionally name
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
        assert!(content.contains("public static IntPtr Register(ITextBackend impl)"));
        // Verify it's reading impl.Name
        assert!(content.contains("var name = impl.Name;"));
    }

    #[test]
    fn test_bridge_class_has_register_static_method_without_super_trait() {
        // Bridge class should have a static Register method that takes impl and explicit name param
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend", None);
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
        assert!(content.contains("public static IntPtr Register(ITextBackend impl, string name)"));
    }

    /// Regression: enum return types are visible in the interface, so the interface
    /// declares the actual enum type. The callback receives the enum and must serialize it
    /// using .ToFfiJson() extension method.
    #[test]
    fn test_trait_method_enum_return_uses_toffijson_serialization() {
        let mut trait_def = make_trait_def("PostProcessor");
        trait_def.methods.push(crate::core::ir::MethodDef {
            name: "processing_stage".to_string(),
            params: vec![],
            return_type: TypeRef::Named("ProcessingStage".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        });
        let bridge_cfg = make_bridge_cfg("PostProcessor", Some("Plugin"));
        let bridges = vec![("PostProcessor".to_string(), &bridge_cfg, &trait_def)];
        // ProcessingStage IS in visible_types (enums are now visible)
        let visible_types: HashSet<&str> = vec!["PostProcessor", "ProcessingStage"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

        // The interface property returns ProcessingStage (not string), and the callback
        // receives the actual enum value. It must serialize using .ToFfiJson().
        assert!(content.contains("ProcessingStage ProcessingStage { get; }"));
        assert!(content.contains("methodResult.ToFfiJson()"));
        assert!(!content.contains("ToJsonString(methodResult)"));
    }
}
