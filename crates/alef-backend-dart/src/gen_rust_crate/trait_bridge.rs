use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

use super::conversions::frb_rust_type;
use super::trait_types::{
    trait_impl_param_conversion, trait_impl_param_type, trait_impl_return_conversion, trait_impl_return_type,
};

/// Emit a FRB trait bridge for one configured trait.
///
/// Produces the following items in the lib.rs:
///
/// 1. `#[frb(opaque)] pub struct {Trait}DartImpl` — holds one `Box<dyn Fn(...)
///    -> DartFnFuture<ret> + Send + Sync>` closure per own method. If the trait
///    has a `Plugin` super-trait, also holds `plugin_name: String` and
///    `plugin_version: String` fields.
/// 2. `impl SuperTrait for {Trait}DartImpl` — for each super-trait in `super_traits`,
///    emits a stub impl. The well-known `Plugin` super-trait is handled directly;
///    other super-traits emit a `// TODO` comment stub.
/// 3. `impl {Trait} for {Trait}DartImpl` — delegates each method to its closure.
/// 4. `pub fn create_{trait_snake}_dart_impl(...)` — factory function.
///
/// Dart-side wiring (`class MyOcrBackend implements OcrBackend { ... }`) is
/// post-FRB-codegen-runtime work and is NOT generated here.
pub(crate) fn emit_trait_bridge(
    out: &mut String,
    trait_def: &TypeDef,
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) {
    let trait_name = &trait_def.name;
    let trait_snake = trait_name.to_snake_case();
    let struct_name = format!("{trait_name}DartImpl");
    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate_name}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    // Filter to only own methods (no super-trait inherited ones).
    let own_methods: Vec<&MethodDef> = trait_def.methods.iter().filter(|m| m.trait_source.is_none()).collect();

    // Check if Plugin is a direct super-trait.
    let has_plugin_super = trait_def
        .super_traits
        .iter()
        .any(|s| s == "Plugin" || s.ends_with("::Plugin"));

    // --- 1. Opaque struct with one closure field per method ---
    out.push_str("/// FRB opaque handle holding Dart callbacks for each trait method.\n");
    out.push_str("/// Dart-side: register callbacks via `create_{snake}_dart_impl(...)` factory.\n");
    out.push_str("#[frb(opaque)]\n");
    out.push_str(&crate::template_env::render(
        "rust_mirror_struct_open.jinja",
        minijinja::context! {
            name => struct_name.as_str(),
        },
    ));
    // Plugin fields for name/version (required by Plugin super-trait).
    if has_plugin_super {
        out.push_str("    /// Plugin name used by the Plugin super-trait impl.\n");
        out.push_str("    plugin_name: String,\n");
        out.push_str("    /// Plugin version used by the Plugin super-trait impl.\n");
        out.push_str("    plugin_version: String,\n");
    }
    for method in &own_methods {
        let field_name = &method.name;
        let callback_ty = dart_fn_future_callback_type(method, source_crate_name, type_paths);
        out.push_str(&crate::template_env::render(
            "rust_trait_struct_field.jinja",
            minijinja::context! {
                field_name => field_name.as_str(),
                callback_ty => callback_ty,
            },
        ));
    }
    out.push_str(&crate::template_env::render(
        "rust_mirror_struct_close.jinja",
        minijinja::context! {},
    ));
    out.push('\n');

    // --- 2. impl Plugin for Struct (super-trait) ---
    if has_plugin_super {
        // Find Plugin trait def to get its rust_path.
        let plugin_path = api
            .types
            .iter()
            .find(|t| t.is_trait && (t.name == "Plugin" || t.name.ends_with("::Plugin")))
            .map(|t| t.rust_path.replace('-', "_"))
            .unwrap_or_else(|| format!("{source_crate_name}::plugins::Plugin"));

        out.push_str(&crate::template_env::render(
            "rust_plugin_impl_open.jinja",
            minijinja::context! {
                plugin_path => plugin_path.as_str(),
                struct_name => struct_name.as_str(),
            },
        ));
        out.push_str("    fn name(&self) -> &str {\n");
        out.push_str("        &self.plugin_name\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str("    fn version(&self) -> String {\n");
        out.push_str("        self.plugin_version.clone()\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "rust_plugin_initialize.jinja",
            minijinja::context! {
                source_crate => source_crate_name,
            },
        ));
        out.push_str("        Ok(())\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "rust_plugin_shutdown.jinja",
            minijinja::context! {
                source_crate => source_crate_name,
            },
        ));
        out.push_str("        Ok(())\n");
        out.push_str("    }\n");
        out.push_str("}\n");
        out.push('\n');
    }

    // --- 3. impl Trait for Struct ---
    // async_trait macro is required for async methods in trait impls.
    let has_async = own_methods.iter().any(|m| m.is_async);
    if has_async {
        out.push_str("#[async_trait::async_trait]\n");
    }
    out.push_str(&crate::template_env::render(
        "rust_trait_impl_open.jinja",
        minijinja::context! {
            trait_path => trait_path.as_str(),
            struct_name => struct_name.as_str(),
        },
    ));
    for method in &own_methods {
        emit_trait_bridge_method(out, method, source_crate_name, type_paths);
        out.push('\n');
    }
    out.push_str("}\n");
    out.push('\n');

    // --- 4. Factory function ---
    out.push_str(&crate::template_env::render(
        "rust_trait_factory_doc.jinja",
        minijinja::context! {
            struct_name => struct_name.as_str(),
        },
    ));
    if has_plugin_super {
        out.push_str("/// `plugin_name` and `plugin_version` are required for the Plugin super-trait.\n");
    }
    out.push_str(&crate::template_env::render(
        "rust_trait_factory_fn.jinja",
        minijinja::context! {
            trait_snake => trait_snake.as_str(),
        },
    ));
    if has_plugin_super {
        out.push_str("    plugin_name: String,\n");
        out.push_str("    plugin_version: String,\n");
    }
    for method in &own_methods {
        let param_name = &method.name;
        let callback_ty = dart_fn_future_callback_type(method, source_crate_name, type_paths);
        out.push_str(&crate::template_env::render(
            "rust_trait_factory_param.jinja",
            minijinja::context! {
                param_name => param_name.as_str(),
                callback_ty => callback_ty,
            },
        ));
    }
    out.push_str(&crate::template_env::render(
        "rust_trait_factory_return.jinja",
        minijinja::context! {
            struct_name => struct_name.as_str(),
        },
    ));
    out.push_str(&crate::template_env::render(
        "rust_trait_factory_struct_init.jinja",
        minijinja::context! {
            struct_name => struct_name.as_str(),
        },
    ));
    if has_plugin_super {
        out.push_str("        plugin_name,\n");
        out.push_str("        plugin_version,\n");
    }
    for method in &own_methods {
        out.push_str(&crate::template_env::render(
            "rust_trait_factory_method_init.jinja",
            minijinja::context! {
                param_name => method.name.as_str(),
            },
        ));
    }
    out.push_str("    }\n");
    out.push_str("}\n");

    // --- 5. register_*/unregister_* forwarder functions ---
    // Emitted only when the bridge config sets `register_fn` (and optionally `unregister_fn`).
    // FRB auto-bridges these `pub fn` items so Dart sees them as `Future<void> registerOcrBackend(...)`
    // / `Future<void> unregisterOcrBackend(...)`.
    emit_register_forwarder(out, bridge_config, &struct_name, source_crate_name);
    emit_unregister_forwarder(out, bridge_config, source_crate_name);
}

