//! Which enum members the WASM/NAPI binding actually declares, and what a generated snippet is
//! therefore allowed to reference.
//!
//! Split out of `builders/mod.rs` (which was approaching the 800-line split threshold) as a
//! self-contained concept: everything here answers one question — does this enum have a
//! `Foo.Variant` member, and if so what is it spelled? Every answer is delegated, never
//! re-derived locally: representation to `backends::wasm::gen_bindings::enums`, wire-value
//! mapping to `codegen::serde_enum_repr`.

use super::*;

/// True when `type_name` (possibly with a `Wasm` binding-prefix) names an IR enum the WASM
/// backend emits as a discriminator *struct* rather than a C-style `#[wasm_bindgen]` enum.
///
/// WASM bindings expose such enums via field setters of type
/// `JsValue`/`Option<JsValue>`, which `serde_wasm_bindgen::from_value` then
/// deserializes from a plain JS object. Wrapping the value with the
/// per-variant `default()` factory + setters produces an opaque
/// wasm-bindgen wrapper class whose own-property table is empty — serde
/// then fails to read the discriminator. The e2e builder must emit a plain
/// JS object literal for these instead.
///
/// The predicate is the backend's own, not a local restatement of it. This file previously
/// required `serde_tag.is_some()`, which is strictly narrower than what
/// `wasm::gen_bindings::enums::gen_enum` actually acts on: an externally tagged enum (no
/// `#[serde(tag/content/untagged)]` at all) that carries a payload variant is ALSO emitted as a
/// struct, because a C-style `#[wasm_bindgen]` enum cannot hold the payload. For those enums the
/// two surfaces disagreed — the binding declared `pub struct WasmOutputFormat { .. }` while this
/// generator still emitted `WasmOutputFormat.Markdown`, so every generated snippet touching such
/// an enum failed to compile with `TS2339: Property 'Markdown' does not exist on type 'typeof
/// WasmOutputFormat'`. Delegating removes the second opinion instead of re-syncing it. ~keep
pub(super) fn is_tagged_data_enum(type_name: &str, enums: &[EnumDef], wasm_type_prefix: &str) -> bool {
    let stripped = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
    enums
        .iter()
        .any(|e| e.name == stripped && crate::backends::wasm::gen_bindings::enums::is_tagged_data_enum(e))
}

/// True when `enum_name` (already unprefixed IR name) is an enum the binding routes through raw
/// serde value passthrough instead of a nominal type — mirrors the `is_json_passthrough_data_enum`
/// gate both `.d.ts` dispatchers use. On the wire such an enum serializes as the bare payload of
/// whichever variant matched, not a named member — a string-typed instance is the raw JS value
/// itself. Treating it as `EnumType.Variant` turned an empty string into `WasmEmbeddingInput.`
/// (missing member, a syntax error).
///
/// This asks the *combined* predicate, not the container-level `is_untagged_data_enum` alone. An
/// enum whose data variant carries its own `#[serde(untagged)]` — `OutputFormat`'s `Custom(String)`
/// — is declared as a flat string-literal union with no value binding at all, so
/// `OutputFormat.Markdown` is a type-used-as-value error rather than a missing member. Naming only
/// the container-level predicate here is what let the fixture and snippet emitters keep emitting
/// member references after the `.d.ts` emitter had already been fixed. ~keep
fn is_node_untagged_data_enum(enum_name: &str, enums: &[EnumDef]) -> bool {
    enums
        .iter()
        .any(|e| e.name == enum_name && crate::backends::napi::is_json_passthrough_data_enum(e))
}

fn is_wasm_untagged_data_enum(enum_name: &str, enums: &[EnumDef]) -> bool {
    enums
        .iter()
        .any(|e| e.name == enum_name && crate::backends::wasm::gen_bindings::enums::is_json_passthrough_data_enum(e))
}

