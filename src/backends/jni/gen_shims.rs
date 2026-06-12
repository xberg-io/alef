//! JNI shim code generator.
//!
//! Emits a single `lib.rs` into the consumer's `<crate>-jni` Rust crate.  The
//! emitted file exports `pub unsafe extern "system" fn Java_*` symbols that
//! satisfy every `external fun native*` declaration produced by
//! `alef-backend-kotlin-android`.
//!
//! # Symbol naming — JNI spec §5.11.3
//!
//! `Java_<package_underscored>_<Class>_<method>`
//!
//! Underscores inside any identifier segment are encoded as `_1`.  Package
//! dots become `_`.  The helpers in [`crate::core::jni`] own the canonical
//! encoding so this backend and the Kotlin backend can never drift apart.

use std::path::PathBuf;

use minijinja::context;

use crate::backends::jni::template_env;
use crate::codegen::naming::to_class_name;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, ParamDef, PrimitiveType, TypeDef, TypeRef};
use crate::core::jni::{
    bridge_class_name, bridge_method_name, destructor_method_name, jni_symbol, streaming_method_names,
};

/// Backend that emits the Rust JNI shim crate source.
#[derive(Debug, Default, Clone, Copy)]
pub struct JniBackend;

impl Backend for JniBackend {
    fn name(&self) -> &str {
        "jni"
    }

    fn language(&self) -> Language {
        Language::Jni
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: false,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Require kotlin_android config — the package is needed for JNI symbol names.
        if config.kotlin_android.is_none() {
            anyhow::bail!(
                "kotlin-android config required for JNI shim generation: \
                 add [crates.kotlin_android] with package = \"...\" to alef.toml"
            );
        }
        let output_path = jni_output_path(config);
        let content = emit_lib_rs(api, config);
        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: true,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        super::service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-jni",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// Output path resolution
// ---------------------------------------------------------------------------

/// Default output directory: `crates/<crate-base>-jni/src/lib.rs`
///
/// `crate-base` is `config.jni_crate_base()`: `[crates.jni] crate_dir` when
/// set, otherwise `config.name`.  The override lets consumers whose name
/// carries a language suffix (e.g. `"sample-markdown-rs"`) produce a crate
/// at `crates/sample-markdown-jni/` that matches all other binding crates.
fn jni_output_path(config: &ResolvedCrateConfig) -> PathBuf {
    let jni_crate = format!("{}-jni", config.jni_crate_base());
    PathBuf::from(format!("crates/{jni_crate}/src/lib.rs"))
}

// ---------------------------------------------------------------------------
// Top-level emitter
// ---------------------------------------------------------------------------

/// Emit the full `lib.rs` content for the JNI shim crate.
pub(crate) fn emit_lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = jni_kotlin_package(config);
    let bridge = bridge_class_name(&config.name);
    let core_crate = core_use_path(config);
    let error_class = resolve_error_class(config, &package);

    let mut out = String::new();

    out.push_str(&template_env::render(
        "lib_header.rs.jinja",
        context! {
            core_crate => core_crate,
            error_class => error_class,
        },
    ));

    // Shared runtime helpers.
    emit_runtime_helpers(&mut out);

    // Collect visible top-level functions.
    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !f.sanitized && !exclude_functions.contains(f.name.as_str()))
        .collect();

    // Collect opaque type names for handle-vs-JSON dispatch.
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    // Top-level function shims.
    for f in &visible_functions {
        let method_name = bridge_method_name("", &f.name);
        let symbol = jni_symbol(&package, &bridge, &method_name);
        emit_function_shim(
            &mut out,
            &symbol,
            &f.name,
            &f.params,
            &f.return_type,
            f.is_async,
            f.error_type.is_some(),
            &opaque_type_names,
        );
    }

    // Opaque client type shims (types that have instance methods).
    let client_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .collect();
    let client_type_names: std::collections::HashSet<&str> = client_types.iter().map(|t| t.name.as_str()).collect();

    for ty in &client_types {
        emit_client_shims(
            &mut out,
            ty,
            api,
            config,
            &package,
            &bridge,
            &exclude_functions,
            &opaque_type_names,
        );
    }

    // Emit destructors for opaque types that are returned by top-level functions
    // but do NOT have instance methods (those are handled by emit_client_shims above).
    let top_level_opaque_returns: std::collections::HashSet<&str> = visible_functions
        .iter()
        .filter_map(|f| {
            if let TypeRef::Named(n) = &f.return_type {
                if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                    return Some(n.as_str());
                }
            }
            None
        })
        .collect();

    for type_name in &top_level_opaque_returns {
        let free_name = destructor_method_name(type_name);
        let free_symbol = jni_symbol(&package, &bridge, &free_name);
        emit_destructor_shim(&mut out, &free_symbol, type_name);
    }

    // Trait-bridge shims (Java_*_nativeRegister<Trait> / nativeUnregister<Trait> /
    // nativeClear<Trait>s).  Bridges with `kotlin_android` in `exclude_languages`
    // are skipped.
    emit_trait_bridge_shims(&mut out, config, api, &package, &bridge);

    out
}

