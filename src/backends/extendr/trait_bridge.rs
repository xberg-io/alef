//! R (extendr) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to R objects (named lists of functions) via extendr.

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
use crate::codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, format_type_ref,
    gen_bridge_all, visitor_param_type,
};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Extendr-specific trait bridge generator.
/// Implements code generation for bridging R objects to Rust traits.
pub struct ExtendrBridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    pub error_type: String,
    /// Callback-param type names that get NATIVE-object marshalling — known serde structs
    /// (per the shared [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`]
    /// rule) that are also registered as extendr classes (so the `#[extendr]`-generated
    /// `From<Binding> for Robj` exists). For such a param the bridge builds the binding's native
    /// R object (an `ExternalPtr` class env, via the same `From<core::T>` conversion used for
    /// return values) and hands THAT to the host closure, instead of serializing the param to a
    /// JSON string. Enums, opaque/handle types, extendr-incompatible structs, and excluded/unknown
    /// `Named` params are absent and keep their prior JSON-string representation.
    pub struct_param_types: std::collections::HashSet<String>,
}

impl ExtendrBridgeGenerator {
    /// True when a `Named(name)` callback param should be handed to the host as the binding's
    /// native R object rather than a JSON string — i.e. it is a known serde struct that is also
    /// registered as an extendr class. The native object is the `ExternalPtr` class env,
    /// constructed from the core value via the same `From<core::T>` conversion the binding uses
    /// for function return values.
    fn is_native_struct_param(&self, name: &str) -> bool {
        self.struct_param_types.contains(name)
    }
}

