use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

use super::conversions::frb_rust_type_with_source;
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
    let own_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none())
        .collect();

    // Check if Plugin is a direct super-trait.
    let has_plugin_super = trait_def.super_traits.iter().any(|s| s == "Plugin" || s.ends_with("::Plugin"));

    // --- 1. Opaque struct with one closure field per method ---
    out.push_str("/// FRB opaque handle holding Dart callbacks for each trait method.\n");
    out.push_str("/// Dart-side: register callbacks via `create_{snake}_dart_impl(...)` factory.\n");
    out.push_str("#[frb(opaque)]\n");
    out.push_str(&format!("pub struct {struct_name} {{\n"));
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
        out.push_str(&format!("    {field_name}: {callback_ty},\n"));
    }
    out.push_str("}\n");
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

        out.push_str(&format!("impl {plugin_path} for {struct_name} {{\n"));
        out.push_str("    fn name(&self) -> &str {\n");
        out.push_str("        &self.plugin_name\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str("    fn version(&self) -> String {\n");
        out.push_str("        self.plugin_version.clone()\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str(&format!(
            "    fn initialize(&self) -> {source_crate_name}::Result<()> {{\n"
        ));
        out.push_str("        Ok(())\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str(&format!(
            "    fn shutdown(&self) -> {source_crate_name}::Result<()> {{\n"
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
    out.push_str(&format!("impl {trait_path} for {struct_name} {{\n"));
    for method in &own_methods {
        emit_trait_bridge_method(out, method, source_crate_name, type_paths);
        out.push('\n');
    }
    out.push_str("}\n");
    out.push('\n');

    // --- 4. Factory function ---
    out.push_str(&format!("/// Create a `{struct_name}` from Dart callback closures.\n"));
    out.push_str("/// Each method parameter is a `DartFnFuture`-returning closure.\n");
    if has_plugin_super {
        out.push_str("/// `plugin_name` and `plugin_version` are required for the Plugin super-trait.\n");
    }
    out.push_str(&format!("pub fn create_{trait_snake}_dart_impl(\n"));
    if has_plugin_super {
        out.push_str("    plugin_name: String,\n");
        out.push_str("    plugin_version: String,\n");
    }
    for method in &own_methods {
        let param_name = &method.name;
        let callback_ty = dart_fn_future_callback_type(method, source_crate_name, type_paths);
        out.push_str(&format!("    {param_name}: {callback_ty},\n"));
    }
    out.push_str(&format!(") -> {struct_name} {{\n"));
    out.push_str(&format!("    {struct_name} {{\n"));
    if has_plugin_super {
        out.push_str("        plugin_name,\n");
        out.push_str("        plugin_version,\n");
    }
    for method in &own_methods {
        out.push_str(&format!("        {},\n", method.name));
    }
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Build the callback closure type stored in the bridge struct field.
///
/// Closures always accept **owned** FRB-friendly types (the Dart FFI layer passes
/// owned values). Reference parameters in the original trait are converted to owned
/// before the closure is invoked. Returns a `DartFnFuture<T>` wrapping the
/// FRB-friendly return type.
///
/// Example: `Box<dyn Fn(Vec<u8>, OcrConfig) -> DartFnFuture<ExtractionResult> + Send + Sync>`
fn dart_fn_future_callback_type(
    method: &MethodDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    // Closures take owned FRB-friendly types — strip any reference wrappers.
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| frb_rust_type_with_source(&p.ty, p.optional, source_crate_name, type_paths))
        .collect();

    let ret = frb_rust_type_with_source(&method.return_type, false, source_crate_name, type_paths);
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
    let sig_line = format!("    {async_kw}fn {method_name}({}) -> {return_sig} {{", params_sig.join(", "));
    out.push_str(&sig_line);
    out.push('\n');

    // Emit owned-conversion let-bindings for each parameter before calling the closure.
    // References become owned; primitives may be widened; mut refs are copied for the callback.
    for p in &method.params {
        let conv = trait_impl_param_conversion(p);
        if !conv.is_empty() {
            out.push_str(&format!("        {conv}\n"));
        }
    }

    // Build call-site arg list (use the local owned var names).
    let call_args: Vec<String> = method.params.iter().map(|p| p.name.clone()).collect();
    let call_expr = format!("(self.{method_name})({})", call_args.join(", "));

    // Emit the body, adapting the return value from FRB-widened to original type.
    let ret_conv = trait_impl_return_conversion(&method.return_type);

    if method.error_type.is_some() {
        // DartFnFuture never fails: wrap the awaited value in Ok(...).
        if method.is_async {
            if ret_conv.is_empty() {
                out.push_str(&format!("        Ok({call_expr}.await)\n"));
            } else {
                out.push_str(&format!("        Ok({call_expr}.await{ret_conv})\n"));
            }
        } else {
            out.push_str("        let __result = tokio::runtime::Handle::current()\n");
            out.push_str(&format!("            .block_on(async {{ {call_expr}.await }});\n"));
            if ret_conv.is_empty() {
                out.push_str("        Ok(__result)\n");
            } else {
                out.push_str(&format!("        Ok(__result{ret_conv})\n"));
            }
        }
    } else if method.is_async {
        if ret_conv.is_empty() {
            out.push_str(&format!("        {call_expr}.await\n"));
        } else {
            out.push_str(&format!("        ({call_expr}.await){ret_conv}\n"));
        }
    } else {
        out.push_str("        let __result = tokio::runtime::Handle::current()\n");
        out.push_str(&format!("            .block_on(async {{ {call_expr}.await }});\n"));
        if ret_conv.is_empty() {
            out.push_str("        __result\n");
        } else {
            out.push_str(&format!("        __result{ret_conv}\n"));
        }
    }
    out.push_str("    }\n");
}
