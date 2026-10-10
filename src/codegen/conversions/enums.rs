use std::collections::HashSet;

use crate::codegen::cfg::is_host_owned_rust_path;
use crate::core::ir::{EnumDef, EnumVariant};

use super::ConversionConfig;
use super::helpers::{binding_to_core_match_arm_ext_cfg, core_enum_path_remapped, core_to_binding_match_arm_ext_cfg};

/// Whether a generated wrapper type (the `#[napi(string_enum)] pub enum Js...`,
/// `#[wasm_bindgen] pub enum Wasm...`, etc. a `gen_enum`-style backend emitter declares) should
/// include `variant`, and under what `#[cfg(...)]` guard, if any.
///
/// This is the SAME authority [`enum_conversion_needs_catch_all`]'s callers in this module
/// consult for the conversion arm belonging to the same variant, so a wrapper type's declared
/// shape and its conversion's match arms can never disagree about which variants exist. Two
/// distinct alef defects trace back to that disagreement: a disabled foreign variant still gets
/// declared unconditionally while its conversion adds a now-unreachable catch-all (alef #534),
/// and a host-owned variant's conversion arm carries a `#[cfg(...)]` guard the wrapper's OWN
/// declaration never mirrors, so the wrapper type is missing exactly the variant the arm exists
/// to handle whenever that feature is off from the wrapper's own point of view -- and, since the
/// wrapper's declaration is unconditional (no gate at all) while the arm's gate DOES vary with
/// the feature, the two also drift the moment the feature comes back on (alef #536). Attaching
/// the identical guard to both the wrapper's own declaration and its conversion arm removes the
/// second disagreement entirely: variant, arm, and now the wrapper's own copy of the variant all
/// compile in or out together. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantDeclaration {
    /// Emit `variant` in the wrapper's declaration, optionally behind `cfg`.
    Keep { cfg: Option<String> },
    /// Omit `variant` entirely: this binding's own configured feature set proves the gate can
    /// never be satisfied, so the variant can never exist for this binding either.
    Drop,
}

#[must_use]
pub fn enum_variant_declaration(
    variant: &EnumVariant,
    is_host_enum: bool,
    configured_features: Option<&HashSet<&str>>,
) -> VariantDeclaration {
    let Some(cfg) = variant.cfg.as_deref() else {
        return VariantDeclaration::Keep { cfg: None };
    };
    if is_host_enum {
        // The wrapper crate already declares this feature -- `codegen::cfg::collect_cfg_features`
        // walks host-owned items -- so attaching the identical guard here keeps the wrapper's own
        // declaration, its conversion arm, and the source variant compiling in or out together,
        // deferring exhaustiveness to the compiler instead of alef's own (necessarily incomplete)
        // static feature analysis. ~keep
        return VariantDeclaration::Keep {
            cfg: Some(cfg.to_string()),
        };
    }
    match configured_features {
        Some(features) if foreign_variant_proven_unreachable(cfg, features) => VariantDeclaration::Drop,
        // Unknown, or not proven absent: keep unconditionally. The wrapper crate cannot declare a
        // foreign crate's own feature as its own (`unexpected cfg condition value`), so a "maybe
        // reachable" foreign variant can only be represented unconditionally, never conditionally
        // compiled -- Cargo feature unification could still turn the dependency's feature on some
        // way alef's static configuration read cannot observe.
        _ => VariantDeclaration::Keep { cfg: None },
    }
}