impl TraitBridgeGenerator for ExtendrBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "extendr_api::Robj"
    }

    fn bridge_imports(&self) -> Vec<String> {
        // Return bare import paths (no `use` keyword, no trailing `;`).
        // RustFileBuilder::add_import() wraps them as `use {path};`.
        vec!["extendr_api::prelude::*".to_string(), "std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();

        // Build argument list for the R function call
        let (empty_args, args_pairs) = if method.params.is_empty() {
            (true, String::new())
        } else {
            let args: Vec<String> = method
                .params
                .iter()
                .map(|p| match &p.ty {
                    // Known serde struct registered as an extendr class: hand the host the
                    // binding's native R object, built from the core value through the same
                    // Rust→R conversion used for return values (`Robj::from(Binding::from(core))`).
                    // The `#[extendr]`-generated `From<Binding> for Robj` wraps it as an
                    // `ExternalPtr` class env. No JSON round-trip.
                    TypeRef::Named(n) if self.is_native_struct_param(n) => {
                        let owned = if p.is_ref {
                            format!("(*{}).clone()", p.name)
                        } else {
                            format!("{}.clone()", p.name)
                        };
                        format!("extendr_api::Robj::from({n}::from({owned}))")
                    }
                    // Other params (enums, opaque/handle, excluded/unknown, primitives, strings)
                    // keep their prior representation.
                    _ => build_extendr_arg(p, spec.bridge_config.context_type.as_deref()),
                })
                .collect();
            let pairs: Vec<String> = method
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
                .collect();
            (false, pairs.join(", "))
        };

        // Determine which template to use based on return type
        let is_primitive_return = matches!(&method.return_type, TypeRef::Primitive(_));
        let template_name = match &method.return_type {
            TypeRef::Unit => "sync_method_unit_return.jinja",
            TypeRef::String | TypeRef::Char => "sync_method_string_return.jinja",
            _ => "sync_method_complex_return.jinja",
        };

        let ret_ty = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => format_type_ref(other, &self.type_paths),
        };

        crate::backends::extendr::template_env::render(
            template_name,
            minijinja::context! {
                method_name => name,
                has_error => has_error,
                has_error_check => if has_error { "true" } else { "false" },
                empty_args => empty_args,
                args_pairs => args_pairs,
                return_type => ret_ty,
                is_primitive_return => is_primitive_return,
                missing_method_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "self.cached_name",
                    "missing method",
                ),
                failed_method_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "self.cached_name",
                    "failed",
                ),
                invalid_type_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "self.cached_name",
                    "returned invalid type",
                ),
                deserialization_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "self.cached_name",
                    "deserialization failed",
                ),
                parse_error => make_error_expr(
                    &spec.error_constructor,
                    r#"format!("Failed to parse return value: {}", e)"#,
                ),
            },
        )
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        // Generate param cloning statements
        let mut params_to_clone = Vec::new();
        for p in &method.params {
            let template_name = match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => "async_param_clone_bytes_ref.jinja",
                (TypeRef::Path, true) => "async_param_clone_path_ref.jinja",
                // Native serde struct: clone the OWNED core value (`{name}_owned`, which is
                // `Send`) before the closure. The native R object cannot cross the
                // `spawn_blocking` thread boundary (it wraps a `!Send` `Robj`), so it is
                // constructed from the cloned core value INSIDE the closure instead.
                (TypeRef::Named(n), true) if self.is_native_struct_param(n) => {
                    "async_param_clone_native_struct_ref.jinja"
                }
                (TypeRef::Named(_), true) => "async_param_clone_named_ref.jinja",
                (_, true) => "async_param_clone_ref.jinja",
                _ => "async_param_clone_value.jinja",
            };
            let clone_stmt = crate::backends::extendr::template_env::render(
                template_name,
                minijinja::context! {
                    name => &p.name,
                },
            );
            params_to_clone.push(clone_stmt);
        }

        // Build argument list for the R function call
        let (empty_args, args_pairs) = if method.params.is_empty() {
            (true, String::new())
        } else {
            let args: Vec<String> = method
                .params
                .iter()
                .map(|p| match (&p.ty, p.is_ref) {
                    (TypeRef::Bytes, true) => format!("extendr_api::Robj::from(&{0}[..])", p.name),
                    (TypeRef::Path, true) => format!("extendr_api::Robj::from({0}_str.as_str())", p.name),
                    // Native serde struct: build the binding's native R object from the cloned
                    // owned core value INSIDE the closure, through the same `From<core::T>`
                    // conversion used for return values. The `#[extendr]`-generated
                    // `From<Binding> for Robj` wraps it as an `ExternalPtr`. No JSON round-trip.
                    (TypeRef::Named(n), true) if self.is_native_struct_param(n) => {
                        format!("extendr_api::Robj::from({n}::from({0}_owned.clone()))", p.name)
                    }
                    (TypeRef::Named(_), true) => format!("extendr_api::Robj::from({0}_json.as_str())", p.name),
                    _ => format!("extendr_api::Robj::from({})", p.name),
                })
                .collect();
            let pairs: Vec<String> = method
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
                .collect();
            (false, pairs.join(", "))
        };

        // Choose template based on return type
        let template_name = match &method.return_type {
            TypeRef::Unit => "async_method_unit_return.jinja",
            TypeRef::String | TypeRef::Char => "async_method_string_return.jinja",
            _ => "async_method_complex_return.jinja",
        };

        let ret_ty = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => format_type_ref(other, &self.type_paths),
        };

        crate::backends::extendr::template_env::render(
            template_name,
            minijinja::context! {
                method_name => name,
                params_to_clone => params_to_clone,
                empty_args => empty_args,
                args_pairs => args_pairs,
                return_type => ret_ty,
                missing_method_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "cached_name_inner",
                    "missing method",
                ),
                failed_method_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "cached_name_inner",
                    "failed",
                ),
                invalid_type_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "cached_name_inner",
                    "returned invalid type",
                ),
                deserialization_error => method_error_expr(
                    &spec.error_constructor,
                    name,
                    "cached_name_inner",
                    "deserialization failed",
                ),
                spawn_blocking_error => make_error_expr(
                    &spec.error_constructor,
                    r#"format!("spawn_blocking failed: {}", e)"#,
                ),
            },
        )
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods: Vec<String> = spec.required_methods().iter().map(|m| m.name.clone()).collect();

        crate::backends::extendr::template_env::render(
            "bridge_constructor.jinja",
            minijinja::context! {
                wrapper => wrapper,
                required_methods => required_methods,
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        crate::backends::extendr::template_env::render(
            "unregistration_fn.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
                host_path => host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        crate::backends::extendr::template_env::render(
            "clear_fn.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
                host_path => host_path,
            },
        )
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(register_fn) = spec.bridge_config.register_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let wrapper = spec.wrapper_name();
        let trait_path = spec.trait_path();

        let req_methods = spec.required_methods();
        let has_methods = !req_methods.is_empty();
        let required_methods_list = req_methods
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect::<Vec<_>>()
            .join(", ");

        crate::backends::extendr::template_env::render(
            "registration_fn.jinja",
            minijinja::context! {
                register_fn => register_fn,
                wrapper => wrapper,
                trait_path => trait_path,
                registry_getter => registry_getter,
                required_methods => has_methods,
                required_methods_list => required_methods_list,
                error_map => ".map_err(|e| extendr_api::Error::Other(e))?",
            },
        )
    }
}