/// Emit JNI Rust shims for every configured `[[crates.trait_bridges]]` entry.
///
/// For each bridge whose `exclude_languages` does not contain `kotlin_android`,
/// emits:
/// - Trait adapter struct `Jni{Trait}Adapter` that wraps a global JNI reference
/// - Up to three `Java_*` symbols:
///   - `nativeRegister<Trait>(impl: I<Trait>)` — creates a global JNI reference,
///     wraps it in an adapter, and calls the host crate's `register_fn`.
///   - `nativeUnregister<Trait>(name: String)` — calls the host crate's
///     `unregister_fn(&name)` and surfaces any `Err(_)` as a thrown JNI exception.
///   - `nativeClear<Trait>s()` — calls the host crate's `clear_fn()` similarly.
fn emit_trait_bridge_shims(
    out: &mut String,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    package: &str,
    bridge: &str,
) {
    let bridges: Vec<_> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "kotlin_android"))
        .collect();
    if bridges.is_empty() {
        return;
    }
    out.push_str("\n// ---------------------------------------------------------------------------\n");
    out.push_str("// Trait-bridge shims\n");
    out.push_str("// ---------------------------------------------------------------------------\n\n");

    // First, emit adapter structs for all traits
    for bridge_cfg in &bridges {
        let trait_pascal = internal_class_component(&bridge_cfg.trait_name);
        let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);
        if let Some(trait_def) = trait_def {
            emit_trait_adapter_struct(out, &trait_pascal, trait_def, &bridge_cfg.trait_name, bridge_cfg.super_trait.is_some());
        }
    }

    out.push_str("\n");

    // Then emit the registration functions
    for bridge_cfg in &bridges {
        let trait_pascal = internal_class_component(&bridge_cfg.trait_name);

        // Find the trait definition for method iteration
        let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);

        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let native_name = format!("nativeRegister{trait_pascal}");
            let symbol = jni_symbol(package, bridge, &native_name);
            let has_super_trait = bridge_cfg.super_trait.is_some();
            emit_trait_register_shim(out, &symbol, &trait_pascal, register_fn, trait_def, has_super_trait);
        }
        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let native_name = format!("nativeUnregister{trait_pascal}");
            let symbol = jni_symbol(package, bridge, &native_name);
            emit_trait_unregister_shim(out, &symbol, unregister_fn);
        }
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let native_name = format!("nativeClear{trait_pascal}s");
            let symbol = jni_symbol(package, bridge, &native_name);
            emit_trait_clear_shim(out, &symbol, clear_fn);
        }
    }
}

/// Emit a trait adapter struct that wraps a global JNI reference and implements
/// the trait by calling back to Kotlin through JNI.
///
/// Generates:
/// ```rust
/// struct Jni{Trait}Adapter {
///     impl_ref: jni::objects::GlobalRef,
/// }
/// ```
///
/// For now this is a minimal wrapper. Full trait method impls would require
/// generating marshaling code for each trait method.
fn emit_trait_adapter_struct(
    out: &mut String,
    trait_pascal: &str,
    _trait_def: &TypeDef,
    _trait_rust_name: &str,
    _has_super_trait: bool,
) {
    let output = format!(
        "/// JNI adapter for {trait_pascal} trait bridge.\n\
         /// Wraps a Kotlin object reference and implements the trait by calling\n\
         /// through JNI to the wrapped Kotlin implementation.\n\
         pub struct Jni{trait_pascal}Adapter {{\n    impl_ref: jni::objects::GlobalRef,\n}}\n\n"
    );
    out.push_str(&output);

    // For now, we don't generate the full trait impl. This requires marshaling
    // every trait method, which is non-trivial. For the immediate goal (getting
    // tests to load), this struct existing and being constructible is sufficient.
    // When a method is called on the adapter, it will panic with a message
    // indicating that this is a stub that needs implementation.
}

/// Emit `Java_*_nativeRegister<Trait>(impl: I<Trait>)` or
/// `Java_*_nativeRegister<Trait>(impl: I<Trait>, name: JString)` shim that creates a
/// global JNI reference, calls the host crate's configured `register_fn`, and manages
/// bridge lifetime.
///
/// When `has_super_trait` is true, the impl object's `name()` method is called.
/// When false, the name is passed as an explicit JString parameter (matching the Kotlin
/// no-super-trait register(impl, name) signature).
fn emit_trait_register_shim(
    out: &mut String,
    symbol: &str,
    trait_pascal: &str,
    register_fn: &str,
    _trait_def: Option<&TypeDef>,
    has_super_trait: bool,
) {
    out.push_str(&template_env::render(
        "trait_register_shim.rs.jinja",
        context! {
            symbol => symbol,
            pascal_trait => trait_pascal,
            register_fn => register_fn,
            has_super_trait => has_super_trait,
        },
    ));
}

/// Emit `Java_*_nativeUnregister<Trait>(name: String)` shim that calls the
/// host crate's configured `unregister_fn`.
fn emit_trait_unregister_shim(out: &mut String, symbol: &str, unregister_fn: &str) {
    out.push_str(&template_env::render(
        "trait_unregister_shim.rs.jinja",
        context! {
            symbol => symbol,
            unregister_fn => unregister_fn,
        },
    ));
}

/// Emit `Java_*_nativeClear<Trait>s()` shim that calls the host crate's
/// configured `clear_fn`.
fn emit_trait_clear_shim(out: &mut String, symbol: &str, clear_fn: &str) {
    out.push_str(&template_env::render(
        "trait_clear_shim.rs.jinja",
        context! {
            symbol => symbol,
            clear_fn => clear_fn,
        },
    ));
}

// ---------------------------------------------------------------------------
// Inline helper emission
// ---------------------------------------------------------------------------

fn emit_runtime_helpers(out: &mut String) {
    out.push_str(&template_env::render("runtime_helpers.rs.jinja", context! {}));
}