/// Like [`enum_variant_declaration`], but for a target enum macro that cannot express a
/// conditionally-compiled variant at all -- resolves every decision to fully present (`Keep {
/// cfg: None }`) or fully absent (`Drop`), never `Keep { cfg: Some(_) }`.
///
/// wasm-bindgen's `#[wasm_bindgen]` is exactly such a macro: it parses an enum's variants from
/// the raw `syn::ItemEnum` token stream -- before cfg-stripping removes a disabled variant from
/// the item -- and unconditionally generates code (`IntoWasmAbi`, `TryFromJsValue`, ...)
/// referencing every variant it saw. Attaching `#[cfg(...)]` to only the variant's own
/// declaration line, mirroring what a host-owned variant's conversion arm carries, leaves that
/// generated code referencing a variant the compiler has already dropped: `E0599: no variant ...
/// found`, reported AT the declaration line that (correctly, syntactically) still names the
/// variant. See rustwasm/wasm-bindgen#2058 -- confirmed against the macro's own parser
/// (`wasm-bindgen-macro-support::parser`, which walks `self.variants.iter()` with no cfg
/// awareness) and codegen (`wasm-bindgen-macro-support::codegen::ToTokens for ast::Enum`, whose
/// `cast_clauses` unconditionally maps every variant name), not merely inferred from the error
/// shape. ~keep
///
/// A host-owned variant's gate is therefore evaluated directly against `configured_features`
/// here -- REQUIRED, unlike `enum_variant_declaration`'s optional parameter, because this caller
/// has no compiler-deferred escape hatch to fall back on the way a `#[cfg(...)]`-gated arm does.
/// A foreign-owned variant defers to `enum_variant_declaration` with `None`, keeping it always
/// unconditionally present exactly as before this function existed: proving a foreign variant
/// disabled here, while the conversion side's catch-all computation (which does not thread
/// `configured_features`) still assumes it might exist, would reopen the same "declaration and
/// conversion disagree" defect one level down. ~keep
#[must_use]
pub fn enum_variant_declaration_without_cfg_attribute(
    variant: &EnumVariant,
    is_host_enum: bool,
    configured_features: &HashSet<&str>,
) -> VariantDeclaration {
    if is_host_enum {
        return match variant.cfg.as_deref() {
            None => VariantDeclaration::Keep { cfg: None },
            Some(cfg) if crate::core::ir::cfg_feature_satisfied(Some(cfg), configured_features) => {
                VariantDeclaration::Keep { cfg: None }
            }
            Some(_) => VariantDeclaration::Drop,
        };
    }
    // Foreign: unchanged conservative behavior. This function exists only to work around
    // wasm-bindgen's inability to express per-variant cfg on a HOST-owned variant's own
    // declaration -- a foreign variant's declaration was already unconditional (never carried a
    // `#[cfg(...)]` attribute) before this function existed, so it stays unconditional here too.
    // Proving a foreign variant disabled would let this declaration disagree with the catch-all
    // `codegen::conversions::gen_enum_from_*_cfg` still emits for it, since wasm's own
    // `ConversionConfig` does not thread `configured_features` into that computation -- passing
    // `None` here keeps the two in lockstep the same way they already were. ~keep
    enum_variant_declaration(variant, is_host_enum, None)
}

/// Whether `cfg` (a foreign-owned variant's gate) is proven unsatisfied by this binding's own
/// configured feature set. Reuses [`crate::core::ir::cfg_feature_satisfied`], the canonical cfg
/// evaluator every other cfg-gating decision in alef defers to, rather than re-parsing the cfg
/// string here. ~keep
fn foreign_variant_proven_unreachable(cfg: &str, configured_features: &HashSet<&str>) -> bool {
    !crate::core::ir::cfg_feature_satisfied(Some(cfg), configured_features)
}

/// Whether at least one of `enum_def`'s FOREIGN cfg-gated variants remains a real gap that a
/// conversion catch-all must cover. Host-owned cfg-gated variants never count -- see
/// [`enum_conversion_needs_catch_all`]'s own doc comment for why the compiler already guarantees
/// exhaustiveness for those without a catch-all.
///
/// `declaration_may_drop_variant` says whether `configured_features`' proof that a foreign
/// variant is unreachable is actually reflected in the shape being matched: `true` when the match
/// is over the real CORE type (alef never declares that type, so the dependency's own compiled
/// shape -- exactly what `configured_features` predicts -- is the whole story), or over a BINDING
/// declaration that itself drops the variant on the same proof. `false` when the match is over a
/// BINDING declaration that keeps the variant unconditionally regardless of `configured_features`
/// -- there the gap is real independent of the proof, so every foreign cfg-gated variant counts.
/// See [`ConversionConfig::declaration_drops_unreachable_foreign_variants`]'s doc comment for the
/// full reasoning and which backends fall in which bucket. ~keep
fn has_unresolved_foreign_cfg_variants(
    enum_def: &EnumDef,
    is_host_enum: bool,
    configured_features: Option<&HashSet<&str>>,
    declaration_may_drop_variant: bool,
) -> bool {
    if is_host_enum {
        return false;
    }
    enum_def.variants.iter().any(|v| match v.cfg.as_deref() {
        None => false,
        Some(cfg) => {
            if !declaration_may_drop_variant {
                return true;
            }
            match configured_features {
                Some(features) => !foreign_variant_proven_unreachable(cfg, features),
                None => true,
            }
        }
    })
}