fn make_error_expr(error_constructor: &str, message_expr: &str) -> String {
    error_constructor.replace("{msg}", message_expr)
}

fn method_error_expr(error_constructor: &str, method_name: &str, plugin_name_expr: &str, reason: &str) -> String {
    let message_expr = if reason == "missing method" {
        format!(r#"format!("Plugin '{{}}' missing method '{method_name}'", {plugin_name_expr})"#)
    } else {
        format!(r#"format!("Plugin '{{}}' method '{method_name}' {reason}", {plugin_name_expr})"#)
    };
    make_error_expr(error_constructor, &message_expr)
}

/// Compute the set of trait-callback struct params that extendr should marshal to the host as
/// the binding's native R object (an `ExternalPtr` class env) rather than a JSON string.
///
/// Starts from the shared, backend-agnostic allowlist
/// ([`crate::codegen::generators::trait_bridge::native_marshalled_struct_params`] — known serde
/// structs) and removes structs that extendr cannot register as a class because their own fields
/// contain types extendr cannot convert (`Vec<Named>` / `Option<Vec<_>>` / nested `Vec`). Those
/// have no `#[extendr]`-generated `From<Binding> for Robj`, so the native conversion would not
/// compile; they keep their prior JSON-string representation.
pub(crate) fn native_marshalled_extendr_struct_params(
    trait_type: &TypeDef,
    api: &ApiSurface,
) -> std::collections::HashSet<String> {
    let mut out = crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
    out.retain(|name| {
        api.types
            .iter()
            .find(|t| &t.name == name)
            .is_some_and(|t| !t.fields.iter().any(|f| field_is_extendr_incompatible(&f.ty)))
    });
    out
}

/// True if a field type prevents extendr from registering the containing struct as a class —
/// `Vec<Named>`, `Option<Vec<_>>`, or nested `Vec<Vec<_>>`. Mirrors the
/// `is_extendr_native_incompatible` check in `gen_bindings` (kept in sync; extendr cannot
/// auto-convert these from/to `Robj`).
fn field_is_extendr_incompatible(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(_) | TypeRef::Vec(_)),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Vec(inner2) if matches!(inner2.as_ref(), TypeRef::Named(_) | TypeRef::Vec(_)))
        }
        _ => false,
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &crate::core::ir::ApiSurface,
) -> anyhow::Result<BridgeOutput> {
    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("R", bridge_cfg);
    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup (owned HashMap for use with new generator)
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (for example, `&HiddenDoc`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        gen_visitor_bridge(
            &mut out,
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
            api,
        )?;
        Ok(BridgeOutput {
            imports: vec![],
            code: out,
        })
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure.
        //
        // Classify which callback params get native-object marshalling using the SHARED rule
        // (`native_marshalled_struct_params`) so the allowlist is identical to what other backends
        // consult, then narrow it to structs extendr can actually represent as a native R object:
        // extendr-incompatible structs (those with `Vec<Named>` / `Option<Vec<_>>` fields) are NOT
        // registered as extendr classes and have no `#[extendr]`-generated `From<Binding> for Robj`,
        // so they must keep the JSON-string representation. For the remaining params the bridge hands
        // the host the binding's native R object (an `ExternalPtr` class env) built via the same
        // `From<core::T>` conversion used for return values.
        let struct_param_types = native_marshalled_extendr_struct_params(trait_type, api);
        let generator = ExtendrBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
        };
        let lifetime_type_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|typ| typ.has_lifetime_params)
            .map(|typ| typ.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "R",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let mut output = gen_bridge_all(&spec, &generator);
        // SAFETY: `extendr_api::Robj` wraps a `*mut SEXPREC` and is therefore neither `Send`
        // nor `Sync` by default. The Rust `Plugin` super-trait requires `Send + Sync + 'static`,
        // and `async_trait`-generated futures must be `Send`. R itself is single-threaded — the
        // user must guarantee that the bridge is only invoked from the R main thread. We assert
        // `Send + Sync` so the trait bounds are satisfied; callers misusing this from background
        // threads will encounter R-runtime undefined behaviour. This contract is consistent with
        // how PyO3 handles `Py<PyAny>` under the GIL.
        let send_sync_impl = format!(
            "\n#[allow(clippy::non_send_fields_in_send_ty)]\n\
             // SAFETY: R is single-threaded; the user must invoke plugins from the R main thread.\n\
             unsafe impl Send for {struct_name} {{}}\n\
             // SAFETY: see Send impl.\n\
             unsafe impl Sync for {struct_name} {{}}\n"
        );
        output.code.push_str(&send_sync_impl);
        Ok(output)
    }
}

