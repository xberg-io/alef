use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};
use heck::ToSnakeCase;

use super::conversions::frb_rust_type_excluded_aware;
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

    // Filter to own methods that the foreign object must provide.
    // - `trait_source.is_none()` excludes methods inherited from super-traits (handled
    //   separately: `Plugin` via the dedicated impl below, other super-traits via stubs).
    // - Methods with `has_default_impl = true` are intentionally included: the bridge exists
    //   precisely to dispatch to Dart-side implementations. Relying on the Rust default impl
    //   would silently no-op every visitor/plugin callback (D8 fix).
    // - Methods whose return type references another trait (e.g. `Option<&dyn SyncExtractor>`)
    //   are NOT bridgeable to Dart — the foreign side cannot construct or return a Rust
    //   trait object across FFI. Skip them and let the Rust trait's default impl handle
    //   the receiver. The skipped methods must have `has_default_impl = true`; otherwise
    //   the emitted `impl Trait for Struct` will fail to compile because the required
    //   method is missing.
    let own_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !return_type_references_trait(&m.return_type, api))
        .collect();

    // Check if Plugin is a direct super-trait.
    let has_plugin_super = trait_def
        .super_traits
        .iter()
        .any(|s| s == "Plugin" || s.ends_with("::Plugin"));

    // The `type_alias` mode (e.g. `VisitorHandle` for the `HtmlVisitor` trait) wraps the
    // Rust-side impl in the trait's `Arc<Mutex<dyn Trait + Send>>` alias before handing
    // it back to Dart. In that mode:
    //
    //   - The impl struct is PRIVATE (no `pub`, no `#[frb(opaque)]`) so FRB never sees
    //     it. This avoids FRB v2's failure mode where `Box<dyn Fn(...)>` fields on an
    //     opaque struct render as uninstantiable opaque callback classes on the Dart side.
    //   - The factory takes closures as `impl Fn(...) -> DartFnFuture<R> + Send + Sync +
    //     'static` parameters — FRB synthesises Dart-callable function types for closure
    //     **parameters** (but not for closure **fields** on opaque structs).
    //   - The factory returns the already-emitted local `type_alias` opaque wrapper
    //     (e.g. `VisitorHandle { inner: Arc<Mutex<...>> }`) which IS exposed to FRB.
    //
    // Bridge configs WITHOUT `type_alias` (the plugin/ocr pattern) keep the legacy
    // factory shape: a `#[frb(opaque)] pub struct TraitDartImpl { Box<dyn Fn(...)> }`
    // exposed directly to FRB and handed to a `register_*` forwarder. Those callsites
    // use the Box-typed fields internally and never construct callbacks from Dart user
    // code — so the FRB-opaque-callback limitation does not bite.
    let uses_type_alias = bridge_config.type_alias.is_some();

    // --- 1. Impl struct holding Dart callbacks ---
    if uses_type_alias {
        out.push_str("/// Internal Rust-side storage for Dart-provided visitor callbacks.\n");
        out.push_str("/// Not exposed via FRB (private to the bridge crate); the public factory\n");
        out.push_str("/// `create_{trait_snake}(...)` wraps this in the trait's configured `type_alias`\n");
        out.push_str("/// (e.g. `VisitorHandle`) which FRB does expose as opaque.\n");
        out.push_str(&format!("struct {struct_name} {{\n"));
    } else {
        out.push_str("/// FRB opaque handle holding Dart callbacks for each trait method.\n");
        out.push_str("/// Dart-side: register callbacks via `create_{snake}_dart_impl(...)` factory.\n");
        out.push_str("#[frb(opaque)]\n");
        out.push_str(&crate::template_env::render(
            "rust_mirror_struct_open.jinja",
            minijinja::context! {
                name => struct_name.as_str(),
            },
        ));
    }
    // Plugin fields for name/version (required by Plugin super-trait).
    if has_plugin_super {
        out.push_str("    /// Plugin name used by the Plugin super-trait impl.\n");
        out.push_str("    plugin_name: String,\n");
        out.push_str("    /// Plugin version used by the Plugin super-trait impl.\n");
        out.push_str("    plugin_version: String,\n");
    }
    for method in &own_methods {
        let field_name = &method.name;
        let callback_ty = dart_fn_future_callback_type(method, source_crate_name, type_paths, &api.excluded_type_paths);
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
    // D4: emit a manual Debug impl so the struct satisfies `Debug` supertrait bounds
    // (e.g. `pub trait HtmlVisitor: Debug + Send`). Closure fields are not Debug;
    // we use `finish_non_exhaustive()` to produce a valid but opaque representation.
    out.push_str(&format!(
        "impl ::std::fmt::Debug for {struct_name} {{\n    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {{\n        f.debug_struct(\"{struct_name}\").finish_non_exhaustive()\n    }}\n}}\n"
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
        emit_trait_bridge_method(out, method, source_crate_name, type_paths, &api.excluded_type_paths);
        out.push('\n');
    }
    out.push_str("}\n");
    out.push('\n');

    // --- 4. Factory function ---
    // Two emission shapes:
    //
    // (A) `type_alias` is set (visitor pattern): factory takes closures as
    //     `impl Fn(...) -> DartFnFuture<R> + Send + Sync + 'static` parameters and returns
    //     the already-emitted local opaque wrapper. FRB synthesises a Dart-callable
    //     function type for each closure parameter (whereas closure FIELDS on opaque
    //     structs render as uninstantiable opaque types in FRB v2).
    //
    // (B) `type_alias` is unset (plugin/ocr pattern): legacy factory shape — takes
    //     `Box<dyn Fn(...) -> DartFnFuture<R> + Send + Sync>` and returns the opaque
    //     bridge struct directly. The Dart-side wiring goes through `register_*` /
    //     `unregister_*` forwarders that consume the bridge struct opaquely.
    if uses_type_alias {
        let type_alias = bridge_config.type_alias.as_deref().unwrap_or("");
        // Locate the local opaque-wrapper TypeDef so we can pull its `rust_path` (the
        // qualified core path, e.g. `html_to_markdown_rs::visitor::VisitorHandle`).
        let alias_def = api.types.iter().find(|t| t.name == type_alias);
        let inner_path = match alias_def {
            Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
            _ => format!("{}::{}", source_crate_name.replace('-', "_"), type_alias),
        };

        out.push_str(&format!(
            "/// Construct a `{type_alias}` from Dart callback closures.\n"
        ));
        out.push_str("/// FRB synthesises a Dart-callable function type for each closure parameter,\n");
        out.push_str("/// which is the whole point of taking them as `impl Fn(...) -> DartFnFuture<R>`\n");
        out.push_str("/// parameters rather than storing them as `Box<dyn Fn(...)>` fields on an\n");
        out.push_str("/// opaque struct (FRB v2 cannot generate callable closure types in that shape).\n");
        if has_plugin_super {
            out.push_str("/// `plugin_name` and `plugin_version` are required for the Plugin super-trait.\n");
        }
        out.push_str(&format!("pub async fn create_{trait_snake}(\n"));
        if has_plugin_super {
            out.push_str("    plugin_name: String,\n");
            out.push_str("    plugin_version: String,\n");
        }
        for method in &own_methods {
            let param_name = &method.name;
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| frb_rust_type_excluded_aware(&p.ty, p.optional, &api.excluded_type_paths))
                .collect();
            let ret = frb_rust_type_excluded_aware(&method.return_type, false, &api.excluded_type_paths);
            let params_str = params.join(", ");
            // FRB v2's closure-parameter parser matches the return type by inspecting
            // the FIRST path segment of the return type (`path.segments.first().ident`).
            // A fully-qualified `flutter_rust_bridge::DartFnFuture<...>` makes that first
            // segment resolve to `flutter_rust_bridge`, causing the parser to bail with
            // "DartFn does not support return types except `DartFnFuture<T>` yet". Use
            // the bare ident — `DartFnFuture` is already brought into scope via the
            // `pub use flutter_rust_bridge::DartFnFuture` at the top of every generated
            // lib.rs (see `gen_rust_crate::mod::generate_lib_rs`).
            out.push_str(&format!(
                "    {param_name}: impl Fn({params_str}) -> DartFnFuture<{ret}> + Send + Sync + 'static,\n"
            ));
        }
        out.push_str(&format!(") -> {type_alias} {{\n"));
        out.push_str(&format!("    let __impl = {struct_name} {{\n"));
        if has_plugin_super {
            out.push_str("        plugin_name,\n");
            out.push_str("        plugin_version,\n");
        }
        for method in &own_methods {
            out.push_str(&format!("        {name}: Box::new({name}),\n", name = method.name));
        }
        out.push_str("    };\n");
        // VisitorHandle is `Arc<Mutex<dyn HtmlVisitor + Send>>`. Build the inner alias and
        // wrap it in the local opaque struct via its `From<core_type>` impl.
        out.push_str(&format!(
            "    let __inner: {inner_path} = std::sync::Arc::new(std::sync::Mutex::new(__impl));\n"
        ));
        out.push_str(&format!("    {type_alias}::from(__inner)\n"));
        out.push_str("}\n");

        // --- 4b. Options-builder helper (options_field binding only) ---
        //
        // ConversionOptions is a mirror struct rendered as a Dart class with `final` fields
        // and a `const` constructor — there is no copyWith and no way to set `visitor` after
        // construction. To thread a visitor handle into an options blob loaded from JSON
        // (e.g. the e2e fixture pattern), we emit a small Rust helper:
        //
        //     pub fn create_<options>_from_json_with_<field>(json, visitor) -> Result<Mirror, String>
        //
        // It deserialises the core options, sets the `visitor` field on the core value, then
        // converts to the mirror type via the already-emitted `From<core>` impl.
        if bridge_config.bind_via == alef_core::config::BridgeBinding::OptionsField {
            if let (Some(options_type), Some(field_raw)) = (
                bridge_config.options_type.as_deref(),
                bridge_config.resolved_options_field(),
            ) {
                let field = field_raw.to_string();
                let options_snake = options_type.to_snake_case();
                let opts_def = api.types.iter().find(|t| t.name == options_type);
                let core_options_path = match opts_def {
                    Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
                    _ => format!("{}::{}", source_crate_name.replace('-', "_"), options_type),
                };
                out.push('\n');
                out.push_str(&format!(
                    "/// Build a `{options_type}` from a JSON blob and attach a Dart-built\n"
                ));
                out.push_str(&format!(
                    "/// `{type_alias}` to its `{field}` field. The mirror struct uses `final`\n"
                ));
                out.push_str("/// dart fields, so callers cannot patch the visitor in after JSON load —\n");
                out.push_str("/// this helper does the merge on the Rust side instead.\n");
                out.push_str("#[frb]\n");
                out.push_str(&format!(
                    "pub fn create_{options_snake}_from_json_with_{field}(\n    json: String,\n    {field}: Option<{type_alias}>,\n) -> Result<{options_type}, String> {{\n"
                ));
                out.push_str(&format!(
                    "    let mut __core: {core_options_path} = serde_json::from_str(&json).map_err(|e| e.to_string())?;\n"
                ));
                out.push_str(&format!("    __core.{field} = {field}.map(<{inner_path}>::from);\n"));
                out.push_str(&format!("    Ok({options_type}::from(__core))\n"));
                out.push_str("}\n");
            }
        }
    } else {
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
            let callback_ty =
                dart_fn_future_callback_type(method, source_crate_name, type_paths, &api.excluded_type_paths);
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
    }

    // --- 5. register_*/unregister_*/clear_* forwarder functions ---
    // Emitted only when the bridge config sets `register_fn` (and optionally `unregister_fn`
    // / `clear_fn`). FRB auto-bridges these `pub fn` items so Dart sees them as:
    //   Future<void> registerOcrBackend(...)
    //   Future<void> unregisterOcrBackend(...)
    //   Future<void> clearOcrBackends()
    emit_register_forwarder(out, bridge_config, &struct_name, source_crate_name);
    emit_unregister_forwarder(out, bridge_config, source_crate_name);
    emit_clear_forwarder(out, bridge_config, source_crate_name);
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

    out.push_str(&crate::template_env::render(
        "rust_trait_register_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            registry_getter => registry_getter,
            register_fn => register_fn,
            struct_name => struct_name,
            trait_path => trait_path.as_str(),
            extra_args => extra_args.as_str(),
        },
    ));
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

    out.push_str(&crate::template_env::render(
        "rust_trait_unregister_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            registry_getter => registry_getter,
            unregister_fn => unregister_fn,
        },
    ));
}

