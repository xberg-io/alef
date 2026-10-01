//! Small, self-contained WASM backend helpers factored out of `mod.rs` to keep it under the
//! repo's file-size ceiling: symbol-reachability queries (`wasm_callability` and friends), cfg/
//! omission-marker string prep, and two narrowly-scoped codegen post-processing passes. None of
//! these depend on `WasmBackend` or `generate_bindings`'s own locals -- they were free functions
//! sitting above the `impl Backend for WasmBackend` block, so the split is purely mechanical.

use crate::codegen::cfg::enabled_features_for_language;
use crate::codegen::shared;
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_layout};
use crate::core::ir::{ApiSurface, ReceiverKind};
use ahash::AHashSet;
use regex::Regex;

use super::cfg::is_gated_behind_disabled_feature;
use super::trait_bridge_docs;

/// The emitted wasm crate's directory layout.
///
/// One derivation shared by the file-path side (`lib.rs`, `Cargo.toml`) and the manifest's own
/// core-crate `path = "..."` dependency. When `gen_cargo_toml` hard-coded `../{core_crate_dir}`
/// instead, the two disagreed for any layout that is not a `crates/` sibling pair: the manifest
/// pointed at a `crates/<core>` that the emitted tree does not contain, and cargo failed to read
/// it before compiling anything. ~keep
pub(super) fn wasm_output_layout(config: &ResolvedCrateConfig) -> crate::core::config::OutputLayout {
    resolve_output_layout(config.output_paths.get("wasm"), &config.name, "crates/{name}-wasm/src/")
}

/// Why a symbol is or is not callable from a WASM snippet.
///
/// `UnknownSymbol` stays distinct from `NotExported` because collapsing them misdirects the
/// reader: a typo'd `overrides.wasm.function` then reads as a capability gap in the target,
/// sending someone to audit the wasm backend for a name that was only ever misspelled in
/// config. That misdirection already cost one diagnostic cycle on this exact gate. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmCallability {
    Callable,
    /// A symbol by this name exists, but the target does not export it.
    NotExported,
    /// Nothing in the API surface or the bridge registry answers to this name under either the
    /// Rust or the JavaScript spelling.
    UnknownSymbol,
}

pub(crate) fn wasm_callability(
    function_name: &str,
    functions: &[crate::core::ir::FunctionDef],
    config: &ResolvedCrateConfig,
) -> WasmCallability {
    if function_is_callable(function_name, functions, config) {
        return WasmCallability::Callable;
    }
    match rust_identity_for_wasm_symbol(function_name, functions, config) {
        Some(_) => WasmCallability::NotExported,
        None => WasmCallability::UnknownSymbol,
    }
}

/// Whether a WASM *snippet* or *test* may call `function_name`.
///
/// Deliberately wider than [`function_is_exported`], which answers a codegen question — "should
/// the plain-function generator emit a wrapper for this?" — and returns `false` for trait-bridge
/// register/unregister/clear functions precisely because the trait-bridge generator emits them
/// instead. Those functions are exported all the same, so a caller asking "is this callable?"
/// must not reuse the codegen predicate.
///
/// Prefer [`wasm_callability`] when the answer feeds a diagnostic: this predicate cannot say
/// whether a `false` means the target dropped the function or the name resolved to nothing.
pub(super) fn function_is_callable(
    function_name: &str,
    functions: &[crate::core::ir::FunctionDef],
    config: &ResolvedCrateConfig,
) -> bool {
    let Some(identity) = rust_identity_for_wasm_symbol(function_name, functions, config) else {
        return false;
    };
    // An explicit exclusion outranks the bridge: the symbol is not emitted at all.
    if config
        .wasm
        .as_ref()
        .is_some_and(|wasm| wasm.exclude_functions.iter().any(|name| name == identity))
    {
        return false;
    }
    if config.trait_bridge_manages_function(identity) {
        return true;
    }
    function_is_exported(identity, functions, config)
}