/// Generate the shared `SendRobj` wrapper module that allows `Robj` to cross thread boundaries
/// in `spawn_blocking` closures. Emitted once per generated file when any trait bridge is
/// produced, before any bridge struct. The wrapper is required because `extendr_api::Robj`
/// contains a raw pointer and is therefore `!Send`/`!Sync`.
pub fn gen_send_robj_helper() -> &'static str {
    "/// Newtype wrapper around `extendr_api::Robj` that asserts `Send + Sync`.\n\
     ///\n\
     /// # Safety\n\
     ///\n\
     /// R is single-threaded; user-supplied R callbacks must only be invoked from the R main\n\
     /// thread. This wrapper exists to satisfy the `Send`/`Sync` bounds required by the Rust\n\
     /// plugin trait system and by `tokio::spawn_blocking`. Misuse from a background thread\n\
     /// triggers R-runtime undefined behaviour.\n\
     #[repr(transparent)]\n\
     #[derive(Clone)]\n\
     pub(crate) struct SendRobj(pub extendr_api::Robj);\n\
     // SAFETY: see SendRobj docs.\n\
     unsafe impl Send for SendRobj {}\n\
     // SAFETY: see SendRobj docs.\n\
     unsafe impl Sync for SendRobj {}\n\
     impl SendRobj {\n\
         /// Consume the wrapper and yield the inner `Robj`. Used inside `spawn_blocking`\n\
         /// closures so that the closure captures the whole `SendRobj` (which is `Send`)\n\
         /// rather than the inner `Robj` field (which is `!Send`) under 2021+ disjoint\n\
         /// capture rules.\n\
         #[inline]\n\
         pub(crate) fn into_inner(self) -> extendr_api::Robj { self.0 }\n\
     }\n"
}

/// Generate a visitor-style bridge wrapping an `extendr_api::Robj` (a named list of functions).
///
/// Every trait method checks if the list has a function with the snake_case method name,
/// calls it via extendr's `.call()`, and maps the return value to the configured result enum.
#[allow(clippy::too_many_arguments)]
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<()> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Extendr,
    )?;
    let context_type = bridge_cfg.context_type.as_deref();
    let mut method_impls = String::with_capacity(4096);
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method_extendr(&mut method_impls, method, context_type, type_paths, &result_metadata);
    }

    out.push_str(&crate::backends::extendr::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            core_crate => core_crate,
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
            struct_name => struct_name,
            trait_path => trait_path,
            method_impls => method_impls,
        },
    ));
    Ok(())
}