/// Build the `HashSet<&str>` view [`enum_variant_declaration`]/[`has_unresolved_foreign_cfg_variants`]
/// need from `config.configured_features`, once per generator call.
fn configured_features_set<'a>(config: &ConversionConfig<'a>) -> Option<HashSet<&'a str>> {
    config
        .configured_features
        .map(|features| features.iter().map(String::as_str).collect())
}

/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; the generated binding crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`, the same authority this asks), so forwarding it verbatim as
/// `#[cfg(feature = "...")]` is an `unexpected cfg condition value` error. Such an arm is dropped
/// entirely instead -- named and counted via `tracing::debug!`, not silently;
/// `codegen::foreign_cfg_variants` raises the same fact to WARN once for the whole run, instead of
/// once per backend and direction that drops the arm -- mirroring
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` and
/// `backends::swift::gen_rust_crate::enums::emit_enum_wrapper`. A host-owned cfg keeps its arm and
/// its `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
///
/// Shared by every backend that calls [`gen_enum_from_core_to_binding_cfg`] /
/// [`gen_enum_from_binding_to_core_cfg`] (napi, magnus, rustler, wasm) -- fixing the decision once
/// here fixes it for all of them, the same "ask cfg.rs" pattern `is_host_owned_rust_path` exists
/// to enforce. ~keep
fn emit_cfg_gated_arm(
    enum_def: &EnumDef,
    variant: &crate::core::ir::EnumVariant,
    is_host_enum: bool,
    arm: String,
    direction: &str,
) -> Option<minijinja::value::Value> {
    let Some(cfg) = variant.cfg.as_deref() else {
        return Some(crate::alef_context! { arm => arm, cfg => Option::<&str>::None });
    };
    if !is_host_enum {
        tracing::debug!(
            enum_name = %enum_def.name,
            enum_rust_path = %enum_def.rust_path,
            variant_name = %variant.name,
            cfg = cfg,
            direction = direction,
            "dropping enum conversion match arm for a foreign-crate variant behind a #[cfg(...)] \
             this binding crate cannot declare as a Cargo feature; the variant is unreachable \
             from this conversion"
        );
        return None;
    }
    Some(crate::alef_context! { arm => arm, cfg => cfg })
}

/// Whether a generated `From<...>` match over `enum_def`'s variants needs a trailing
/// `_ => Default::default()` catch-all to stay exhaustive under `-D warnings`.
///
/// A cfg-gated variant that is host-owned keeps a match arm carrying the identical
/// `#[cfg(...)]` guard as the variant's own declaration ([`emit_cfg_gated_arm`] and every
/// backend-local mirror of it), so in any single build the variant and its arm compile in or
/// out together: the match stays exhaustive either way and a catch-all over it is never
/// reachable. Only a cfg-gated variant that gets DROPPED -- a foreign-crate variant whose
/// `#[cfg(...)]` names a feature this binding crate cannot declare, see
/// `codegen::cfg::is_host_owned_rust_path` -- leaves a real gap: the match loses that arm
/// unconditionally while the matched type may still carry the variant, so a catch-all is
/// required. `has_cfg_variants` alone (ignoring host ownership) over-reports this and produces
/// an `unreachable_patterns` error the moment every cfg-gated variant happens to be host-owned,
/// which is the common case once every binding language forwards its cfg features (alef #464).
///
/// `has_excluded_variants` covers the orthogonal case where the compared type carries variants
/// entirely absent from the generated arms regardless of cfg -- pass `true` only when matching a
/// representation (typically the CORE type) that can hold more variants than the binding
/// generates arms for; pass `false` when matching the binding's own type, which by construction
/// never contains a variant absent from the arm list. ~keep
#[must_use]
pub fn enum_conversion_needs_catch_all(
    has_cfg_variants: bool,
    is_host_enum: bool,
    has_excluded_variants: bool,
) -> bool {
    has_excluded_variants || (has_cfg_variants && !is_host_enum)
}