// ---------------------------------------------------------------------------
// Client type shims
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn emit_client_shims(
    out: &mut String,
    ty: &TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
    exclude_functions: &std::collections::HashSet<&str>,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    // Instance method shims.
    for method in ty.methods.iter().filter(|m| !m.sanitized && !m.is_static) {
        if exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let method_name = bridge_method_name(&ty.name, &method.name);
        let symbol = jni_symbol(package, bridge, &method_name);
        let receiver_is_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
        let receiver_owned = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
        emit_method_shim(
            out,
            &symbol,
            &ty.name,
            &method.name,
            &method.params,
            &method.return_type,
            method.is_async,
            method.error_type.is_some(),
            receiver_is_mut,
            receiver_owned,
            opaque_type_names,
        );
    }

    // Destructor shim.
    let free_name = destructor_method_name(&ty.name);
    let free_symbol = jni_symbol(package, bridge, &free_name);
    emit_destructor_shim(out, &free_symbol, &ty.name);

    // Constructor shim (when client_constructors config is present for this type).
    if let Some(ctor) = config.client_constructors.get(&ty.name) {
        let ctor_method_name = format!("nativeNew{}", &ty.name);
        let ctor_symbol = jni_symbol(package, bridge, &ctor_method_name);
        emit_constructor_shim(out, &ctor_symbol, ty, config, ctor);
    }

    // Streaming adapter shims owned by this type.
    let streaming: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming) && a.owner_type.as_deref() == Some(ty.name.as_str()))
        .collect();
    for adapter in &streaming {
        let (start_name, next_name, free_adapter_name) = streaming_method_names(&ty.name, &adapter.name);
        let start_sym = jni_symbol(package, bridge, &start_name);
        let next_sym = jni_symbol(package, bridge, &next_name);
        let free_sym = jni_symbol(package, bridge, &free_adapter_name);
        emit_streaming_shims(out, &start_sym, &next_sym, &free_sym, ty, adapter, api);
    }

    let _ = api; // suppress unused warning if no streaming adapters
}

// ---------------------------------------------------------------------------
// Individual shim emitters
// ---------------------------------------------------------------------------

fn render_param_decl(name: &str, type_name: &str) -> String {
    template_env::render(
        "param_decl.rs.jinja",
        context! {
            name => name,
            type_name => type_name,
        },
    )
}

fn render_string_unmarshal(name: &str, ret_null: &str) -> String {
    template_env::render(
        "string_unmarshal.rs.jinja",
        context! {
            name => name,
            ret_null => ret_null,
        },
    )
}