/// Generate a single visitor method that checks if the R list has an element with this name
/// and calls it as a function.
fn gen_visitor_method_extendr(
    out: &mut String,
    method: &MethodDef,
    context_type: Option<&str>,
    type_paths: &std::collections::HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let empty_args = method.params.is_empty();
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| build_extendr_arg(p, context_type))
        .collect();
    let args_pairs: Vec<String> = method
        .params
        .iter()
        .zip(args.iter())
        .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
        .collect();
    let args_pairs = args_pairs.join(", ");

    out.push_str(&crate::backends::extendr::template_env::render(
        "visitor_method.jinja",
        minijinja::context! {
            method_name => name,
            signature => signature,
            return_type => return_type,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&return_type, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &return_type,
                result_metadata,
                "s.to_string()",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            empty_args => empty_args,
            args_pairs => args_pairs,
        },
    ));
}

/// Build a single extendr `Pairlist` arg expression for a visitor method parameter.
fn build_extendr_arg(p: &crate::core::ir::ParamDef, context_type: Option<&str>) -> String {
    use crate::core::ir::TypeRef;

    // context_type param: convert to an R list via nodecontext_to_robj
    if let TypeRef::Named(n) = &p.ty {
        if Some(n.as_str()) == context_type {
            let ref_prefix = if p.is_ref { "" } else { "&" };
            return format!("extendr_api::Robj::from(nodecontext_to_robj({}{}))", ref_prefix, p.name);
        }
    }

    // Option<&str>: IR collapses to String + optional + is_ref
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {name} {{ Some(s) => extendr_api::Robj::from(s), None => extendr_api::Robj::from(extendr_api::NULL) }}",
            name = p.name
        );
    }

    // &[u8] / Vec<u8>: pass as a raw vector reference
    if matches!(&p.ty, TypeRef::Bytes) {
        if p.is_ref {
            return format!("extendr_api::Robj::from(&{}[..])", p.name);
        }
        return format!("extendr_api::Robj::from(&{}[..])", p.name);
    }

    // &str: wrap in Robj
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("extendr_api::Robj::from({})", p.name);
    }

    // Owned String
    if matches!(&p.ty, TypeRef::String) {
        return format!("extendr_api::Robj::from({}.as_str())", p.name);
    }

    // Named types (structs/enums): serialize to JSON. `extendr_api::Robj` does not
    // implement `From` for arbitrary user types, so we pass them across the R boundary
    // as JSON strings. The R callback is responsible for deserializing on its side.
    if let TypeRef::Named(_) = &p.ty {
        let serde_target = if p.is_ref {
            p.name.clone()
        } else {
            format!("&{}", p.name)
        };
        return format!(
            "extendr_api::Robj::from(serde_json::to_string({}).unwrap_or_default().as_str())",
            serde_target
        );
    }

    // bool
    if matches!(&p.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
        return format!("extendr_api::Robj::from({})", p.name);
    }

    // Integer-like primitives: cast to i32 (R INTEGER)
    if let TypeRef::Primitive(prim) = &p.ty {
        use crate::core::ir::PrimitiveType;
        match prim {
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => {
                return format!("extendr_api::Robj::from({} as i32)", p.name);
            }
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                return format!("extendr_api::Robj::from({} as f64)", p.name);
            }
            PrimitiveType::F32 | PrimitiveType::F64 => {
                return format!("extendr_api::Robj::from({} as f64)", p.name);
            }
            PrimitiveType::Bool => {
                return format!("extendr_api::Robj::from({})", p.name);
            }
        }
    }

    // Fallback
    format!("extendr_api::Robj::from({})", p.name)
}