/// Resolve the Rust identity behind a symbol that may be spelled the way JavaScript sees it.
///
/// wasm-bindgen exports every symbol under `js_name = to_camel_case(rust_name)`, so a
/// `[e2e.calls.<name>.overrides.wasm] function` legitimately names `clearRerankerBackends` for a
/// Rust `clear_reranker_backends`. Every question below is keyed on the Rust identity, so matching
/// the configured spelling against those keys directly reports a symbol the target does export as
/// missing. Trait-bridge registry operations are searched alongside `functions` because the
/// trait-bridge generator emits them and they need not appear in the plain function surface.
///
/// Returns `None` when nothing the target could export answers to `symbol` under either spelling.
pub(super) fn rust_identity_for_wasm_symbol<'a>(
    symbol: &str,
    functions: &'a [crate::core::ir::FunctionDef],
    config: &'a ResolvedCrateConfig,
) -> Option<&'a str> {
    // The Rust spelling is matched first so a crate that happens to export both `foo_bar` and
    // `fooBar` resolves an exact name to itself rather than to whichever came first. ~keep
    wasm_export_candidates(functions, config)
        .find(|candidate| *candidate == symbol)
        .or_else(|| {
            wasm_export_candidates(functions, config)
                .find(|candidate| crate::codegen::generators::trait_bridge::to_camel_case(candidate) == symbol)
        })
}

/// Every Rust identity the WASM target could export: the plain function surface plus the
/// registry operations the trait-bridge generator emits.
pub(super) fn wasm_export_candidates<'a>(
    functions: &'a [crate::core::ir::FunctionDef],
    config: &'a ResolvedCrateConfig,
) -> impl Iterator<Item = &'a str> {
    let bridge_registry_fns = trait_bridge_docs::active_bridges(config)
        .flat_map(|bridge| {
            [
                bridge.register_fn.as_deref(),
                bridge.unregister_fn.as_deref(),
                bridge.clear_fn.as_deref(),
            ]
        })
        .flatten();
    functions
        .iter()
        .map(|function| function.name.as_str())
        .chain(bridge_registry_fns)
}

pub(super) fn function_is_exported(
    function_name: &str,
    functions: &[crate::core::ir::FunctionDef],
    config: &ResolvedCrateConfig,
) -> bool {
    if config
        .wasm
        .as_ref()
        .is_some_and(|wasm| wasm.exclude_functions.iter().any(|name| name == function_name))
    {
        return false;
    }
    if config.trait_bridge_manages_function(function_name) {
        return false;
    }
    let enabled_features = &enabled_features_for_language(config, Language::Wasm);
    let core_import = config.core_import_for_language(Language::Wasm);
    let source_remaps = config
        .wasm
        .as_ref()
        .map(|wasm| &wasm.source_crate_remaps)
        .into_iter()
        .flatten()
        .map(|name| name.replace('-', "_"))
        .collect::<AHashSet<_>>();
    let dropped_crates = config
        .wasm
        .as_ref()
        .map(|wasm| &wasm.exclude_extra_dependencies)
        .into_iter()
        .flatten()
        .map(|name| name.replace('-', "_"))
        .filter(|name| name != &core_import && !source_remaps.contains(name))
        .collect::<AHashSet<_>>();
    functions.iter().any(|function| {
        function.name == function_name
            && !is_gated_behind_disabled_feature(&function.cfg, enabled_features)
            && !dropped_crates.contains(&function.rust_path.split("::").next().unwrap_or("").replace('-', "_"))
    })
}

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
pub(super) fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

/// Prepend a visible marker comment listing fields dropped from `item` because they reference a
/// type with no generated wasm binding (see `first_unknown_named_type` in `cfg.rs`).
///
/// The comment is emitted directly above the struct so the omission is discoverable by reading
/// the generated source, not just by grepping build logs for the accompanying `tracing::warn!`
/// (nothing may be silently omitted from a binding).
pub(super) fn prepend_unknown_type_omission_marker(omissions: Option<&Vec<(String, String)>>, item: String) -> String {
    let Some(omissions) = omissions else {
        return item;
    };
    let mut marker = String::from(
        "// ALEF-OMITTED: the field(s) below were dropped from this WASM binding\n\
         // because their Rust type has no generated wasm-bindgen representation.\n",
    );
    for (field_name, type_name) in omissions {
        marker.push_str(&format!(
            "//   - field `{field_name}`: type `{type_name}` is not part of the bound wasm API surface\n"
        ));
    }
    format!("{marker}{item}")
}