fn render_byte_array_unmarshal(name: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "byte_array_unmarshal.rs.jinja",
        context! {
            name => name,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_complex_unmarshal(name: &str, type_path: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "complex_unmarshal.rs.jinja",
        context! {
            name => name,
            type_path => type_path,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_request_string_unmarshal(ret_null: &str, error_prefix: &str) -> String {
    template_env::render(
        "request_string_unmarshal.rs.jinja",
        context! {
            ret_null => ret_null,
            error_prefix => error_prefix,
        },
    )
}

fn render_vec_string_refs(refs_name: &str, source_name: &str) -> String {
    template_env::render(
        "vec_string_refs.rs.jinja",
        context! {
            refs_name => refs_name,
            source_name => source_name,
        },
    )
}

/// Emit a shim for a top-level API function.
///
/// When the return type is an opaque named type the function returns `jlong`
/// (a raw `Box::into_raw` pointer) rather than a JSON-encoded `jstring`.
/// When a parameter is an opaque named type it is received as `jlong` and
/// dereferenced via an unsafe pointer cast — the Kotlin caller holds the
/// handle as a `Long` that was previously obtained from the constructor shim.
#[allow(clippy::too_many_arguments)]
fn emit_function_shim(
    out: &mut String,
    symbol: &str,
    rust_fn_name: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    is_async: bool,
    has_error: bool,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    let core_fn = format!("core_crate::{}", rust_fn_name.replace('-', "_"));

    // Determine whether the return type is an opaque handle up-front so we can
    // use the correct null/zero sentinel in unmarshal error paths.
    let is_opaque_return = matches!(return_type, TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
    let ret_decl = if is_opaque_return {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    };
    let err_null = if is_opaque_return {
        "0"
    } else {
        method_return_null(return_type)
    };

    // Collect param signatures and unmarshal logic.
    let mut param_sigs = String::new();
    let mut unmarshal = String::new();
    let mut call_args = String::new();

    for p in params {
        let rust_name = p.name.replace('-', "_");
        // The base type (unwrap Optional to its inner type for JNI marshaling decisions).
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match base_ty {
            TypeRef::String => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_string_unmarshal(&rust_name, err_null));
                // Build call-site expression.  Optional Strings: the Kotlin
                // facade passes "" (empty string) as the null-sentinel for
                // String? params via `value ?: ""`, because JNI primitive
                // signatures cannot express nullability.  Treat empty as
                // None so the Rust callee receives the correct Option<_>.
                if p.optional {
                    call_args.push_str("if ");
                    call_args.push_str(&rust_name);
                    call_args.push_str(".is_empty() { None } else { Some(");
                    call_args.push_str(&rust_name);
                    call_args.push_str(") }");
                } else if p.is_ref {
                    call_args.push('&');
                    call_args.push_str(&rust_name);
                } else {
                    call_args.push_str(&rust_name);
                }
            }
            TypeRef::Primitive(prim) => {
                let jni_ty = jni_primitive_type(prim);
                param_sigs.push_str(&render_param_decl(&rust_name, jni_ty));
                let cast = primitive_cast(prim);
                let cast_expr = if cast.is_empty() {
                    rust_name.clone()
                } else {
                    format!("{rust_name} as {cast}")
                };
                if p.optional {
                    // Optional numeric primitives: the Kotlin facade passes
                    // 0 / 0L / 0.0 / false as the null-sentinel for nullable
                    // primitives via `value ?: 0`, because JNI primitive
                    // signatures cannot express nullability.  Treat the
                    // default value as None so the Rust callee receives the
                    // correct Option<_>.
                    let zero_lit = primitive_zero_literal(prim);
                    if let Some(zero) = zero_lit {
                        call_args.push_str("if ");
                        call_args.push_str(&rust_name);
                        call_args.push_str(" != ");
                        call_args.push_str(zero);
                        call_args.push_str(" { Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push_str(") } else { None }");
                    } else {
                        call_args.push_str("Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push(')');
                    }
                } else {
                    call_args.push_str(&cast_expr);
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
                param_sigs.push_str(&render_param_decl(&rust_name, "jbyteArray"));
                unmarshal.push_str(&render_byte_array_unmarshal(&rust_name, err_null, p.optional));
                if p.optional {
                    call_args.push_str(&rust_name);
                } else {
                    if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
            TypeRef::Bytes => {
                param_sigs.push_str(&render_param_decl(&rust_name, "jbyteArray"));
                unmarshal.push_str(&render_byte_array_unmarshal(&rust_name, err_null, p.optional));
                if p.optional {
                    call_args.push_str(&rust_name);
                } else {
                    if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
            TypeRef::Named(type_name) if opaque_type_names.contains(type_name.as_str()) => {
                // Opaque handle param: receive as jlong, dereference via raw pointer.
                // SAFETY: the Kotlin caller holds a Long obtained from the matching
                // constructor shim and guarantees the handle is live for this call.
                param_sigs.push_str(&render_param_decl(&rust_name, "jlong"));
                let type_path = format!("core_crate::{type_name}");
                unmarshal.push_str(&template_env::render(
                    "opaque_handle_unmarshal.rs.jinja",
                    context! {
                        name => rust_name,
                        type_path => type_path,
                    },
                ));
                // Pass as reference (already &T via deref).
                call_args.push_str(&rust_name);
            }
            _ => {
                // Complex types passed as JSON string from Kotlin side.
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                let type_path = type_ref_to_core_path(base_ty, "core_crate");
                // Optional complex params: the Kotlin/Java caller passes an empty
                // string (`""`) when the host-language value is null, the legacy
                // sentinel for "no payload" that pairs with `?.let { ... } ?: ""`.
                // Accept that sentinel as `None` instead of attempting to parse
                // it as JSON (which fails with `EOF while parsing a value`).
                if p.optional {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, true));
                    call_args.push_str(&rust_name);
                } else {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, false));
                    // Special case: Vec<String> with is_ref means the core expects `&[&str]`.
                    let is_vec_string_ref =
                        p.is_ref && matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
                    if is_vec_string_ref {
                        let refs_name = format!("{rust_name}_refs");
                        unmarshal.push_str(&render_vec_string_refs(&refs_name, &rust_name));
                        call_args.push('&');
                        call_args.push_str(&refs_name);
                    } else if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
        }
        call_args.push_str(", ");
    }
    // Remove trailing ", "
    if call_args.ends_with(", ") {
        call_args.truncate(call_args.len() - 2);
    }

    // Open the extern shim and upgrade EnvUnowned -> &mut Env<'_> via an
    // AttachGuard so the body can call get_string / new_string / throw_new etc.
    // We don't use `EnvUnowned::with_env` because it requires the closure to
    // return `Result<T, E>` and to call `.resolve::<P>()` on the outcome — a
    // significant refactor that would lose the existing early-return + sentinel
    // pattern. AttachGuard upgrades inline; panics inside the body are still
    // caught by `run_or_throw` (the existing per-call wrapper).
    out.push_str(&template_env::render(
        "function_shim_open.rs.jinja",
        context! {
            symbol => symbol,
            param_sigs => param_sigs,
            ret_decl => ret_decl,
        },
    ));

    out.push_str(&unmarshal);

    // Build the raw call expression (without async wrapping yet).
    let raw_call = if call_args.is_empty() {
        format!("{core_fn}()")
    } else {
        format!("{core_fn}({call_args})")
    };

    if has_error {
        let mut ok_body = String::new();
        if is_opaque_return {
            ok_body.push_str("            Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut ok_body, return_type, "            ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, true, err_null, &ok_body, "");
    } else {
        let mut value_body = String::new();
        if is_opaque_return {
            value_body.push_str("    Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut value_body, return_type, "    ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, false, err_null, "", &value_body);
    }
}

/// Emit a shim for an instance method on an opaque client type.
///
/// `receiver_is_mut` controls whether the handle is cast to `*mut T` (`&mut self`)
/// or `*const T` (`&self`).  `opaque_type_names` is used to identify handle-typed
/// params so they can be received as `jlong` rather than a JSON string.
#[allow(clippy::too_many_arguments)]
fn emit_method_shim(
    out: &mut String,
    symbol: &str,
    type_name: &str,
    method_name: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    is_async: bool,
    has_error: bool,
    receiver_is_mut: bool,
    receiver_owned: bool,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    let rust_method = method_name.replace('-', "_");
    let has_params = !params.is_empty();

    // Direct opaque return: `-> NamedType` where the type is opaque.
    let is_opaque_return = matches!(return_type, TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
    // Optional opaque return: `-> Option<NamedType>` where the inner type is opaque.
    let is_optional_opaque_return = matches!(
        return_type,
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if opaque_type_names.contains(n.as_str()))
    );

    let ret_decl = if is_opaque_return || is_optional_opaque_return {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    };
    let ret_null = if is_opaque_return || is_optional_opaque_return {
        "0"
    } else {
        method_return_null(return_type)
    };

    // For single-param methods with Vec<u8>/Bytes params: use jbyteArray as the
    // JNI parameter type (param name matches the rust param name, not request_json).
    // All other single-param and all multi-param methods use request_json: JString.
    let request_param = if !has_params {
        String::new()
    } else if params.len() == 1 {
        let p = &params[0];
        let rust_name = p.name.replace('-', "_");
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match base_ty {
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
                render_param_decl(&rust_name, "jbyteArray")
            }
            TypeRef::Bytes => render_param_decl(&rust_name, "jbyteArray"),
            _ => "    request_json: JString,\n".to_string(),
        }
    } else {
        "    request_json: JString,\n".to_string()
    };

    // See emit_function_shim for why we use AttachGuard::from_unowned instead
    // of EnvUnowned::with_env.
    out.push_str(&template_env::render(
        "method_shim_open.rs.jinja",
        context! {
            symbol => symbol,
            request_param => request_param,
            ret_decl => ret_decl,
        },
    ));

    // Dereference handle.
    out.push_str(&template_env::render(
        "method_client_handle.rs.jinja",
        context! {
            receiver_owned => receiver_owned,
            receiver_is_mut => receiver_is_mut,
            type_name => type_name,
        },
    ));

    // Unmarshal params and build call_args with is_ref/optional adjustments.
    let call_args: String = if !has_params {
        String::new()
    } else if params.len() == 1 {
        let p = &params[0];
        let rust_name = p.name.replace('-', "_");
        // Unwrap Optional wrapper for the JNI unmarshal type.
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        // Branches that understand the target's optional sentinel produce an
        // `Option<T>` binding directly. Other special cases bind the unwrapped
        // `T` and need `Some(name)` wrapping at the call site.
        let unmarshal_produces_option = p.optional
            && (matches!(base_ty, TypeRef::Bytes)
                || matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)))
                || !matches!(base_ty, TypeRef::Vec(_) | TypeRef::Path | TypeRef::String));
        emit_single_param_unmarshal(out, &rust_name, base_ty, ret_null, unmarshal_produces_option);
        // Apply optional/is_ref at the call site.
        // Special case: Vec<String> with is_ref means the core expects `&[&str]`.
        // emit_single_param_unmarshal already bound `<name>_vec: Vec<String>`.
        // We need to collect `Vec<&str>` refs and pass `&<name>_refs`.
        let is_vec_string_ref =
            p.is_ref && matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
        if is_vec_string_ref {
            let refs_name = format!("{rust_name}_refs");
            out.push_str(&render_vec_string_refs(&refs_name, &format!("{rust_name}_vec")));
            format!("&{refs_name}")
        } else if unmarshal_produces_option {
            // Binding is already `Option<T>` — pass through.
            rust_name
        } else if p.optional {
            format!("Some({rust_name})")
        } else if p.is_ref {
            format!("&{rust_name}")
        } else {
            rust_name
        }
    } else {
        // Multi-param: decode JSON map.
        out.push_str(&template_env::render(
            "request_map_unmarshal.rs.jinja",
            context! {
                ret_null => ret_null,
            },
        ));
        let mut args = Vec::new();
        for p in params {
            let rust_name = p.name.replace('-', "_");
            // Unwrap Optional for the deserialization type.
            let base_ty = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let type_path = type_ref_to_core_path(base_ty, "core_crate");
            out.push_str(&template_env::render(
                "request_map_param_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                    type_path => type_path,
                    ret_null => ret_null,
                },
            ));
            // Special case: Vec<String> with is_ref means the core expects `&[&str]`.
            let is_vec_string_ref =
                p.is_ref && matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
            let call_arg = if is_vec_string_ref {
                let refs_name = format!("{rust_name}_refs");
                out.push_str(&render_vec_string_refs(&refs_name, &rust_name));
                format!("&{refs_name}")
            } else if p.optional {
                format!("Some({rust_name})")
            } else if p.is_ref {
                format!("&{rust_name}")
            } else {
                rust_name
            };
            args.push(call_arg);
        }
        args.join(", ")
    };

    // Build the call.
    let call_expr = if call_args.is_empty() {
        format!("client.{rust_method}()")
    } else {
        format!("client.{rust_method}({call_args})")
    };

    if has_error {
        let mut ok_body = String::new();
        if is_opaque_return {
            ok_body.push_str("            Box::into_raw(Box::new(v)) as jlong\n");
        } else if is_optional_opaque_return {
            ok_body.push_str("            match v {\n");
            ok_body.push_str("                None => 0i64,\n");
            ok_body.push_str("                Some(inner) => Box::into_raw(Box::new(inner)) as jlong,\n");
            ok_body.push_str("            }\n");
        } else {
            emit_return_marshal(&mut ok_body, return_type, ret_null);
        }
        render_call_result_body(out, &call_expr, is_async, true, ret_null, &ok_body, "");
    } else {
        let mut value_body = String::new();
        if is_opaque_return {
            value_body.push_str("    Box::into_raw(Box::new(v)) as jlong\n");
        } else if is_optional_opaque_return {
            value_body.push_str("    match v {\n");
            value_body.push_str("        None => 0i64,\n");
            value_body.push_str("        Some(inner) => Box::into_raw(Box::new(inner)) as jlong,\n");
            value_body.push_str("    }\n");
        } else {
            emit_return_marshal_with_indent(&mut value_body, return_type, "    ", ret_null);
        }
        render_call_result_body(out, &call_expr, is_async, false, ret_null, "", &value_body);
    }
}

fn render_call_result_body(
    out: &mut String,
    call_expr: &str,
    is_async: bool,
    has_error: bool,
    ret_null: &str,
    ok_body: &str,
    value_body: &str,
) {
    let async_call_expr = format!("runtime().block_on({call_expr})");
    out.push_str(&template_env::render(
        "call_result_body.rs.jinja",
        context! {
            call_expr => call_expr,
            async_call_expr => async_call_expr,
            is_async => is_async,
            has_error => has_error,
            ret_null => ret_null,
            ok_body => ok_body,
            value_body => value_body,
        },
    ));
}

/// Emit unmarshal code for a single param.
///
/// Special cases:
/// - `Vec<u8>` / `Bytes`: the JNI param is `<rust_name>: jbyteArray`; use
///   `env.convert_byte_array` — no JSON round-trip.
/// - `Path` (`PathBuf`): the JNI param is `request_json: JString`; construct
///   `std::path::PathBuf::from(string)` instead of JSON-deserializing.
/// - Everything else: JSON-deserialize from `request_json: JString`.
///
/// When `is_optional` is true, the emitted binding has type `Option<T>` and an
/// empty-string sentinel (from Kotlin's `obj?.let { writeValueAsString(it) } ?: ""`)
/// is decoded as `None` rather than failing with `EOF while parsing`.
fn emit_single_param_unmarshal(out: &mut String, rust_name: &str, ty: &TypeRef, ret_null: &str, is_optional: bool) {
    match ty {
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            // jbyteArray → Vec<u8> via env.convert_byte_array.
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Bytes => {
            // jbyteArray → Vec<u8> via env.convert_byte_array.
            // The caller uses is_ref=true which will pass &<name> (coerces &Vec<u8> → &[u8]).
            // No bytes crate dependency needed.
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Path => {
            // JString → PathBuf via raw string (no JSON decode).
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "path_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            // Vec<String> — deserialize into `<name>_vec` so the caller can optionally
            // produce `<name>_refs: Vec<&str>` for `&[&str]` call sites.
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "vec_string_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                    ret_null => ret_null,
                },
            ));
        }
        TypeRef::String => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            // A JSON-encoded string from Kotlin: `MAPPER.writeValueAsString(strParam)` → `"\"hello\""`
            out.push_str(&template_env::render(
                "request_string_value_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                },
            ));
        }
        _ => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            let type_path = type_ref_to_core_path(ty, "core_crate");
            // Kotlin passes "" as the sentinel for None (so we don't have to
            // round-trip a JSON `null` and the wire stays clean for the Some case).
            out.push_str(&template_env::render(
                "json_value_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                    type_path => type_path,
                    ret_null => ret_null,
                    is_optional => is_optional,
                },
            ));
        }
    }
}