/// Emit a Dart-side `register_*` forwarder for a configured trait bridge.
///
/// Wraps the user's `{Trait}DartImpl` in `std::sync::Arc::new(...)` and registers
/// it directly via the configured `registry_getter` (mirroring the PyO3/NAPI
/// approach). Going through the registry handle — rather than the host crate's
/// `register_*` free function — sidesteps the host's `pub(crate)` / `#[cfg(test)]`
/// restrictions on those wrappers (notably for `EmbeddingBackend`).
///
/// The forwarder returns `Result<(), String>` because FRB requires owned, FFI-
/// safe error types — the host's typed error is stringified for transport.
///
/// When `register_fn` is unset on the bridge config, no code is emitted.
fn emit_register_forwarder(
    out: &mut String,
    bridge_config: &TraitBridgeConfig,
    struct_name: &str,
    source_crate_name: &str,
) {
    let Some(register_fn) = bridge_config.register_fn.as_deref() else {
        return;
    };
    let Some(registry_getter) = bridge_config.registry_getter.as_deref() else {
        return;
    };
    let extra_args = bridge_config
        .register_extra_args
        .as_deref()
        .map(|a| format!(", {a}"))
        .unwrap_or_default();
    let trait_path = format!("{source_crate_name}::plugins::{}", bridge_config.trait_name);

    out.push('\n');
    out.push_str(&format!(
        "/// Register a Dart implementation as a `{}` plugin.\n",
        bridge_config.trait_name
    ));
    out.push_str("///\n");
    out.push_str(&format!(
        "/// Wraps `impl_` in an `Arc` and inserts it into `{registry_getter}()`.\n"
    ));
    out.push_str("/// Errors from the host registry are stringified for FRB transport.\n");
    out.push_str(&format!(
        "pub fn {register_fn}(impl_: {struct_name}) -> Result<(), String> {{\n"
    ));
    out.push_str(&format!(
        "    let arc: std::sync::Arc<dyn {trait_path}> = std::sync::Arc::new(impl_);\n"
    ));
    out.push_str(&format!("    let registry = {registry_getter}();\n"));
    out.push_str("    let mut registry = registry.write();\n");
    out.push_str(&format!(
        "    registry.register(arc{extra_args}).map_err(|e| e.to_string())\n"
    ));
    out.push_str("}\n");
}