/// Emit a Rust-side `clear_*` forwarder for a configured trait bridge.
///
/// Removes ALL previously-registered plugins of this type via the configured `registry_getter`.
/// Stringifies the host error. No-op when `clear_fn` is unset on the bridge config.
fn emit_clear_forwarder(out: &mut String, bridge_config: &TraitBridgeConfig, _source_crate_name: &str) {
    let Some(clear_fn) = bridge_config.clear_fn.as_deref() else {
        return;
    };
    let Some(registry_getter) = bridge_config.registry_getter.as_deref() else {
        return;
    };

    out.push_str(&crate::template_env::render(
        "rust_trait_clear_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            registry_getter => registry_getter,
            clear_fn => clear_fn,
        },
    ));
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
    excluded_type_paths: &std::collections::HashMap<String, String>,
) -> String {
    // Closures take owned FRB mirror types — use frb_rust_type (no source prefix)
    // for types with an in-scope mirror, and the qualified source-crate path for
    // excluded internal types (e.g. `InternalDocument`) that have no mirror struct.
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| frb_rust_type_excluded_aware(&p.ty, p.optional, excluded_type_paths))
        .collect();

    let ret = frb_rust_type_excluded_aware(&method.return_type, false, excluded_type_paths);
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
    excluded_type_paths: &std::collections::HashMap<String, String>,
) {
    let method_name = &method.name;

    // Build the method signature matching the actual trait.
    // - Reference params use `&` / `&mut` prefix.
    // - Primitive params use their original width (not FRB-widened).
    // Emit the self receiver matching the trait definition so rustc's E0053
    // ("method has an incompatible type for trait") is not triggered for
    // traits that use `&mut self` (e.g. `HtmlVisitor`).
    let self_receiver = match method.receiver {
        Some(ReceiverKind::RefMut) => "&mut self",
        Some(ReceiverKind::Owned) => "self",
        // Default: `&self` (covers `Some(ReceiverKind::Ref)` and `None`).
        _ => "&self",
    };
    let params_sig: Vec<String> = std::iter::once(self_receiver.to_string())
        .chain(method.params.iter().map(|p| {
            let orig_ty = trait_impl_param_type(p, source_crate_name, type_paths);
            format!("{}: {orig_ty}", p.name)
        }))
        .collect();

    // Detect the `&[&str]` (Vec<String> + returns_ref) special case — the trait method
    // expects a borrowed static slice but the Dart-side closure produces owned
    // `Vec<String>`. We materialise that into `&'static [&'static str]` via Box::leak
    // (same pattern as the napi/pyo3 trait-bridges, see
    // `alef-codegen::trait_bridge::gen_method`). The owned vector is leaked once per
    // method invocation: acceptable for plugin metadata that's typically read at
    // registration time.
    let is_ref_slice_of_str = method.returns_ref
        && matches!(
            &method.return_type,
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)
        );
    // Return type: use original primitive/named type; wrap in source-crate Result when error_type set.
    let ret = if is_ref_slice_of_str {
        "&[&str]".to_string()
    } else {
        trait_impl_return_type(&method.return_type, source_crate_name, type_paths)
    };
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
        let conv = trait_impl_param_conversion(p, excluded_type_paths);
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
                out.push_str(&crate::template_env::render(
                    "rust_trait_method_default_await.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                        return_expr => "Ok(Default::default())",
                    },
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
            // FRB workers don't have a tokio runtime installed; `Handle::current()` would
            // panic. Build a fresh current-thread runtime per call to drive the DartFnFuture
            // — overhead is acceptable since visitor callbacks already cross an FFI boundary
            // and the runtime is cheap to construct (no I/O drivers needed).
            out.push_str("        let __result = ::tokio::runtime::Builder::new_current_thread()\n            .build()\n            .expect(\"build alef visitor tokio runtime\")\n");
            out.push_str(&crate::template_env::render(
                "rust_trait_method_block_on.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                },
            ));
            if named_return_default {
                out.push_str(&crate::template_env::render(
                    "rust_trait_method_default_from_result.jinja",
                    minijinja::context! {
                        return_expr => "Ok(Default::default())",
                    },
                ));
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
            out.push_str(&crate::template_env::render(
                "rust_trait_method_default_await.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    return_expr => "Default::default()",
                },
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
        // FRB workers don't have a tokio runtime installed; `Handle::current()` would
        // panic. Build a fresh current-thread runtime per call to drive the DartFnFuture
        // — overhead is acceptable since visitor callbacks already cross an FFI boundary
        // and the runtime is cheap to construct (no I/O drivers needed).
        out.push_str("        let __result = ::tokio::runtime::Builder::new_current_thread()\n            .build()\n            .expect(\"build alef visitor tokio runtime\")\n");
        out.push_str(&crate::template_env::render(
            "rust_trait_method_block_on.jinja",
            minijinja::context! {
                call_expr => call_expr.as_str(),
            },
        ));
        if named_return_default {
            out.push_str(&crate::template_env::render(
                "rust_trait_method_default_from_result.jinja",
                minijinja::context! {
                    return_expr => "Default::default()",
                },
            ));
        } else if is_ref_slice_of_str {
            // Materialise `Vec<String>` into `&'static [&'static str]` so the trait
            // method's `&[&str]` return type is satisfied. Each closure invocation
            // leaks its strings — acceptable for plugin-metadata callsites.
            out.push_str(
                "            ;\n        \
                 let __strs: Vec<&'static str> = __result\n            \
                 .into_iter()\n            \
                 .map(|s| -> &'static str { Box::leak(s.into_boxed_str()) })\n            \
                 .collect();\n        \
                 Box::leak(__strs.into_boxed_slice())\n",
            );
        } else {
            // No error_type: return the plain value (no Ok() wrapping).
            out.push_str(&crate::template_env::render(
                "rust_trait_method_plain_block_on_result.jinja",
                minijinja::context! {
                    ret_conv => ret_conv.as_str(),
                },
            ));
        }
    }
    out.push_str("    }\n");
}

/// Returns true if `ty` references a `Named(name)` at any depth where `name` resolves
/// to a trait — either present in `api.types` or stripped from the binding surface
/// (`api.excluded_trait_names`). Such methods return references to trait objects
/// (`&dyn Trait`, `Option<&dyn Trait>`, `Box<dyn Trait>`) which the Rust IR flattens
/// to `Named(name)`. They cannot be bridged to Dart — the foreign side has no way to
/// construct or return a Rust trait object across FFI — so the trait-bridge generator
/// skips them and falls back to the trait's default impl.
///
/// The `excluded_trait_names` lookup is necessary because traits annotated with
/// `#[cfg_attr(alef, alef(skip))]` (e.g. `SyncExtractor`) are stripped from `api.types`
/// before codegen, but their NAME may still appear in surviving trait method return
/// signatures (e.g. `DocumentExtractor::as_sync_extractor() -> Option<&dyn SyncExtractor>`).
/// Without this fallback, the bridge struct would emit a closure field with the trait
/// path used as a TYPE (`Option<kreuzberg::extractors::SyncExtractor>`), producing
/// `error[E0782]: expected a type, found a trait`. Restricting the check to trait-shaped
/// excluded items (not all excluded items) keeps methods returning excluded structs
/// (`extract_bytes -> Result<InternalDocument>`) emitted, since `InternalDocument` is a
/// concrete struct usable by its qualified core path.
pub(crate) fn return_type_references_trait(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(name) => {
            api.types.iter().any(|t| t.is_trait && &t.name == name) || api.excluded_trait_names.contains(name)
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => return_type_references_trait(inner, api),
        TypeRef::Map(k, v) => return_type_references_trait(k, api) || return_type_references_trait(v, api),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{ApiSurface, TypeDef, TypeRef};

    fn empty_type_def(name: &str, is_trait: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn api_surface(types: Vec<TypeDef>, excluded_paths: Vec<(&str, &str)>, excluded_traits: Vec<&str>) -> ApiSurface {
        ApiSurface {
            types,
            excluded_type_paths: excluded_paths
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            excluded_trait_names: excluded_traits.into_iter().map(String::from).collect(),
            ..ApiSurface::default()
        }
    }

    #[test]
    fn return_type_references_in_surface_trait() {
        // Sanity check: pre-existing behaviour — traits present in api.types are detected.
        let api = api_surface(vec![empty_type_def("MyTrait", true)], vec![], vec![]);
        let ret = TypeRef::Optional(Box::new(TypeRef::Named("MyTrait".into())));
        assert!(return_type_references_trait(&ret, &api));
    }

    #[test]
    fn return_type_references_excluded_trait_is_detected() {
        // Regression: a trait stripped from api.types via `alef(skip)` must still be
        // detected via `excluded_trait_names`, otherwise the trait-bridge field is emitted
        // and the generated `Box<dyn Fn() -> DartFnFuture<Option<demo::SyncExtractor>>>`
        // fails to compile with E0782 (`SyncExtractor` is a trait, not a type).
        let api = api_surface(
            vec![],
            vec![("SyncExtractor", "demo::extractors::SyncExtractor")],
            vec!["SyncExtractor"],
        );
        let ret = TypeRef::Optional(Box::new(TypeRef::Named("SyncExtractor".into())));
        assert!(return_type_references_trait(&ret, &api));
    }

    #[test]
    fn return_type_with_excluded_struct_is_not_detected() {
        // Regression: excluded structs (e.g. `InternalDocument`) appear by qualified path
        // in surviving method signatures (`extract_bytes -> Result<InternalDocument>`) and
        // ARE bridgeable — they must NOT be filtered out, or the trait impl ends up missing
        // a required method (`error[E0046]: not all trait items implemented`).
        let api = api_surface(
            vec![],
            vec![("InternalDocument", "demo::types::internal::InternalDocument")],
            vec![],
        );
        let ret = TypeRef::Named("InternalDocument".into());
        assert!(!return_type_references_trait(&ret, &api));
    }

    #[test]
    fn return_type_with_unrelated_named_is_not_detected() {
        let api = api_surface(vec![empty_type_def("MyStruct", false)], vec![], vec![]);
        let ret = TypeRef::Optional(Box::new(TypeRef::Named("MyStruct".into())));
        assert!(!return_type_references_trait(&ret, &api));
    }
}