/// Emit the return marshalling code inside the `Ok(v) =>` arm.
fn emit_return_marshal(out: &mut String, return_type: &TypeRef, ret_null: &str) {
    emit_return_marshal_with_indent(out, return_type, "            ", ret_null);
}

/// Emit the return marshalling code with a configurable leading indent.
///
/// Use the 12-space variant from inside an `Ok(v) =>` match arm; pass a
/// 4-space indent for the no-error code path that binds `v` directly.
///
/// `ret_null` is the sentinel value emitted on serialization failure so the
/// caller can distinguish an error return from a legitimate zero/null result.
fn emit_return_marshal_with_indent(out: &mut String, return_type: &TypeRef, indent: &str, ret_null: &str) {
    match return_type {
        TypeRef::Unit => {
            // No return value.
        }
        TypeRef::Primitive(PrimitiveType::Bool) => {
            // jni 0.22 + jni-sys 0.4 made `jboolean` a `bool` (it was `u8` in
            // 0.21), so a `bool as bool` cast is a Rust compile error. Return
            // the value as-is.
            out.push_str(&template_env::render(
                "return_bool.rs.jinja",
                context! {
                    indent => indent,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            // Vec<u8> → jbyteArray
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                context! {
                    indent => indent,
                    bytes_expr => "&v",
                },
            ));
        }
        TypeRef::Bytes => {
            // bytes::Bytes → jbyteArray (same as Vec<u8>)
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                context! {
                    indent => indent,
                    bytes_expr => "v.as_ref()",
                },
            ));
        }
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            let bytes_expr = match inner.as_ref() {
                TypeRef::Bytes => "bytes.as_ref()",
                _ => "&bytes",
            };
            out.push_str(&template_env::render(
                "return_optional_byte_array.rs.jinja",
                context! {
                    indent => indent,
                    bytes_expr => bytes_expr,
                },
            ));
        }
        TypeRef::Primitive(p) => {
            // Cast the Rust primitive to the corresponding JNI numeric type.
            // This handles mismatches like u16 → jshort (i16), usize → jlong (i64).
            let jni_ty = jni_primitive_type(p);
            out.push_str(&template_env::render(
                "return_primitive.rs.jinja",
                context! {
                    indent => indent,
                    jni_ty => jni_ty,
                },
            ));
        }
        _ => {
            out.push_str(&template_env::render(
                "return_json.rs.jinja",
                context! {
                    indent => indent,
                    ret_null => ret_null,
                },
            ));
        }
    }
}