/// [`enum_conversion_needs_catch_all`], but resolved against this binding's own configured
/// feature set instead of the raw "does any variant carry a cfg" question -- the same refinement
/// [`has_unresolved_foreign_cfg_variants`] already gives every `ConversionConfig`-driven enum
/// conversion (via [`gen_enum_from_core_to_binding_cfg`]).
///
/// For backends whose enum representation cannot route through `ConversionConfig` at all --
/// Rustler's and PHP's flat-data-enum generators build a bespoke struct-with-discriminator shape
/// instead of an enum-to-enum `From` impl, so they call `enum_conversion_needs_catch_all` with a
/// hand-rolled `has_cfg_variants` that ignores configured features entirely -- this is the direct
/// entry point into the identical resolver, so those generators land on the same verdict as every
/// other backend instead of re-deriving their own rule (alef #544). ~keep
///
/// `declaration_may_drop_variant` carries the same meaning as
/// [`ConversionConfig::declaration_drops_unreachable_foreign_variants`] -- pass `true` when the
/// generated match is over the real CORE type (matches `gen_enum_from_core_to_binding_cfg`'s
/// shape) and `false` when it is over an alef-declared BINDING type that keeps a foreign
/// cfg-gated variant unconditionally (matches `gen_enum_from_binding_to_core_cfg`'s shape for
/// every backend but NAPI). A caller whose match is over a string tag value rather than a real
/// Rust enum (e.g. rustler's and PHP's binding->core flat-enum matches) never risks
/// `unreachable_patterns` and does not need this resolver at all. ~keep
#[must_use]
pub fn enum_conversion_needs_catch_all_for_features(
    enum_def: &EnumDef,
    is_host_enum: bool,
    has_excluded_variants: bool,
    configured_features: Option<&[String]>,
    declaration_may_drop_variant: bool,
) -> bool {
    let features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let has_unresolved_cfg_variants = has_unresolved_foreign_cfg_variants(
        enum_def,
        is_host_enum,
        features_set.as_ref(),
        declaration_may_drop_variant,
    );
    enum_conversion_needs_catch_all(has_unresolved_cfg_variants, is_host_enum, has_excluded_variants)
}

/// Generate `impl From<BindingEnum> for core::Enum` (binding -> core).
pub fn gen_enum_from_binding_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_binding_to_core_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingEnum> for core::Enum` with backend-specific config.
pub fn gen_enum_from_binding_to_core_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let arms: Vec<minijinja::value::Value> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let arm = binding_to_core_match_arm_ext_cfg(
                &binding_name,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                crate::codegen::conversions::helpers::variant_emits_tuple_form(enum_def, variant)
                    && config.binding_tuple_form_for_variants,
            );
            emit_cfg_gated_arm(enum_def, variant, is_host_enum, arm, "binding_to_core")
        })
        .collect();

    // This match is over the BINDING type this backend itself declares (`binding_name` above),
    // not the real core type -- see `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s
    // doc comment for why that makes `config`'s flag (not an unconditional `true`) the correct
    // input here, unlike `gen_enum_from_core_to_binding_cfg` below. ~keep
    let configured_features = configured_features_set(config);
    let has_unresolved_cfg_variants = has_unresolved_foreign_cfg_variants(
        enum_def,
        is_host_enum,
        configured_features.as_ref(),
        config.declaration_drops_unreachable_foreign_variants,
    );
    let needs_catch_all = enum_conversion_needs_catch_all(has_unresolved_cfg_variants, is_host_enum, false);

    crate::codegen::template_env::render(
        "conversions/enum_from_binding_to_core",
        crate::alef_context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}

/// Generate `impl From<core::Enum> for BindingEnum` (core -> binding).
pub fn gen_enum_from_core_to_binding(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_core_to_binding_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<core::Enum> for BindingEnum` with backend-specific config.
pub fn gen_enum_from_core_to_binding_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let arms: Vec<minijinja::value::Value> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let arm = core_to_binding_match_arm_ext_cfg(
                &core_path,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                crate::codegen::conversions::helpers::variant_emits_tuple_form(enum_def, variant)
                    && config.binding_tuple_form_for_variants,
            );
            emit_cfg_gated_arm(enum_def, variant, is_host_enum, arm, "core_to_binding")
        })
        .collect();

    // This match is over the real CORE type (`core_path` above), a shape alef does not declare
    // and cannot influence -- `configured_features`' proof about that dependency is already the
    // complete answer regardless of what any binding declaration does, so this always passes
    // `true` here unlike `gen_enum_from_binding_to_core_cfg` above. See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    let configured_features = configured_features_set(config);
    let has_unresolved_cfg_variants =
        has_unresolved_foreign_cfg_variants(enum_def, is_host_enum, configured_features.as_ref(), true);
    let needs_catch_all = enum_conversion_needs_catch_all(
        has_unresolved_cfg_variants,
        is_host_enum,
        !enum_def.excluded_variants.is_empty(),
    );

    crate::codegen::template_env::render(
        "conversions/enum_from_core_to_binding",
        crate::alef_context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}
