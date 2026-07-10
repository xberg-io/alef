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
/// Public re-export of [`csharp_type_visible`] for the dedicated visitor-bridge generator.
pub(crate) fn csharp_type_visible_pub(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    csharp_type_visible(ty, visible_type_names)
}

fn csharp_type_visible(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) => {
            if visible_type_names.contains(name.as_str()) {
                csharp_type(ty).into_owned()
            } else {
                "string".to_string()
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !visible_type_names.contains(name.as_str()) => "string?".to_string(),
            _ => {
                let inner_type = csharp_type_visible(inner, visible_type_names);
                format!("{}?", inner_type)
            }
        },
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

    for (trait_name, bridge_cfg, trait_def) in bridges {
        if bridge_cfg.exclude_languages.iter().any(|lang| lang == "csharp") {
            continue;
        }

        if bridge_cfg.context_type.is_some() && bridge_cfg.result_type.is_some() {
            crate::backends::csharp::gen_visitor_bridge::gen_visitor_bridge(
                &mut out,
                trait_name,
                trait_def,
                visible_type_names,
            );
            out.push('\n');
            continue;
        }

        gen_single_trait_bridge(&mut out, trait_name, bridge_cfg, trait_def, prefix, visible_type_names);
        out.push('\n');
    }

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

    let num_methods = trait_def.methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    let num_vtable_fields = num_super_slots + num_methods + 2;
    let is_options_field = bridge_cfg.bind_via == BridgeBinding::OptionsField;

    let template_methods: Vec<_> = trait_def
        .methods
        .iter()
        .map(|method| {
            let mut parts: Vec<String> = Vec::new();
            for p in &method.params {
                let p_camel = p.name.to_lower_camel_case();
                parts.push(format!("{} {}", csharp_unmanaged_type(&p.ty), p_camel));
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
                        PrimitiveType::Usize => "ulong",
                        PrimitiveType::Isize => "long",
                        PrimitiveType::F32 => "float",
                        PrimitiveType::F64 => "double",
                        PrimitiveType::Bool => "int",
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

    let mut vtable_slots = String::with_capacity(1024);
    let mut offset = 0usize;
    let ptr_size = std::mem::size_of::<*const ()>();

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

    let mut callbacks = String::with_capacity(4096);

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

    for method in &trait_def.methods {
        let method_pascal = to_csharp_name(&method.name);

        let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_) | TypeRef::Unit);

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

        let params_decl = unmanaged_param_sig.clone();
        let params_decl_no_trailing = params_decl.clone();

        if method.return_type == TypeRef::Unit {
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
                    PrimitiveType::Usize => "ulong",
                    PrimitiveType::Isize => "long",
                    PrimitiveType::F32 => "float",
                    PrimitiveType::F64 => "double",
                    PrimitiveType::Bool => "int",
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
        callbacks.push_str(&render(
            "callback_registry_acquire.jinja",
            minijinja::context! { trait_pascal },
        ));

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

        callbacks.push_str("        try {\n");
        callbacks.push_str("            var bridge = _bridgeFromRegistry!;\n");

        let mut param_call_parts = Vec::new();
        for param in &method.params {
            let param_name = param.name.to_lower_camel_case();
            let managed_type = csharp_type_visible(&param.ty, visible_type_names);
            let is_non_api = matches!(&param.ty, TypeRef::Named(n) if !visible_type_names.contains(n.as_str()));

            match &param.ty {
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    param_call_parts.push(format!("({} != 0)", param_name));
                }
                TypeRef::Primitive(_) | TypeRef::Unit => {
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
                    callbacks.push_str(&render(
                        "callback_json_from_ptr.jinja",
                        minijinja::context! { param_name },
                    ));
                    if is_non_api {
                        param_call_parts.push(format!("json_{param_name}"));
                    } else {
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
            let method_call_syntax = if method.params.is_empty() {
                format!("bridge._impl.{}", method_pascal)
            } else {
                format!("bridge._impl.{}({})", method_pascal, param_call)
            };
            callbacks.push_str(&render(
                "callback_primitive_call.jinja",
                minijinja::context! { method_call_syntax },
            ));
            if matches!(&method.return_type, TypeRef::Primitive(PrimitiveType::Bool)) {
                callbacks.push_str("            return methodResult ? 1 : 0;\n");
            } else {
                callbacks.push_str("            return methodResult;\n");
            }
        } else {
            let is_property = method.params.is_empty();
            callbacks.push_str(&render(
                "callback_result_call.jinja",
                minijinja::context! { method_pascal, param_call, result_var => "methodResult", is_property },
            ));
            let is_named_visible =
                matches!(&method.return_type, TypeRef::Named(n) if visible_type_names.contains(n.as_str()));
            let serialize_expr = if is_named_visible {
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
            callbacks.push_str("        } catch (Exception ex) {\n");
        } else if !is_options_field {
            callbacks.push_str("        } catch (Exception ex) {\n");
        } else {
            callbacks.push_str("        } catch (Exception) {\n");
        }

        if !is_primitive_return {
            callbacks.push_str("            outResult = IntPtr.Zero;\n");
        }
        if !is_options_field && !is_primitive_return {
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
            callbacks.push_str(&format!(
                "            Console.Error.WriteLine($\"[{trait_pascal}Bridge] host '{method_pascal}' threw; returning default: {{ex}}\");\n"
            ));
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

    callbacks.push_str("    private void FreeStringCallback(IntPtr ptr) {\n");
    callbacks.push_str("        if (ptr != IntPtr.Zero) {\n");
    callbacks.push_str("            global::System.Runtime.InteropServices.Marshal.FreeCoTaskMem(ptr);\n");
    callbacks.push_str("        }\n");
    callbacks.push_str("    }\n");
    callbacks.push('\n');

    callbacks.push_str(&render(
        "free_user_data_callback.jinja",
        minijinja::context! { trait_pascal },
    ));
    callbacks.push('\n');

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

fn _to_json_string(_obj: &dyn std::any::Any) -> String {
    "null".to_string()
}

#[cfg(test)]
mod tests;
