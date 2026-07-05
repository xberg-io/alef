use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, MethodDef, TypeDef};
use heck::ToSnakeCase;

use super::analysis::return_type_references_trait;
use super::callbacks::{dart_fn_future_callback_type, dart_fn_future_factory_param_type};
use super::forwarders::{emit_clear_forwarder, emit_register_forwarder, emit_unregister_forwarder};
use super::methods::emit_trait_bridge_method;
use crate::backends::dart::gen_rust_crate::conversions::frb_rust_type_excluded_aware;

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
///    other super-traits emit an unsupported comment stub.
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
    lifetime_type_names: &std::collections::HashSet<String>,
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
    // Bridge configs WITHOUT `type_alias` (the registry-factory pattern) keep the legacy
    // factory shape: a `#[frb(opaque)] pub struct TraitDartImpl { Box<dyn Fn(...)> }`
    // exposed directly to FRB and handed to a `register_*` forwarder. Those callsites
    // use the Box-typed fields internally and never construct callbacks from Dart user
    // code — so the FRB-opaque-callback limitation does not bite.
    let uses_type_alias = bridge_config.type_alias.is_some();

    // The closure-bearing struct is ALWAYS private. FRB v2 walks `#[frb(opaque)]`
    // struct fields and silently drops the surrounding factory if it finds
    // `Box<dyn Fn(...)>` closures it cannot bridge. In the type-alias path the
    // wrapper is the configured `type_alias` (e.g. `VisitorHandle`); in the
    // non-type-alias plugin path the wrapper is a synthesised
    // `#[frb(opaque)] pub struct {Trait}DartImpl(pub Arc<dyn Trait + Send + Sync>)`
    // emitted after the trait impls (see section 3b below).
    let callbacks_struct_name = if uses_type_alias {
        struct_name.clone()
    } else {
        format!("{trait_name}DartCallbacks")
    };

    // --- 1. Impl struct holding Dart callbacks (private) ---
    if uses_type_alias {
        out.push_str("/// Internal Rust-side storage for Dart-provided visitor callbacks.\n");
        out.push_str("/// Not exposed via FRB (private to the bridge crate); the public factory\n");
        out.push_str("/// `create_{trait_snake}(...)` wraps this in the trait's configured `type_alias`\n");
        out.push_str("/// (e.g. `VisitorHandle`) which FRB does expose as opaque.\n");
    } else {
        out.push_str("/// Internal Rust-side storage for Dart-provided plugin callbacks.\n");
        out.push_str("/// Not exposed via FRB (private to the bridge crate). The public factory\n");
        out.push_str("/// `create_{trait_snake}_dart_impl(...)` wraps an `Arc<dyn Trait + Send + Sync>`\n");
        out.push_str("/// of this struct in the public opaque `{Trait}DartImpl` newtype. Hiding the\n");
        out.push_str("/// closure fields behind the wrapper keeps FRB from walking them and silently\n");
        out.push_str("/// dropping the factory (FRB v2 cannot generate callable Dart classes for\n");
        out.push_str("/// `Box<dyn Fn(...)>` opaque-struct fields).\n");
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_mirror_struct_open.jinja",
        minijinja::context! {
            name => callbacks_struct_name.as_str(),
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
        let callback_ty = dart_fn_future_callback_type(method, source_crate_name, type_paths, &api.excluded_type_paths);
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_struct_field.jinja",
            minijinja::context! {
                field_name => field_name.as_str(),
                callback_ty => callback_ty,
            },
        ));
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_mirror_struct_close.jinja",
        minijinja::context! {},
    ));
    // D4: emit a manual Debug impl so the struct satisfies `Debug` supertrait bounds
    // (e.g. `pub trait HtmlVisitor: Debug + Send`). Closure fields are not Debug;
    // we use `finish_non_exhaustive()` to produce a valid but opaque representation.
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_callbacks_debug_impl.rs.jinja",
        minijinja::context! {
            callbacks_struct_name => callbacks_struct_name.as_str(),
        },
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

        out.push_str(&crate::backends::dart::template_env::render(
            "rust_plugin_impl_open.jinja",
            minijinja::context! {
                plugin_path => plugin_path.as_str(),
                struct_name => callbacks_struct_name.as_str(),
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
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_plugin_initialize.jinja",
            minijinja::context! {
                source_crate => source_crate_name,
            },
        ));
        out.push_str("        Ok(())\n");
        out.push_str("    }\n");
        out.push('\n');
        out.push_str(&crate::backends::dart::template_env::render(
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
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_trait_impl_open.jinja",
        minijinja::context! {
            trait_path => trait_path.as_str(),
            struct_name => callbacks_struct_name.as_str(),
        },
    ));
    for method in &own_methods {
        emit_trait_bridge_method(
            out,
            method,
            callbacks_struct_name.as_str(),
            source_crate_name,
            type_paths,
            &api.excluded_type_paths,
            lifetime_type_names,
        );
        out.push('\n');
    }
    out.push_str("}\n");
    out.push('\n');

    // --- 3b. Public opaque wrapper (non-type-alias path only) ---
    // FRB sees ONLY this single-field tuple struct: an `Arc<dyn Trait + Send + Sync>`
    // already constructed by the factory. The closure-bearing struct above stays
    // invisible to FRB. The wrapper is what Dart user code holds and what the
    // register forwarder consumes.
    //
    // The trait name inside `Arc<dyn …>` is emitted UNQUALIFIED. FRB v2 strips
    // `dyn` and the qualified path when copying the inner type into the generated
    // `frb_generated.rs` (it writes `Arc<DocumentExtractor + Send + Sync>` not
    // `Arc<dyn example_crate::plugins::DocumentExtractor + …>`). `frb_generated.rs`
    // imports `use crate::*;` at its top, so a sibling `pub use {trait_path};`
    // at this site makes the bare trait ident resolvable from inside FRB's code.
    if !uses_type_alias {
        // Named field literally `field0` to match FRB v2's auto-opaque accessor
        // codegen, which references `api_that_guard.field0` regardless of whether
        // the source was a tuple struct (`.0`) or a single-field named struct.
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_reexport_opaque_wrapper.rs.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                trait_path => trait_path.as_str(),
                trait_snake => trait_snake.as_str(),
                struct_name => struct_name.as_str(),
            },
        ));
        out.push('\n');
    }

    // --- 4. Factory function ---
    // Two emission shapes:
    //
    // (A) `type_alias` is set (visitor pattern): factory takes closures as
    //     `impl Fn(...) -> DartFnFuture<R> + Send + Sync + 'static` parameters and returns
    //     the already-emitted local opaque wrapper. FRB synthesises a Dart-callable
    //     function type for each closure parameter (whereas closure FIELDS on opaque
    //     structs render as uninstantiable opaque types in FRB v2).
    //
    // (B) `type_alias` is unset (registry-factory pattern): legacy factory shape — takes
    //     `Box<dyn Fn(...) -> DartFnFuture<R> + Send + Sync>` and returns the opaque
    //     bridge struct directly. The Dart-side wiring goes through `register_*` /
    //     `unregister_*` forwarders that consume the bridge struct opaquely.
    if uses_type_alias {
        let type_alias = bridge_config.type_alias.as_deref().unwrap_or("");
        // Locate the local opaque-wrapper TypeDef so we can pull its `rust_path` (the
        // qualified core path, e.g. `sample_markdown_rs::visitor::VisitorHandle`).
        let alias_def = api.types.iter().find(|t| t.name == type_alias);
        let inner_path = match alias_def {
            Some(td) if !td.rust_path.is_empty() => td.rust_path.replace('-', "_"),
            _ => format!("{}::{}", source_crate_name.replace('-', "_"), type_alias),
        };

        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_type_alias_factory_doc.jinja",
            minijinja::context! {
                type_alias => type_alias,
                has_plugin_super => has_plugin_super,
            },
        ));
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_type_alias_factory_open.jinja",
            minijinja::context! {
                trait_snake => &trait_snake,
                has_plugin_super => has_plugin_super,
            },
        ));
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
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_type_alias_factory_param.jinja",
                minijinja::context! {
                    param_name => param_name,
                    params_str => &params_str,
                    return_type => &ret,
                },
            ));
        }
        // VisitorHandle is `Arc<Mutex<dyn HtmlVisitor + Send>>`. Build the inner alias and
        // wrap it in the local opaque struct via its `From<core_type>` impl.
        let method_names: Vec<&str> = own_methods.iter().map(|method| method.name.as_str()).collect();
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_type_alias_factory_body.jinja",
            minijinja::context! {
                type_alias => type_alias,
                struct_name => &struct_name,
                has_plugin_super => has_plugin_super,
                method_names => method_names,
                inner_path => &inner_path,
            },
        ));

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
        if bridge_config.bind_via == BridgeBinding::OptionsField {
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
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_options_from_json_with_field.jinja",
                    minijinja::context! {
                        options_type => options_type,
                        type_alias => type_alias,
                        field => &field,
                        options_snake => &options_snake,
                        core_options_path => &core_options_path,
                        inner_path => &inner_path,
                    },
                ));
            }
        }
    } else {
        // Non-type-alias plugin factory: same `impl Fn` parameter shape as the
        // type-alias path so FRB can synthesise Dart-callable function types for
        // each closure parameter. Returns the `pub struct {Trait}DartImpl(Arc<dyn …>)`
        // wrapper emitted in section 3b.
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_plugin_factory_doc.rs.jinja",
            minijinja::context! {
                struct_name => struct_name.as_str(),
            },
        ));
        if has_plugin_super {
            out.push_str("/// `plugin_name` and `plugin_version` are required for the Plugin super-trait.\n");
        }
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_plugin_factory_open.rs.jinja",
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
            // Use the same substitution as the closure-field type
            // (`dart_fn_future_callback_type`) so the `impl Fn` param shape matches
            // the field's `Box<dyn Fn>` shape at the `Box::new(name)` init site —
            // including excluded-type carrier substitution applied to trait signatures.
            let callback_ty =
                dart_fn_future_factory_param_type(method, source_crate_name, type_paths, &api.excluded_type_paths);
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_factory_param.jinja",
                minijinja::context! {
                    param_name => param_name.as_str(),
                    callback_ty => callback_ty.as_str(),
                },
            ));
        }
        let plugin_fields = if has_plugin_super {
            "        plugin_name,\n        plugin_version,\n".to_string()
        } else {
            String::new()
        };
        let method_fields = own_methods
            .iter()
            .map(|method| format!("        {name}: Box::new({name}),\n", name = method.name))
            .collect::<String>();
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_plugin_factory_body.rs.jinja",
            minijinja::context! {
                struct_name => struct_name.as_str(),
                callbacks_struct_name => callbacks_struct_name.as_str(),
                plugin_fields => plugin_fields.as_str(),
                method_fields => method_fields.as_str(),
            },
        ));
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