/// True when the WASM binding exposes `enum_name` as a raw serde value rather than as a C-style
/// `#[wasm_bindgen]` enum, so there is no `WasmFoo.Variant` member to reference.
///
/// Both shapes it covers — the discriminator struct ([`is_tagged_data_enum`]) and the
/// `JsValue`/`serde_wasm_bindgen` bridge ([`is_untagged_data_enum`]) — take the raw serde value
/// at the field site, so a caller that gets `true` here must emit the fixture value verbatim.
///
/// An enum name this generator has no IR entry for (an `alef.toml` `enum_fields` override naming
/// a type that never entered the IR) is reported `false`: there is no declaration to contradict,
/// and reporting `true` would silently drop a member reference that used to be emitted. ~keep
pub(super) fn wasm_enum_bridged_as_raw_value(enum_name: &str, enums: &[EnumDef], wasm_type_prefix: &str) -> bool {
    let stripped = enum_name.strip_prefix(wasm_type_prefix).unwrap_or(enum_name);
    is_tagged_data_enum(enum_name, enums, wasm_type_prefix) || is_wasm_untagged_data_enum(stripped, enums)
}

/// The member identifier the binding declares for the variant a fixture named by its wire value,
/// e.g. `("markdown", OutputFormat)` -> `Markdown`.
///
/// `#[wasm_bindgen]` (and napi's `#[napi]`) publish a C-style enum's members under the Rust
/// variant identifier verbatim, so the only correct answer is that identifier — never a re-cased
/// copy of the wire value. Returns `None` when no declared variant carries that wire value, so
/// callers can refuse an undeclared value rather than inventing a member that does not
/// exist. See [`crate::codegen::serde_enum_repr::variant_name_for_wire`], which owns the
/// wire-value-to-variant mapping and delegates to `naming::wire_variant_value`. ~keep
fn declared_enum_member(enum_name: &str, enums: &[EnumDef], wire_value: &str) -> Option<String> {
    let enum_def = enums.iter().find(|e| e.name == enum_name)?;
    crate::codegen::serde_enum_repr::variant_name_for_wire(enum_def, wire_value).map(str::to_string)
}

/// [`declared_enum_member`] keyed by an already binding-prefixed type name (`WasmOutputFormat`),
/// falling back to the historical `to_upper_camel_case` re-casing only for a type this generator
/// has no IR enum for — an `enum_fields` override in `alef.toml` can name an enum that never
/// entered the IR, and that path has no declaration to agree with either way.
pub(super) fn declared_enum_member_for_prefixed(
    prefixed_enum: &str,
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    wire_value: &str,
) -> Option<String> {
    let stripped = prefixed_enum.strip_prefix(wasm_type_prefix).unwrap_or(prefixed_enum);
    if let Some(member) = declared_enum_member(stripped, enums, wire_value)
        .or_else(|| declared_enum_member(prefixed_enum, enums, wire_value))
    {
        return Some(member);
    }
    if let Some(definition) = enums
        .iter()
        .find(|definition| definition.name == stripped || definition.name == prefixed_enum)
    {
        crate::e2e::codegen::fixture_refusal::record_enum_value(&definition.name, wire_value);
        return None;
    }
    Some(wire_value.to_upper_camel_case())
}

pub(super) fn node_tagged_unit_variant_literal(
    enum_name: &str,
    enums: &[EnumDef],
    wire_value: &str,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> Option<String> {
    let enum_def = enums
        .iter()
        .find(|definition| definition.name == enum_name && crate::backends::napi::is_tagged_data_enum(definition))?;
    let variant_name = crate::codegen::serde_enum_repr::variant_name_for_wire(enum_def, wire_value)?;
    let variant = enum_def
        .variants
        .iter()
        .find(|variant| variant.name == variant_name && variant.fields.is_empty())?;
    let wire_value = crate::codegen::naming::wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    );
    let tag = crate::backends::napi::tagged_enum_discriminant_js_name(enum_def);
    let tag = js_object_key(tag);
    let quoted = serde_json::to_string(&wire_value).expect("enum wire values serialize as JSON strings");
    referenced_enums.insert(format!("type {enum_name}"));
    Some(format!("{{ {tag}: {quoted} }} as {enum_name}"))
}

pub(in crate::e2e::codegen::typescript::test_file) fn node_enum_string_literal(
    enum_name: &str,
    enums: &[EnumDef],
    wire_value: &str,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    if is_node_untagged_data_enum(enum_name, enums) {
        return serde_json::to_string(wire_value).expect("enum payloads serialize as JSON strings");
    }
    if let Some(literal) = node_tagged_unit_variant_literal(enum_name, enums, wire_value, referenced_enums) {
        return literal;
    }
    let member = declared_enum_member_for_prefixed(enum_name, enums, "", wire_value);
    enum_member_reference(enum_name, member.as_deref(), referenced_enums)
}