/// Emit a Dart-side `unregister_*` forwarder for a configured trait bridge.
///
/// Removes a previously-registered plugin by name via the configured `registry_getter`.
/// Stringifies the host error. No-op when `unregister_fn` is unset on the bridge config.
fn emit_unregister_forwarder(out: &mut String, bridge_config: &TraitBridgeConfig, _source_crate_name: &str) {
    let Some(unregister_fn) = bridge_config.unregister_fn.as_deref() else {
        return;
    };
    let Some(registry_getter) = bridge_config.registry_getter.as_deref() else {
        return;
    };

    out.push('\n');
    out.push_str(&format!(
        "/// Unregister a previously-registered `{}` plugin by name.\n",
        bridge_config.trait_name
    ));
    out.push_str(&format!(
        "/// Removes the plugin from `{registry_getter}()` and stringifies any host error.\n"
    ));
    out.push_str(&format!(
        "pub fn {unregister_fn}(name: String) -> Result<(), String> {{\n"
    ));
    out.push_str(&format!("    let registry = {registry_getter}();\n"));
    out.push_str("    let mut registry = registry.write();\n");
    out.push_str("    registry.remove(&name).map_err(|e| e.to_string())\n");
    out.push_str("}\n");
}

/// Build the callback closure type stored in the bridge struct field.
///
/// Closures always accept **owned** FRB-friendly mirror types (the Dart FFI layer
/// decodes arguments as mirror types, not source-crate types). Returns a
/// `DartFnFuture<T>` wrapping the FRB-friendly mirror return type.
///
/// Example: `Box<dyn Fn(Vec<u8>, OcrConfig) -> DartFnFuture<ExtractionResult> + Send + Sync>`
fn dart_fn_future_callback_type(
    method: &MethodDef,
    _source_crate_name: &str,
    _type_paths: &std::collections::HashMap<String, String>,
) -> String {
    // Closures take owned FRB mirror types — use frb_rust_type (no source prefix).
    let params: Vec<String> = method.params.iter().map(|p| frb_rust_type(&p.ty, p.optional)).collect();

    let ret = frb_rust_type(&method.return_type, false);
    let dart_fn_ret = format!("flutter_rust_bridge::DartFnFuture<{ret}>");

    let params_str = params.join(", ");
    format!("Box<dyn Fn({params_str}) -> {dart_fn_ret} + Send + Sync>")
}