/// Generate an extendr free function that has one parameter replaced by `Option<extendr_api::Robj>`
/// (a trait bridge). The bridge is constructed before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("R", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list — replace the bridge param with Option<extendr_api::Robj>
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            // The visitor is always optional from R's perspective (NULL means "no visitor")
            sig_parts.push(format!("{}: Option<extendr_api::Robj>", p.name));
        } else {
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            let ty = if p.optional || promoted {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let has_error = func.error_type.is_some();
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| extendr_api::Error::Other(e.to_string()))";

    // Bridge wrapping: Option<Robj> -> Option<configured handle>
    // We always treat it as optional since R passes NULL for missing visitors.
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_null() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        // Non-optional in IR, but we expose it as Option<Robj> regardless and
        // unwrap or construct a bridge from a non-null Robj.
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_null() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    };

    // Serde let-bindings for non-bridge Named params
    let serde_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == bridge_param_idx {
                return false;
            }
            let named = match &p.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            named.is_some_and(|n| !opaque_types.contains(n))
        })
        .map(|(_, p)| {
            let name = &p.name;
            let core_path = format!(
                "{core_import}::{}",
                match &p.ty {
                    TypeRef::Named(n) => n.clone(),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            n.clone()
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                }
            );
            let template_name = if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                "serde_named_optional_binding.jinja"
            } else {
                "serde_named_required_binding.jinja"
            };
            crate::backends::extendr::template_env::render(
                template_name,
                minijinja::context! {
                    name => name,
                    core_path => core_path,
                    err_conv => err_conv,
                },
            )
        })
        .collect();

    // Build call args for the core function
    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == bridge_param_idx {
                return p.name.clone();
            }
            match &p.ty {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => format!("{}_core", p.name),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if opaque_types.contains(n.as_str()) {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect();
    let call_args_str = call_args.join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args_str})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) | TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    let func_name = &func.name;
    crate::backends::extendr::template_env::render(
        "bridge_function.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            func_name => func_name,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let output = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
        assert!(output.code.contains("\"display_name\""));
    }

    // -----------------------------------------------------------------------
    // Native-object marshalling of struct callback params (neutral fixtures).
    //
    // A trait-callback param that is a known serde struct registered as an extendr class must be
    // handed to the host as the binding's NATIVE R object — built via the same `From<core::T>`
    // conversion the binding uses for return values, then wrapped as an `Robj` ExternalPtr — NOT
    // serialized to a JSON string. Enum / opaque / unknown / extendr-incompatible params keep
    // their prior JSON-string representation. The positive allowlist comes from the SHARED
    // classifier (`native_marshalled_struct_params`), narrowed to extendr-representable structs.
    // -----------------------------------------------------------------------

    use crate::backends::extendr::trait_bridge::{ExtendrBridgeGenerator, native_marshalled_extendr_struct_params};
    use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
    use std::collections::{HashMap, HashSet};

    fn struct_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample_core::{name}"),
            fields,
            has_serde: true,
            ..Default::default()
        }
    }

    fn named_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..Default::default()
        }
    }

    fn greeter_trait_with(methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: "Greeter".to_string(),
            rust_path: "sample_core::Greeter".to_string(),
            is_trait: true,
            methods,
            ..Default::default()
        }
    }

    fn ref_named_param(name: &str, ty_name: &str) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::Named(ty_name.to_string()),
            is_ref: true,
            ..Default::default()
        }
    }

    fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async,
            ..Default::default()
        }
    }

    fn generator_with(struct_params: &[&str]) -> ExtendrBridgeGenerator {
        ExtendrBridgeGenerator {
            core_import: "sample_core".to_string(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_string(),
            struct_param_types: struct_params.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn plugin_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "R",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_string(),
            error_constructor: "SampleError::Message { message: {msg} }".to_string(),
        }
    }

    #[test]
    fn allowlist_includes_serde_struct_excludes_enum_opaque_and_incompatible() {
        // Opts is a plain serde struct param (qualifies). Bag is a serde struct param with a
        // Vec<Named> field — extendr cannot register it as a class, so it is excluded. Mood is an
        // enum (lives in api.enums, never api.types) and Widget is an unknown Named — both absent.
        let mut api = ApiSurface::default();
        api.types
            .push(struct_typedef("Opts", vec![named_field("greeting", TypeRef::String)]));
        api.types.push(struct_typedef(
            "Bag",
            vec![named_field(
                "items",
                TypeRef::Vec(Box::new(TypeRef::Named("Opts".to_string()))),
            )],
        ));

        let trait_def = greeter_trait_with(vec![
            method(
                "greet",
                vec![ref_named_param("opts", "Opts"), ref_named_param("bag", "Bag")],
                TypeRef::Named("Doc".to_string()),
                false,
            ),
            method(
                "decorate",
                vec![ref_named_param("mood", "Mood"), ref_named_param("widget", "Widget")],
                TypeRef::Unit,
                false,
            ),
        ]);

        let allow = native_marshalled_extendr_struct_params(&trait_def, &api);
        assert!(allow.contains("Opts"), "serde struct param must qualify: {allow:?}");
        assert!(
            !allow.contains("Bag"),
            "extendr-incompatible struct must be excluded: {allow:?}"
        );
        assert!(!allow.contains("Mood"), "enum must be excluded: {allow:?}");
        assert!(!allow.contains("Widget"), "unknown type must be excluded: {allow:?}");
    }

    #[test]
    fn sync_struct_param_marshalled_as_native_r_object_not_json_string() {
        let generator = generator_with(&["Opts"]);
        let trait_def = greeter_trait_with(vec![]);
        let bridge_cfg = TraitBridgeConfig::default();
        let spec = plugin_spec(&trait_def, &bridge_cfg);

        let m = method(
            "greet",
            vec![ref_named_param("opts", "Opts")],
            TypeRef::Named("Doc".to_string()),
            false,
        );
        let body = generator.gen_sync_method_body(&m, &spec);

        assert!(
            body.contains("extendr_api::Robj::from(Opts::from((*opts).clone()))"),
            "struct param must be built as the binding's native R object via From<core>:\n{body}"
        );
        assert!(
            !body.contains("serde_json::to_string(opts)"),
            "struct param must NOT be serialized to a JSON string:\n{body}"
        );
    }

    #[test]
    fn async_struct_param_marshalled_as_native_r_object_not_json_string() {
        let generator = generator_with(&["Opts"]);
        let trait_def = greeter_trait_with(vec![]);
        let bridge_cfg = TraitBridgeConfig::default();
        let spec = plugin_spec(&trait_def, &bridge_cfg);

        let m = method(
            "greet",
            vec![ref_named_param("opts", "Opts")],
            TypeRef::Named("Doc".to_string()),
            true,
        );
        let body = generator.gen_async_method_body(&m, &spec);

        // The preamble clones the OWNED core value (Send) before the spawn_blocking closure; the
        // native R object is constructed from it INSIDE the closure (R objects are !Send).
        assert!(
            body.contains("let opts_owned = (*opts).clone();"),
            "async preamble must clone the owned core struct value:\n{body}"
        );
        assert!(
            body.contains("extendr_api::Robj::from(Opts::from(opts_owned.clone()))"),
            "async struct param must be built as the binding's native R object:\n{body}"
        );
        assert!(
            !body.contains("opts_json"),
            "async struct param must NOT be serialized to a JSON string:\n{body}"
        );
    }

    #[test]
    fn enum_and_unknown_named_params_keep_json_string_representation() {
        // Only Opts is on the allowlist; Mood (enum) and Widget (unknown) are not.
        let generator = generator_with(&["Opts"]);
        let trait_def = greeter_trait_with(vec![]);
        let bridge_cfg = TraitBridgeConfig::default();
        let spec = plugin_spec(&trait_def, &bridge_cfg);

        let m = method(
            "decorate",
            vec![ref_named_param("mood", "Mood"), ref_named_param("widget", "Widget")],
            TypeRef::Unit,
            false,
        );
        let body = generator.gen_sync_method_body(&m, &spec);
        assert!(
            body.contains("serde_json::to_string(mood)") && body.contains("serde_json::to_string(widget)"),
            "non-struct Named params must keep the JSON-string representation:\n{body}"
        );
        assert!(
            !body.contains("Mood::from(") && !body.contains("Widget::from("),
            "non-struct Named params must NOT be built as native objects:\n{body}"
        );
    }
}
