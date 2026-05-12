//! C# trait bridge support via P/Invoke and managed delegates.
//!
//! For C# backends that use C FFI (FFI dependency), this module generates:
//! 1. P/Invoke declarations for trait bridge registration/unregistration functions
//! 2. Managed `interface I{TraitName}` with Plugin lifecycle + trait methods
//! 3. Bridge class `{TraitName}Bridge` implementing marshal helpers, delegate rooting, and vtable construction
//! 4. Static registration helpers: `RegisterOcrBackend(IOcrBackend impl)`, `UnregisterOcrBackend(string name)`

use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::config::{BridgeBinding, TraitBridgeConfig};
use alef_core::ir::{TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;

/// Maps a TypeRef to its C# representation, substituting non-visible Named types with string.
/// This prevents internal types like `InternalDocument` or `SyncExtractor` from appearing
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
    use crate::template_env::render;
    use minijinja::Value;

    if bridges.is_empty() {
        return String::new();
    }

    let bridge_data: Vec<_> = bridges
        .iter()
        .map(|(trait_name, config, _trait_def)| {
            let trait_snake = trait_name.to_snake_case();
            let register_fn = config
                .register_fn
                .as_deref()
                .map(|f| f.to_string())
                .unwrap_or_else(|| format!("{prefix}_register_{trait_snake}"));
            let has_unregister = config.unregister_fn.is_some();
            let unregister_fn = config.unregister_fn.as_deref().unwrap_or("").to_string();
            Value::from_serialize(serde_json::json!({
                "trait_name": trait_name,
                "register_fn": register_fn,
                "has_unregister": has_unregister,
                "unregister_fn": unregister_fn,
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
    use crate::template_env::render;
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

fn gen_single_trait_bridge(
    out: &mut String,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    _prefix: &str,
    visible_type_names: &HashSet<&str>,
) {
    use crate::template_env::render;
    use minijinja::Value;

    let trait_pascal = trait_name.to_pascal_case();
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
            let return_type = csharp_type_visible(&method.return_type, visible_type_names);
            let params = method
                .params
                .iter()
                .map(|p| format!("{} {}", csharp_type_visible(&p.ty, visible_type_names), to_csharp_name(&p.name)))
                .collect::<Vec<_>>()
                .join(", ");
            serde_json::json!({
                "name": method.name,
                "method_name": to_csharp_name(&method.name),
                "return_type": return_type,
                "params_sig": params,
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
    let num_vtable_fields = num_super_slots + num_methods + 1;
    let is_options_field = bridge_cfg.bind_via == BridgeBinding::OptionsField;

    // Build method data for template
    let template_methods: Vec<_> = trait_def
        .methods
        .iter()
        .map(|method| {
            let unmanaged_params = method
                .params
                .iter()
                .map(|p| format!("{} {}", csharp_unmanaged_type(&p.ty), to_csharp_name(&p.name)))
                .collect::<Vec<_>>()
                .join(", ");
            serde_json::json!({
                "pascal_name": to_csharp_name(&method.name),
                "params_empty": method.params.is_empty(),
                "unmanaged_params": unmanaged_params,
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
        callbacks.push_str("    private int NameFnCallback(IntPtr userData, out IntPtr outName) {\n");
        callbacks.push_str("        try {\n");
        callbacks.push_str("            var name = _impl.Name;\n");
        callbacks.push_str("            outName = Marshal.StringToCoTaskMemUTF8(name);\n");
        callbacks.push_str("            return 0;\n");
        callbacks.push_str("        } catch {\n");
        callbacks.push_str("            outName = IntPtr.Zero;\n");
        callbacks.push_str("            return 1;\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');

        callbacks.push_str("    private int VersionFnCallback(IntPtr userData, out IntPtr outVersion) {\n");
        callbacks.push_str("        try {\n");
        callbacks.push_str("            var version = _impl.Version;\n");
        callbacks.push_str("            outVersion = Marshal.StringToCoTaskMemUTF8(version);\n");
        callbacks.push_str("            return 0;\n");
        callbacks.push_str("        } catch {\n");
        callbacks.push_str("            outVersion = IntPtr.Zero;\n");
        callbacks.push_str("            return 1;\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');

        callbacks.push_str("    private int InitializeFnCallback(IntPtr userData, out IntPtr outError) {\n");
        callbacks.push_str("        try {\n");
        callbacks.push_str("            _impl.Initialize();\n");
        callbacks.push_str("            outError = IntPtr.Zero;\n");
        callbacks.push_str("            return 0;\n");
        callbacks.push_str("        } catch (Exception ex) {\n");
        callbacks.push_str("            outError = Marshal.StringToCoTaskMemUTF8(ex.Message);\n");
        callbacks.push_str("            return 1;\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');

        callbacks.push_str("    private int ShutdownFnCallback(IntPtr userData, out IntPtr outError) {\n");
        callbacks.push_str("        try {\n");
        callbacks.push_str("            _impl.Shutdown();\n");
        callbacks.push_str("            outError = IntPtr.Zero;\n");
        callbacks.push_str("            return 0;\n");
        callbacks.push_str("        } catch (Exception ex) {\n");
        callbacks.push_str("            outError = Marshal.StringToCoTaskMemUTF8(ex.Message);\n");
        callbacks.push_str("            return 1;\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');
    }

    // Trait method callbacks
    for method in &trait_def.methods {
        let method_pascal = to_csharp_name(&method.name);

        // Build parameter signature for unmanaged delegate (what we receive)
        let unmanaged_param_sig = method
            .params
            .iter()
            .map(|p| format!("{} {}", csharp_unmanaged_type(&p.ty), to_csharp_name(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");

        let params_decl = if unmanaged_param_sig.is_empty() {
            String::new()
        } else {
            format!("{}, ", unmanaged_param_sig)
        };

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
        callbacks.push_str("        try {\n");

        // Marshal parameters from IntPtr to managed types
        let mut param_call_parts = Vec::new();
        for param in &method.params {
            let param_name = to_csharp_name(&param.name);
            let managed_type = csharp_type_visible(&param.ty, visible_type_names);
            let is_non_api = matches!(&param.ty, TypeRef::Named(n) if !visible_type_names.contains(n.as_str()));

            match &param.ty {
                TypeRef::Primitive(_) | TypeRef::Unit => {
                    // Primitives don't need conversion
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
                    callbacks.push_str(&render(
                        "callback_bytes_param.jinja",
                        minijinja::context! { param_name },
                    ));
                    param_call_parts.push(format!("managed_{param_name}"));
                }
                _ => {
                    // For complex types (including non-API types), assume JSON deserialization
                    // Non-API types like InternalDocument are marshalled as strings (JSON)
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
            callbacks.push_str("            outResult = IntPtr.Zero;\n");
        } else {
            callbacks.push_str(&render(
                "callback_result_call.jinja",
                minijinja::context! { method_pascal, param_call },
            ));
            let is_non_api_return = matches!(&method.return_type, TypeRef::Named(n) if !visible_type_names.contains(n.as_str()));
            let serialize_expr = if is_non_api_return {
                // Non-API Named types: serialize as JSON string
                "ToJsonString(result)".to_string()
            } else if matches!(method.return_type, TypeRef::Named(_)) {
                // API Named types: use ToFfiJson()
                "result.ToFfiJson()".to_string()
            } else {
                "ToJsonString(result)".to_string()
            };
            callbacks.push_str(&render(
                "callback_result_serialize.jinja",
                minijinja::context! { serialize_expr },
            ));
        }

        if !is_options_field {
            callbacks.push_str("            outError = IntPtr.Zero;\n");
        }
        callbacks.push_str("            return 0;\n");
        if is_options_field {
            callbacks.push_str("        } catch (Exception) {\n");
        } else {
            callbacks.push_str("        } catch (Exception ex) {\n");
        }
        callbacks.push_str("            outResult = IntPtr.Zero;\n");
        if !is_options_field {
            callbacks.push_str("            outError = Marshal.StringToCoTaskMemUTF8(ex.Message);\n");
        }
        callbacks.push_str("            return 1;\n");
        callbacks.push_str("        }\n");
        callbacks.push_str("    }\n");
        callbacks.push('\n');
    }

    // free_user_data callback
    callbacks.push_str("    private void FreeUserDataCallback(IntPtr userData) {\n");
    callbacks.push_str("        if (userData != IntPtr.Zero) {\n");
    callbacks.push_str("            try {\n");
    callbacks.push_str("                var handle = GCHandle.FromIntPtr(userData);\n");
    callbacks.push_str("                handle.Free();\n");
    callbacks.push_str("            } catch (ObjectDisposedException) {\n");
    callbacks.push_str("                // Handle already freed; safe to ignore during finalization\n");
    callbacks.push_str("            }\n");
    callbacks.push_str("        }\n");
    callbacks.push_str("    }\n");
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
    out.push_str(&render(
        "trait_registry_class.jinja",
        Value::from_serialize(serde_json::json!({
            "trait_pascal": trait_pascal,
            "has_super_trait": has_super_trait,
            "has_unregister": has_unregister,
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
            rust_path: format!("kreuzberg::{}", name),
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
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    #[test]
    fn test_interface_contains_lifecycle_when_super_trait_set() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public interface IOcrBackend"));
        assert!(content.contains("string Name { get; }"));
        assert!(content.contains("string Version { get; }"));
        assert!(content.contains("void Initialize();"));
        assert!(content.contains("void Shutdown();"));
    }

    #[test]
    fn test_interface_omits_lifecycle_when_super_trait_empty() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", None);
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public interface IOcrBackend"));
        assert!(!content.contains("string Name { get; }"));
    }

    #[test]
    fn test_bridge_class_exists() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", None);
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public sealed class OcrBackendBridge : IDisposable"));
    }

    #[test]
    fn test_registry_no_super_trait_requires_explicit_name_param() {
        // Without super_trait, the interface has no Name property, so Register must
        // accept an explicit string name from the caller.
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", None);
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public static class OcrBackendRegistry"));
        assert!(content.contains("public static void Register(IOcrBackend impl, string name)"));
        // unregister_fn is None — Unregister must not be emitted
        assert!(!content.contains("public static void Unregister(string name)"));
        // No impl.Name reference when interface lacks it
        assert!(!content.contains("impl.Name"));
    }

    #[test]
    fn test_registry_with_super_trait_reads_name_from_impl() {
        // With super_trait, interface declares Name property; Register reads it from impl.
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public static class OcrBackendRegistry"));
        assert!(content.contains("public static void Register(IOcrBackend impl)"));
        assert!(!content.contains("Register(IOcrBackend impl, string name)"));
        assert!(content.contains("impl.Name"));
    }

    #[test]
    fn test_exclude_languages_skips_csharp() {
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.exclude_languages = vec!["csharp".to_string()];
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(!content.contains("interface IOcrBackend"));
        assert!(!content.contains("class OcrBackendBridge"));
    }

    #[test]
    fn test_native_methods_declarations_without_unregister() {
        // unregister_fn is None — only the register P/Invoke should be emitted.
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", None);
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("RegisterOcrBackend"));
        assert!(!content.contains("UnregisterOcrBackend"));
        assert!(content.contains("[DllImport"));
        assert!(content.contains("kreuzberg_register_ocr_backend"));
        assert!(!content.contains("kreuzberg_unregister_ocr_backend"));
    }

    #[test]
    fn test_native_methods_declarations_with_configured_unregister() {
        // When unregister_fn is set, both register and unregister P/Invokes are emitted
        // using the configured function names.
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.register_fn = Some("kreuzberg_register_ocr_backend".to_string());
        bridge_cfg.unregister_fn = Some("kreuzberg_unregister_ocr_backend".to_string());
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let content = gen_native_methods_trait_bridges("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("RegisterOcrBackend"));
        assert!(content.contains("UnregisterOcrBackend"));
        assert!(content.contains("[DllImport"));
        assert!(content.contains("kreuzberg_register_ocr_backend"));
        assert!(content.contains("kreuzberg_unregister_ocr_backend"));
    }

    #[test]
    fn test_registry_emits_unregister_when_configured() {
        // When unregister_fn is set, the registry class should contain an Unregister method.
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.unregister_fn = Some("kreuzberg_unregister_ocr_backend".to_string());
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public static class OcrBackendRegistry"));
        assert!(content.contains("public static void Unregister(string name)"));
        assert!(content.contains("NativeMethods.UnregisterOcrBackend(name, out var outError)"));
    }

    #[test]
    fn test_registry_omits_unregister_when_not_configured() {
        // When unregister_fn is None, the registry class must not emit an Unregister method.
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", None);
        let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
        let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();
        let (_filename, content) = gen_trait_bridges_file("Kreuzberg", "kreuzberg", &bridges, &visible_types);

        assert!(content.contains("public static class OcrBackendRegistry"));
        assert!(!content.contains("public static void Unregister(string name)"));
        assert!(!content.contains("NativeMethods.UnregisterOcrBackend"));
    }
}