/// Emit one method implementation on the bridge struct.
///
/// The method signature must match the **original** trait signature (ref-aware,
/// original primitive widths). The closures stored in the struct hold
/// FRB-friendly widened types (e.g. `i64` for `u64`, `f64` for `f32`). The
/// impl body converts between the two representations.
///
/// For methods with an `error_type`, the return type is
/// `{source_crate}::Result<T>` — the Dart callback never fails, so the body
/// wraps the awaited value in `Ok(...)`.
fn emit_trait_bridge_method(
    out: &mut String,
    method: &MethodDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) {
    let method_name = &method.name;

    // Build the method signature matching the actual trait.
    // - Reference params use `&` / `&mut` prefix.
    // - Primitive params use their original width (not FRB-widened).
    let params_sig: Vec<String> = std::iter::once("&self".to_string())
        .chain(method.params.iter().map(|p| {
            let orig_ty = trait_impl_param_type(p, source_crate_name, type_paths);
            format!("{}: {orig_ty}", p.name)
        }))
        .collect();

    // Return type: use original primitive/named type; wrap in source-crate Result when error_type set.
    let ret = trait_impl_return_type(&method.return_type, source_crate_name, type_paths);
    let return_sig = if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            format!("{source_crate_name}::Result<()>")
        } else {
            format!("{source_crate_name}::Result<{ret}>")
        }
    } else {
        ret.clone()
    };

    let async_kw = if method.is_async { "async " } else { "" };
    out.push_str(&crate::template_env::render(
        "rust_method_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name.as_str(),
            params => params_sig.join(", "),
            return_sig => return_sig.as_str(),
        },
    ));

    // Emit owned-conversion let-bindings for each parameter before calling the closure.
    // References become owned; primitives may be widened; mut refs are copied for the callback.
    for p in &method.params {
        let conv = trait_impl_param_conversion(p);
        if !conv.is_empty() {
            out.push_str(&crate::template_env::render(
                "rust_trait_method_param_conversion.jinja",
                minijinja::context! {
                    conversion => conv,
                },
            ));
        }
    }

    // Build call-site arg list (use the local owned var names).
    let call_args: Vec<String> = method.params.iter().map(|p| p.name.clone()).collect();
    let call_expr = format!("(self.{method_name})({})", call_args.join(", "));

    // Emit the body, adapting the return value from FRB-widened to original type.
    let ret_conv = trait_impl_return_conversion(&method.return_type, source_crate_name);

    // Special case: Named return type — the mirror type cannot be trivially converted
    // back to the core type. Drop the result and return Default::default().
    let named_return_default = ret_conv == "__NAMED_RETURN_DEFAULT__";

    if method.error_type.is_some() {
        // DartFnFuture never fails: wrap the awaited value in Ok(...).
        if method.is_async {
            if named_return_default {
                out.push_str(&format!(
                    "        let _ = {call_expr}.await;\n        Ok(Default::default())\n"
                ));
            } else if ret_conv.is_empty() {
                out.push_str(&crate::template_env::render(
                    "rust_trait_method_ok_await.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "rust_trait_method_await_result.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                        ret_conv => ret_conv.as_str(),
                    },
                ));
            }
        } else {
            out.push_str("        let __result = tokio::runtime::Handle::current()\n");
            out.push_str(&crate::template_env::render(
                "rust_trait_method_block_on.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                },
            ));
            if named_return_default {
                out.push_str("        let _ = __result;\n        Ok(Default::default())\n");
            } else {
                // error_type present: the Dart callback never fails, so wrap in Ok(...).
                out.push_str(&crate::template_env::render(
                    "rust_trait_method_ok_block_on.jinja",
                    minijinja::context! {
                        ret_conv => ret_conv.as_str(),
                    },
                ));
            }
        }
    } else if method.is_async {
        if named_return_default {
            out.push_str(&format!(
                "        let _ = {call_expr}.await;\n        Default::default()\n"
            ));
        } else if ret_conv.is_empty() {
            out.push_str(&crate::template_env::render(
                "rust_trait_method_await_plain.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "rust_trait_method_await_result.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    ret_conv => ret_conv.as_str(),
                },
            ));
        }
    } else {
        out.push_str("        let __result = tokio::runtime::Handle::current()\n");
        out.push_str(&crate::template_env::render(
            "rust_trait_method_block_on.jinja",
            minijinja::context! {
                call_expr => call_expr.as_str(),
            },
        ));
        if named_return_default {
            out.push_str("        let _ = __result;\n        Default::default()\n");
        } else {
            // No error_type: return the plain value (no Ok() wrapping).
            out.push_str(&format!("        __result{ret_conv}\n"));
        }
    }
    out.push_str("    }\n");
}