/// Emit a constructor shim for an opaque client type.
///
/// The `client_constructors` workspace config supplies the body template and
/// the ordered list of parameters.  Each parameter whose `ty` contains
/// `c_char` is received as `JString` and unmarshalled via `jstring_to_string`;
/// other parameter types are received as their JNI primitive equivalent.
///
/// The emitted shim returns `jlong` (a `Box::into_raw` pointer) on success or
/// `0` on failure (with a JNI exception pending).
fn emit_constructor_shim(
    out: &mut String,
    symbol: &str,
    ty: &TypeDef,
    config: &ResolvedCrateConfig,
    ctor: &ClientConstructorConfig,
) {
    let type_name = &ty.name;
    let core_prefix = core_use_path(config);

    // Build param signature lines and unmarshal blocks.
    let mut param_sigs = String::new();
    let mut unmarshal = String::new();
    let mut call_args = Vec::new();

    for param in &ctor.params {
        let rust_name = param.name.replace('-', "_");
        if param.ty.contains("c_char") {
            // String parameter: receive as JString and unmarshal to Rust String.
            param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
            unmarshal.push_str(&render_string_unmarshal(&rust_name, "0"));
            call_args.push(rust_name.clone());
        } else {
            // Non-string: use as-is (caller passes primitive JNI type).
            param_sigs.push_str(&render_param_decl(&rust_name, "jlong"));
            call_args.push(rust_name.clone());
        }
    }

    // Expand the body template.
    let body_expr = ctor
        .body
        .replace("{type_name}", type_name)
        .replace("{source_path}", &format!("{core_prefix}::{type_name}"));

    // Build the call expression: body_expr already encodes the full constructor
    // call (e.g. `core_crate::DemoClient::new(api_key)`).  If the body uses
    // positional references we substitute them; otherwise trust the template.
    // When the body template ends with `(...)` we leave it intact.
    let call_expr = if call_args.is_empty() || body_expr.contains('(') {
        body_expr.clone()
    } else {
        format!("{}({})", body_expr, call_args.join(", "))
    };

    // Always treat the constructor as fallible (match Result<_, E>) since the
    // typical body is `core_crate::TypeName::new(param)` which returns Result.
    out.push_str(&template_env::render(
        "constructor_shim.rs.jinja",
        context! {
            symbol => symbol,
            param_sigs => param_sigs,
            unmarshal => unmarshal,
            call_expr => call_expr,
        },
    ));
}