/// Types for which `methods::gen_method` emits a self-delegating
/// `{core_import}::{type_name}::from(self.clone()).{method}(..)` call for at least one method.
///
/// That delegation form is a hard requirement on `impl From<Wasm{Type}> for {core}::{Type}`
/// existing (see `methods.rs`). The reverse (`binding -> core`) conversion is otherwise only
/// emitted for types in `input_type_names(api)` — types reachable as a function/method
/// parameter, directly or transitively through struct fields. A struct that is only ever
/// *returned* (never taken as a parameter, directly or transitively) but that also has an
/// auto-delegated instance method — e.g. `PageRange::page_count(&self)` — falls through that
/// gap: `input_type_names` has no reason to include it, yet `gen_method` still needs the
/// reverse impl to compile the delegation. Mirrors the exact branching `gen_method` uses to
/// decide between self-delegation and the opaque mutex-lock path, so the two stay in sync.
pub(super) fn types_needing_self_delegation_reverse_impl(
    api: &ApiSurface,
    opaque_types: &AHashSet<String>,
) -> AHashSet<String> {
    let mut needed = AHashSet::default();
    for typ in api.types.iter().filter(|t| !t.is_trait) {
        let has_mut_methods = typ
            .methods
            .iter()
            .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));
        let is_opaque_type = opaque_types.contains(&typ.name);

        for method in &typ.methods {
            if method.is_static {
                continue;
            }
            let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(ReceiverKind::RefMut));
            // Mirrors gen_method: this path calls `self.inner.lock().unwrap().{method}(..)`
            // directly on the core value held by the opaque wrapper — no `From` impl needed.
            if is_opaque_type && has_mut_methods && !is_ref_mut_receiver {
                continue;
            }

            let delegates_via_self_conversion = if method.is_async {
                // gen_method's async branch always builds `core_call` via self-delegation
                // (or the mutex path excluded above), regardless of `can_delegate`.
                true
            } else if is_ref_mut_receiver && has_mut_methods {
                !method.sanitized
                    && method
                        .params
                        .iter()
                        .all(|p| !p.sanitized && shared::is_delegatable_param(&p.ty, opaque_types))
                    && shared::is_opaque_delegatable_type(&method.return_type)
            } else {
                shared::can_auto_delegate(method, opaque_types)
            };

            if delegates_via_self_conversion {
                needed.insert(typ.name.clone());
                break;
            }
        }
    }
    needed
}

/// Fix up `<field>: Default::default().map(Box::new),` lines left behind by the shared
/// binding->core `From` conversion generator (`crate::codegen::conversions`, shared with every
/// other backend) when a field's type is a payload-carrying enum (a `#[serde(tag = "type")]`
/// enum with struct variants).
///
/// wasm_bindgen only supports fieldless, C-style enums, so `gen_struct` (this backend, see
/// `types.rs`) drops any field referencing such an enum from the generated Wasm struct entirely.
/// The shared conversion generator does not know the field was dropped: it still emits a value
/// for it and falls back to `Default::default()`. For an `Option<Box<T>>` field the generic
/// Option<Box<_>> wrapper then unconditionally appends `.map(Box::new)`, producing
/// `Default::default().map(Box::new)` -- a value that is always `None` (`Option::default()` is
/// `None` for every `T`, and `None.map(Box::new)` is `None`), but whose type `T` rustc cannot
/// infer (E0282), since nothing in the expression pins it down.
///
/// Replacing the whole expression with the equivalent literal `None` is behavior-preserving and
/// compiles; the comment documents *why* the field is always `None` on wasm for anyone reading
/// the generated binding.
pub(super) fn fix_dropped_payload_enum_option_fields(content: String) -> String {
    let Ok(dropped_boxed_option_field) =
        Regex::new(r"(?m)^(?P<indent>[ \t]*)(?P<field>\w+): Default::default\(\)\.map\(Box::new\),$")
    else {
        return content;
    };
    dropped_boxed_option_field
        .replace_all(&content, |caps: &regex::Captures<'_>| {
            format!(
                "{indent}// ALEF-OMITTED: `{field}` is always None on wasm -- its Rust type is a \
                 payload-carrying enum, which wasm_bindgen cannot represent.\n\
                 {indent}{field}: None,",
                indent = &caps["indent"],
                field = &caps["field"],
            )
        })
        .into_owned()
}