/// Emit the destructor shim for an opaque type.
fn emit_destructor_shim(out: &mut String, symbol: &str, type_name: &str) {
    out.push_str(&template_env::render(
        "destructor_shim.rs.jinja",
        context! {
            symbol => symbol,
            type_name => type_name,
        },
    ));
}

/// Emit Start/Next/Free streaming shims for one adapter.
#[allow(clippy::too_many_arguments)]
fn emit_streaming_shims(
    out: &mut String,
    start_sym: &str,
    next_sym: &str,
    free_sym: &str,
    ty: &TypeDef,
    adapter: &crate::core::config::AdapterConfig,
    _api: &ApiSurface,
) {
    let type_name = &ty.name;
    let adapter_pascal = internal_class_component(&adapter.name);
    let stream_handle_type = format!("{type_name}{adapter_pascal}StreamHandle");
    let adapter_method = adapter.name.replace('-', "_");

    // Determine item type path.
    let item_type = adapter
        .item_type
        .as_deref()
        .map(|t| format!("core_crate::{t}"))
        .unwrap_or_else(|| "serde_json::Value".to_string());

    let stream_item_alias = format!("{stream_handle_type}Item");
    let stream_box_alias = format!("{stream_handle_type}Stream");
    let mut request_unmarshal = String::new();
    let stream_call_block;
    if let Some(first_param) = adapter.params.first() {
        let param_type = first_param.ty.rsplit("::").next().unwrap_or(&first_param.ty);
        request_unmarshal.push_str(&template_env::render(
            "stream_request_unmarshal.rs.jinja",
            context! {
                param_type => param_type,
            },
        ));
        stream_call_block = template_env::render(
            "stream_call_block.rs.jinja",
            context! {
                adapter_method => adapter_method,
                request_arg => "request",
            },
        );
    } else {
        stream_call_block = template_env::render(
            "stream_call_block.rs.jinja",
            context! {
                adapter_method => adapter_method,
                request_arg => "",
            },
        );
    }

    out.push_str(&template_env::render(
        "streaming_shims.rs.jinja",
        context! {
            stream_item_alias => stream_item_alias,
            stream_box_alias => stream_box_alias,
            stream_handle_type => stream_handle_type,
            item_type => item_type,
            start_sym => start_sym,
            next_sym => next_sym,
            free_sym => free_sym,
            type_name => type_name,
            request_unmarshal => request_unmarshal,
            stream_call_block => stream_call_block,
        },
    ));
}

// ---------------------------------------------------------------------------
// Return type helpers
// ---------------------------------------------------------------------------

fn internal_class_component(name: &str) -> String {
    to_class_name(name)
}

/// Return the ` -> <JniReturnType>` suffix for a method shim signature.
fn method_return_type_decl(return_type: &TypeRef) -> String {
    match return_type {
        TypeRef::Unit => String::new(),
        TypeRef::Primitive(PrimitiveType::Bool) => " -> jboolean".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            " -> jbyteArray".to_string()
        }
        TypeRef::Bytes => " -> jbyteArray".to_string(),
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            " -> jbyteArray".to_string()
        }
        TypeRef::Primitive(_) => {
            let jni_ty = jni_return_type(return_type);
            format!(" -> {jni_ty}")
        }
        _ => " -> jstring".to_string(),
    }
}

/// Return the "null" / zero value for a method return type (used in error paths).
fn method_return_null(return_type: &TypeRef) -> &'static str {
    match return_type {
        TypeRef::Unit => "()",
        // jni 0.22 + jni-sys 0.4 changed `jboolean` from `u8` to `bool`; the
        // sentinel value for an error-path return therefore needs to be `false`,
        // not the legacy `0u8`.
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(PrimitiveType::F32) => "0.0f32",
        TypeRef::Primitive(PrimitiveType::F64) => "0.0f64",
        TypeRef::Primitive(_) => "0",
        TypeRef::Bytes => "std::ptr::null_mut()",
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            "std::ptr::null_mut()"
        }
        _ => "std::ptr::null_mut()",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a TypeRef to a JNI return type string.
fn jni_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "()",
        TypeRef::Primitive(p) => jni_primitive_type(p),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => "jbyteArray",
        TypeRef::Bytes => "jbyteArray",
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            "jbyteArray"
        }
        // String and complex types cross the boundary as Java objects.
        TypeRef::String | TypeRef::Named(_) | TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => "jstring",
        // Opaque handles → Long.
        _ => "jlong",
    }
}

fn jni_primitive_type(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "jboolean",
        PrimitiveType::I8 | PrimitiveType::U8 => "jni::sys::jbyte",
        PrimitiveType::I16 | PrimitiveType::U16 => "jni::sys::jshort",
        PrimitiveType::I32 | PrimitiveType::U32 => "jni::sys::jint",
        PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "jlong",
        PrimitiveType::F32 => "jni::sys::jfloat",
        PrimitiveType::F64 => "jni::sys::jdouble",
    }
}

/// Return the Rust zero-literal for a JNI primitive, used as the null-sentinel
/// for optional primitive parameters.  Returns None for `Bool`, which has no
/// meaningful "absent" sentinel (false is a real value); optional bools cannot
/// be marshalled through plain JNI primitives.
fn primitive_zero_literal(p: &PrimitiveType) -> Option<&'static str> {
    match p {
        PrimitiveType::Bool => None,
        PrimitiveType::I8
        | PrimitiveType::U8
        | PrimitiveType::I16
        | PrimitiveType::U16
        | PrimitiveType::I32
        | PrimitiveType::U32
        | PrimitiveType::I64
        | PrimitiveType::U64
        | PrimitiveType::Usize
        | PrimitiveType::Isize => Some("0"),
        PrimitiveType::F32 | PrimitiveType::F64 => Some("0.0"),
    }
}

/// Return a Rust cast target for a JNI primitive → Rust type conversion, or "" if no cast needed.
fn primitive_cast(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "bool",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::U64 => "u64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Map a TypeRef to a Rust type path for serde deserialization.
fn type_ref_to_core_path(ty: &TypeRef, core_prefix: &str) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(p) => primitive_rust_type(p).to_string(),
        TypeRef::Named(n) => format!("{core_prefix}::{n}"),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_core_path(inner, core_prefix)),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_core_path(inner, core_prefix)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_core_path(k, core_prefix),
            type_ref_to_core_path(v, core_prefix)
        ),
        _ => "serde_json::Value".to_string(),
    }
}

fn primitive_rust_type(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "bool",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::U64 => "u64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Resolve the Kotlin package string used when constructing JNI symbols.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then falls back to `config.kotlin_package()`.
fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
}

/// Resolve the fully-qualified error class name for `ERROR_CLASS`.
///
/// Uses `<package_slashed>/<BridgeName>Exception` as default.
fn resolve_error_class(config: &ResolvedCrateConfig, package: &str) -> String {
    let package_slashed = package.replace('.', "/");
    let bridge = bridge_class_name(&config.name);
    format!("{package_slashed}/{bridge}Exception")
}

/// Return the `use` path for the core crate from the JNI shim.
///
/// Uses the `name` field of the config (which is the crate name, e.g.
/// `sample-llm`), converting hyphens to underscores per Rust convention.
fn core_use_path(config: &ResolvedCrateConfig) -> String {
    config.name.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jni_return_type_unit() {
        assert_eq!(jni_return_type(&TypeRef::Unit), "()");
    }

    #[test]
    fn jni_return_type_i64() {
        assert_eq!(jni_return_type(&TypeRef::Primitive(PrimitiveType::I64)), "jlong");
    }

    #[test]
    fn jni_return_type_string() {
        assert_eq!(jni_return_type(&TypeRef::String), "jstring");
    }

    #[test]
    fn jni_return_type_vec_u8() {
        assert_eq!(
            jni_return_type(&TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8)))),
            "jbyteArray"
        );
    }

    /// The generated `throw_jni_error` helper must use `env.throw_new(...).is_err()`
    /// and fall back to `java/lang/RuntimeException` rather than silently discarding
    /// a failed throw (which would leave the Kotlin caller with no exception pending
    /// and a null/zero sentinel that looks like a valid return value).
    #[test]
    fn throw_jni_error_has_runtime_exception_fallback() {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
        )
        .unwrap();
        let config = raw.resolve().unwrap().remove(0);
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let content = emit_lib_rs(&api, &config);
        // The generated helper must NOT use `let _ = env.throw_new(...)` which
        // silently swallows a missing-class error.
        assert!(
            !content.contains("let _ = env.throw_new(ERROR_CLASS"),
            "throw_jni_error must not discard the throw_new result: {content}"
        );
        // It must check the result and fall back to RuntimeException.
        // (`ERROR_CLASS` / `msg` are now wrapped in `JNIString::from(...)` per
        // the jni 0.22 API; assert on the structural pattern instead of the
        // exact arg form.)
        assert!(
            content.contains("if env.throw_new(&class_jni, &msg_jni).is_err()"),
            "throw_jni_error must check throw_new result: {content}"
        );
        assert!(
            content.contains("jni::strings::JNIString::from(ERROR_CLASS)"),
            "throw_jni_error must wrap ERROR_CLASS in JNIString::from: {content}"
        );
        assert!(
            content.contains("java/lang/RuntimeException"),
            "throw_jni_error must fall back to RuntimeException: {content}"
        );
    }
}
