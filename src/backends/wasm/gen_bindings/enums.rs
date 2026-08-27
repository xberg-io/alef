//! WASM enum code generation.

use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::field_init::struct_field_init;
use crate::codegen::naming::{to_node_name, wire_variant_value};
use crate::codegen::type_mapper::TypeMapper;

use super::functions::emit_rustdoc;

/// True if this enum is a serde-tagged data enum (`#[serde(tag = "...")]` with variant fields).
/// These are emitted as a flat wasm-bindgen struct with a discriminator field and the union of
/// all variant fields (each made optional) — analogous to the NAPI tagged-enum-as-object path.
///
/// Also true for a default-representation (externally tagged, no
/// `#[serde(tag/content/untagged)]`) enum that carries a payload variant (e.g. `Custom(String)`):
/// a plain `#[wasm_bindgen]` C-style enum can only hold unit variants, so without this the
/// payload was silently dropped (`Custom = 1` with no field). Route it through the same
/// discriminator-struct emitter as an explicitly tagged enum, defaulting the discriminant field
/// name to "type" like `gen_tagged_enum_as_struct` already does. ~keep
pub(super) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    has_data_variants && (enum_def.serde_tag.is_some() || !enum_def.serde_untagged)
}

/// True if this enum is a serde-untagged data enum (`#[serde(untagged)]` with at least one
/// variant carrying fields), e.g. `enum EmbeddingInput { Single(String), Multiple(Vec<String>) }`.
///
/// Unlike `is_tagged_data_enum`, there is no tag to key a struct-with-discriminator
/// representation on — the wire shape *is* whichever variant's payload serialized bare. A
/// fieldless `#[wasm_bindgen]` C-style enum cannot carry that payload either, so `gen_enum` is
/// never called for these: `mod.rs` redirects every field of this type straight to `JsValue`
/// (via `type_overrides`) and bridges it through `serde_wasm_bindgen` at the field site, the same
/// mechanism already used for `is_tagged_data_enum` fields. ~keep
pub(super) fn is_untagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_untagged && enum_def.variants.iter().any(|v| !v.fields.is_empty())
}

/// Detect every [`is_untagged_data_enum`] in `api` and default its `type_overrides` entry to
/// `JsValue`, mirroring how `mod.rs` already redirects `untagged_union_text_types` to `String`.
/// `or_insert_with` leaves an explicit consumer `type_overrides` entry untouched. The returned
/// names also drive the field-level `JsValue`/`serde_wasm_bindgen` bridging in `types.rs` and
/// `crate::codegen::conversions`, via `mod.rs`'s `jsvalue_bridged_enum_names`. ~keep
pub(super) fn register_untagged_data_enum_overrides(
    api: &ApiSurface,
    type_overrides: &mut std::collections::HashMap<String, String>,
) -> AHashSet<String> {
    let names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    for name in &names {
        type_overrides
            .entry(name.clone())
            .or_insert_with(|| "JsValue".to_string());
    }
    names
}

/// Escape a Rust reserved keyword by prepending the raw-identifier prefix.
/// Used when a field/tag name collides with a Rust keyword (e.g. `type`).
fn escape_rust_keyword(name: &str) -> String {
    const RUST_KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
        "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
        "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract",
        "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
    ];
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// Compute the set of field names that appear in multiple variants with different Named types.
///
/// When the same positional or named field (e.g. `_0`) carries a different inner type per
/// variant (e.g. `SystemMessage` vs `UserMessage`), the binding struct cannot represent it
/// as a single concrete WASM type. Instead the field is stored as `JsValue` and converted
/// via `serde_wasm_bindgen` per-variant in the From impls.
fn mixed_type_fields(enum_def: &EnumDef) -> std::collections::BTreeSet<String> {
    let mut field_types: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(field.name.clone()).or_default().insert(n.clone());
            }
        }
    }
    field_types
        .into_iter()
        .filter(|(_, types)| types.len() > 1)
        .map(|(name, _)| name)
        .collect()
}

/// Returns true when `field` was originally `Vec<(K, V)>` but was sanitized to `Vec<String>`.
///
/// These fields must be stored as `Option<JsValue>` in the wasm struct so that the JS wire
/// representation (`[[k, v], ...]`) round-trips correctly through `serde_wasm_bindgen` rather
/// than collapsing to a flat `Vec<String>`.
fn is_sanitized_tuple_vec(field: &FieldDef) -> bool {
    field.sanitized && field.original_type.as_deref().is_some_and(|s| s.starts_with("Vec<("))
}

/// Returns true when `field` was originally a fixed-size array of tuples (`[(K, V); N]`)
/// but was sanitized to `String` (JSON-encoded).
///
/// Like `is_sanitized_tuple_vec`, these fields must be stored as `Option<JsValue>` so that
/// serde round-trips the structured JS value through `serde_wasm_bindgen` rather than treating
/// the field as a plain string.
fn is_sanitized_fixed_tuple_array(field: &FieldDef) -> bool {
    field.sanitized
        && field
            .original_type
            .as_deref()
            .is_some_and(|s| s.starts_with("[(") && s.contains(");"))
}

/// Append `.map(Box::new)` to a `.map(Into::into)` conversion when the core field is
/// `Box<T>` (or, combined with the caller's `Option` handling, `Option<Box<T>>`). Mirrors the
/// box-wrap handling already applied to boxed plain-struct fields by the shared codegen helpers
/// in `src/codegen/conversions`.
fn box_wrap_map_into(base: String, is_boxed: bool) -> String {
    if is_boxed {
        format!("{base}.map(Box::new)")
    } else {
        base
    }
}

fn tagged_enum_binding_to_core_expr(
    field_ident: &str,
    field_ty: &TypeRef,
    field_optional: bool,
    is_boxed: bool,
) -> String {
    if field_optional {
        return match field_ty {
            TypeRef::Named(_) => box_wrap_map_into(format!("val.{field_ident}.clone().map(Into::into)"), is_boxed),
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => box_wrap_map_into(format!("val.{field_ident}.clone().map(Into::into)"), is_boxed),
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        },
        TypeRef::Named(_) => {
            let base = box_wrap_map_into(format!("val.{field_ident}.clone().map(Into::into)"), is_boxed);
            format!("{base}.unwrap_or_default()")
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("val.{field_ident}.clone().unwrap_or_default().into_iter().map(Into::into).collect()")
        }
        TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
        TypeRef::Map(_, _) => format!(
            "val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok()).unwrap_or_default()"
        ),
        _ => format!("val.{field_ident}.clone().unwrap_or_default()"),
    }
}

/// Deref a boxed `.into()` conversion (bare `Box<T>` field). Mirrors the box-unwrap handling
/// already applied to boxed plain-struct fields by the shared codegen helpers in
/// `src/codegen/conversions` (e.g. `bedrock: val.bedrock.map(|v| (*v).into())`).
fn box_unwrap_into(local: &str, is_boxed: bool) -> String {
    if is_boxed {
        format!("(*{local}).into()")
    } else {
        format!("{local}.into()")
    }
}

/// Deref a boxed `.map(Into::into)` conversion (`Option<Box<T>>` field).
fn box_unwrap_map_into(local: &str, is_boxed: bool) -> String {
    if is_boxed {
        format!("{local}.map(|v| (*v).into())")
    } else {
        format!("{local}.map(Into::into)")
    }
}

fn tagged_enum_core_to_binding_expr(
    field_ident: &str,
    local: &str,
    field_ty: &TypeRef,
    field_optional: bool,
    is_boxed: bool,
) -> String {
    if field_optional {
        return match field_ty {
            TypeRef::Named(_) => format!(
                "                {field_ident}: {}",
                box_unwrap_map_into(local, is_boxed)
            ),
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
            TypeRef::Map(_, _) => {
                format!(
                    "                {field_ident}: {local}.as_ref().and_then(|m| serde_wasm_bindgen::to_value(m).ok())"
                )
            }
            _ => format!("                {}", struct_field_init(field_ident, local)),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!(
                "                {field_ident}: {}",
                box_unwrap_map_into(local, is_boxed)
            ),
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
            TypeRef::Map(_, _) => {
                format!(
                    "                {field_ident}: {local}.as_ref().and_then(|m| serde_wasm_bindgen::to_value(m).ok())"
                )
            }
            _ => format!("                {}", struct_field_init(field_ident, local)),
        },
        TypeRef::Named(_) => format!(
            "                {field_ident}: Some({})",
            box_unwrap_into(local, is_boxed)
        ),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("                {field_ident}: Some({local}.into_iter().map(Into::into).collect())")
        }
        TypeRef::Path => format!("                {field_ident}: Some({local}.to_string_lossy().to_string())"),
        TypeRef::Map(_, _) => format!("                {field_ident}: serde_wasm_bindgen::to_value(&{local}).ok()"),
        _ => format!("                {field_ident}: Some({local})"),
    }
}

/// Compute the serde wire tag value for a variant — what JS supplies in the `type` field.
pub(super) fn variant_tag_value(
    variant_name: &str,
    serde_rename: Option<&str>,
    serde_rename_all: Option<&str>,
) -> String {
    wire_variant_value(variant_name, serde_rename, serde_rename_all)
}

/// Generate a wasm-bindgen tagged-enum representation as a flat `#[wasm_bindgen]` struct.
///
/// Serde-tagged data enums (e.g. `#[serde(tag = "type")] enum AuthConfig { Basic { ... }, ...}`)
/// cannot be represented as wasm-bindgen C-style enums because that loses all variant fields.
/// Instead, we emit a struct with:
///  - a `type: String` discriminator (named after `serde_tag`, camelCased for JS)
///  - the union of every variant's fields, each `Option<T>` (so any single instance is valid)
///  - getters/setters for each field, plus a `default()` static factory
///
/// This mirrors the NAPI backend's `gen_tagged_enum_as_object` path. The corresponding
/// `From<Wasm{Enum}> for core::{Enum}` (and reverse) impls are emitted by
/// `gen_tagged_enum_binding_to_core` / `gen_tagged_enum_core_to_binding`.
pub(super) fn gen_tagged_enum_as_struct(enum_def: &EnumDef, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);
    let tag_js_name = to_node_name(tag_field);
    let mapper = WasmMapper::new(std::collections::HashMap::new(), prefix.to_string());

    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        lines.push(doc);
    }

    lines.push("#[wasm_bindgen]".to_string());
    lines.push("#[derive(Clone, Default)]".to_string());
    lines.push(format!("pub struct {js_name} {{"));
    lines.push(format!("    pub(crate) {tag_field_ident}: String,"));

    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut field_entries: Vec<(String, String)> = Vec::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if !seen.insert(field.name.clone()) {
                continue;
            }
            let field_ty = if mixed.contains(&field.name) || tuple_vec_fields.contains(&field.name) {
                "Option<JsValue>".to_string()
            } else {
                let mapped = mapper.map_type(&field.ty);
                if matches!(&field.ty, TypeRef::Optional(_)) {
                    mapped
                } else {
                    format!("Option<{mapped}>")
                }
            };
            field_entries.push((field.name.clone(), field_ty.clone()));
            let escaped = escape_rust_keyword(&field.name);
            lines.push(format!("    pub(crate) {escaped}: {field_ty},"));
        }
    }
    lines.push("}".to_string());

    lines.push(String::new());
    lines.push("#[wasm_bindgen]".to_string());
    lines.push(format!("impl {js_name} {{"));
    lines.push("    #[wasm_bindgen(constructor)]".to_string());
    lines.push(format!("    pub fn new() -> {js_name} {{ Self::default() }}"));
    lines.push(String::new());
    lines.push("    #[wasm_bindgen]".to_string());
    lines.push("    #[allow(clippy::should_implement_trait)]".to_string());
    lines.push(format!(
        "    pub fn default() -> {js_name} {{ <Self as ::core::default::Default>::default() }}"
    ));

    lines.push(String::new());
    lines.push(format!("    #[wasm_bindgen(getter, js_name = \"{tag_js_name}\")]"));
    lines.push(format!(
        "    pub fn {tag_field_ident}(&self) -> String {{ self.{tag_field_ident}.clone() }}"
    ));
    let setter_ident = format!("set_{tag_field}");
    let setter_ident_escaped = escape_rust_keyword(&setter_ident);
    lines.push(format!("    #[wasm_bindgen(setter, js_name = \"{tag_js_name}\")]"));
    lines.push(format!(
        "    pub fn {setter_ident_escaped}(&mut self, value: String) {{ self.{tag_field_ident} = value; }}"
    ));

    for (name, ty) in &field_entries {
        let js_name_for_field = to_node_name(name);
        let field_name = name.as_str();
        let rust_getter_ident = if field_name.starts_with('_')
            && field_name.len() > 1
            && field_name[1..].chars().all(|c| c.is_ascii_digit())
        {
            format!("field_{}", &field_name[1..])
        } else {
            escape_rust_keyword(field_name)
        };
        let rust_setter_ident = format!("set_{rust_getter_ident}");
        let struct_field_ident = escape_rust_keyword(field_name);
        lines.push(String::new());
        lines.push(format!(
            "    #[wasm_bindgen(getter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {rust_getter_ident}(&self) -> {ty} {{ self.{struct_field_ident}.clone() }}"
        ));
        lines.push(format!(
            "    #[wasm_bindgen(setter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {rust_setter_ident}(&mut self, value: {ty}) {{ self.{struct_field_ident} = value; }}"
        ));
    }
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate `From<Wasm{Enum}> for core::{Enum}` for a tagged-struct enum representation.
///
/// JS sets `obj.type = "basic"` and the variant-specific fields; this maps `obj.type` to the
/// matching core variant and reads the relevant fields. Missing fields fall back to
/// `Default::default()` so the conversion never panics for malformed input.
/// Return the first `::` segment of a Rust path (the crate name), normalizing hyphens to
/// underscores to match how Cargo exposes crate names in Rust code.
fn path_crate_segment(path: &str) -> &str {
    path.split("::").next().unwrap_or("")
}

/// True when `rust_path` resolves to a crate other than `core_import`.
/// Such types are not in the WASM consumer's Cargo dependency graph, so emitting
/// `serde_wasm_bindgen::from_value::<{rust_path}>()` would produce E0433.
fn is_external_crate_type(rust_path: &str, core_import: &str) -> bool {
    let crate_seg = path_crate_segment(rust_path);
    let core_seg = core_import.replace('-', "_");
    crate_seg.replace('-', "_") != core_seg
}

/// Render the binding→core expression for a field the tagged-enum struct stores as
/// `Option<JsValue>` because its type differs between variants (see `mixed_type_fields`).
///
/// Shared by the tuple- and named-field variant arms so the two cannot disagree about the
/// field's binding representation. `gen_tagged_enum_as_struct` applies the `mixed` degradation
/// to *every* variant field, tuple or named; `JsValue` implements no `From<CoreType>` in either
/// direction, so the `.into()` that `tagged_enum_binding_to_core_expr` writes for a
/// `TypeRef::Named` is an `E0277` against such a field — serde is the only bridge. ~keep
fn mixed_field_binding_to_core_expr(field: &FieldDef, field_ident: &str, core_import: &str) -> String {
    let is_external = field
        .type_rust_path
        .as_deref()
        .is_some_and(|path| is_external_crate_type(path, core_import));
    let expr = if is_external {
        // The type is not in the wasm crate's dependency graph, so naming it in a
        // `from_value::<T>()` turbofish would be an E0433. ~keep
        "Default::default()".to_string()
    } else {
        let core_inner = match field.type_rust_path.as_deref() {
            Some(path) => path.replace('-', "_"),
            None => match &field.ty {
                TypeRef::Named(n) => format!("{core_import}::{n}"),
                _ => "serde_json::Value".to_string(),
            },
        };
        format!(
            "val.{field_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{core_inner}>(v.clone()).ok()).unwrap_or_default()"
        )
    };
    if field.is_boxed {
        format!("Box::new({expr})")
    } else {
        expr
    }
}

/// Whether `variant`'s `#[cfg(...)]` is safe to re-emit verbatim on its match arm.
///
/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; this WASM crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(feature = "...")]` is
/// an `unexpected cfg condition value` error. Such a variant is dropped entirely instead --
/// named and counted via `tracing::warn!`, not silently -- mirroring
/// `codegen::conversions::enums::emit_cfg_gated_arm`. A host-owned cfg keeps its arm and its
/// `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
fn wasm_tagged_variant_kept(enum_def: &EnumDef, variant: &EnumVariant, is_host_enum: bool, direction: &str) -> bool {
    if variant.cfg.is_none() || is_host_enum {
        return true;
    }
    tracing::warn!(
        enum_name = %enum_def.name,
        enum_rust_path = %enum_def.rust_path,
        variant_name = %variant.name,
        cfg = variant.cfg.as_deref().unwrap_or_default(),
        direction = direction,
        "dropping WASM tagged-enum conversion match arm for a foreign-crate variant behind a \
         #[cfg(...)] this crate cannot declare as a Cargo feature; the variant is unreachable \
         from this conversion"
    );
    false
}

pub(super) fn gen_tagged_enum_binding_to_core(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

    let mut lines = vec![];
    lines.push(format!("impl From<{binding_name}> for {core_path} {{"));
    lines.push(format!("    fn from(val: {binding_name}) -> Self {{"));
    lines.push(format!("        match val.{tag_field_ident}.as_str() {{"));
    for variant in &enum_def.variants {
        if !wasm_tagged_variant_kept(enum_def, variant, is_host_enum, "binding_to_core") {
            continue;
        }
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            lines.push(format!("            \"{tag_value}\" => Self::{},", variant.name));
        } else if variant.is_tuple {
            let args: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        mixed_field_binding_to_core_expr(f, &f_ident, core_import)
                    } else if tuple_vec_fields.contains(&f.name) {
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()"
                        )
                    } else {
                        tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional, f.is_boxed)
                    }
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{}({}),",
                variant.name,
                args.join(", ")
            ));
        } else {
            let inits: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        format!("{}: {}", f.name, mixed_field_binding_to_core_expr(f, &f_ident, core_import))
                    } else if tuple_vec_fields.contains(&f.name) {
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "{}: val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()",
                            f.name
                        )
                    } else {
                        format!(
                            "{}: {}",
                            f.name,
                            tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional, f.is_boxed)
                        )
                    }
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{} {{ {} }},",
                variant.name,
                inits.join(", ")
            ));
        }
    }
    // Prefer the first variant with no cfg gate as the unconditional `_ =>` fallback: a
    // cfg-gated variant (host-owned or foreign) may not exist in every build, so it cannot
    // safely stand in as the always-available default. Falls back to the very first variant
    // only when every variant carries a cfg. ~keep
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.cfg.is_none())
        .or_else(|| enum_def.variants.first());
    if let Some(first) = default_variant {
        if first.fields.is_empty() {
            lines.push(format!("            _ => Self::{},", first.name));
        } else if first.is_tuple {
            let args: Vec<String> = first.fields.iter().map(|_| "Default::default()".to_string()).collect();
            lines.push(format!("            _ => Self::{}({}),", first.name, args.join(", ")));
        } else {
            let defaults: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            lines.push(format!(
                "            _ => Self::{} {{ {} }},",
                first.name,
                defaults.join(", ")
            ));
        }
    }
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate `From<core::{Enum}> for Wasm{Enum}` for a tagged-struct enum representation.
pub(super) fn gen_tagged_enum_core_to_binding(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

    let mut all_field_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            all_field_names.insert(field.name.clone());
        }
    }

    let mut lines = vec![];
    lines.push(format!("impl From<{core_path}> for {binding_name} {{"));
    lines.push(format!("    fn from(val: {core_path}) -> Self {{"));
    lines.push("        match val {".to_string());
    for variant in &enum_def.variants {
        if !wasm_tagged_variant_kept(enum_def, variant, is_host_enum, "core_to_binding") {
            continue;
        }
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let variant_field_names: std::collections::BTreeSet<String> =
            variant.fields.iter().map(|f| f.name.clone()).collect();
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                inits.push(format!("                {n_ident}: None"));
            }
            lines.push(format!("            {core_path}::{} => Self {{", variant.name));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else if variant.is_tuple {
            let local_names: Vec<String> = variant
                .fields
                .iter()
                .enumerate()
                .map(|(i, _)| format!("field{i}"))
                .collect();
            let destructure = local_names.join(", ");
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    let pos = variant.fields.iter().position(|f| &f.name == name).unwrap();
                    let local = &local_names[pos];
                    let init = if mixed.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if tuple_vec_fields.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(&n_ident, local, &field.ty, field.optional, field.is_boxed)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{}({}) => Self {{",
                variant.name, destructure
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else {
            let destructure_names: Vec<String> = variant.fields.iter().map(|f| escape_rust_keyword(&f.name)).collect();
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    // `mixed` degrades the struct field to `Option<JsValue>` for named-field
                    // variants exactly as it does for tuple variants, so this arm must take the
                    // same serde bridge — `tagged_enum_core_to_binding_expr` would write
                    // `Some({local}.into())`, an E0277 against `JsValue`. ~keep
                    let init = if mixed.contains(name) || tuple_vec_fields.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{n_ident}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(&n_ident, &n_ident, &field.ty, field.optional, field.is_boxed)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{} {{ {} }} => Self {{",
                variant.name,
                destructure_names.join(", ")
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        }
    }
    lines.push(
        crate::backends::wasm::template_env::render("tagged_enum_unmapped_core_arm", minijinja::context! {})
            .trim_end()
            .to_string(),
    );
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate a wasm-bindgen enum definition.
///
/// `configured_features` is REQUIRED (not `Option`, unlike `codegen::conversions`'
/// `enum_variant_declaration`): `#[wasm_bindgen]` parses an enum's variants from the raw
/// `syn::ItemEnum` token stream before cfg-stripping ever runs, so it unconditionally generates
/// code (`IntoWasmAbi`, `TryFromJsValue`, ...) referencing every variant it saw -- attaching
/// `#[cfg(...)]` to only the variant's declaration line (which is exactly right for napi, whose
/// macro sees the already-cfg'd item) leaves that generated code dangling once the compiler
/// drops the variant, producing `E0599: no variant ... found` pointing AT the declaration that
/// still, syntactically, names it. See rustwasm/wasm-bindgen#2058 and
/// `codegen::conversions::enum_variant_declaration_without_cfg_attribute`'s doc comment for the
/// confirmation trail. This backend must therefore decide, at generation time, whether a
/// cfg-gated variant exists AT ALL for this binding and emit it fully present or fully absent --
/// never a `#[cfg(...)]` attribute on the variant itself. ~keep
pub(super) fn gen_enum(
    enum_def: &EnumDef,
    prefix: &str,
    core_import: &str,
    configured_features: &std::collections::HashSet<&str>,
) -> String {
    if is_tagged_data_enum(enum_def) {
        return gen_tagged_enum_as_struct(enum_def, prefix);
    }

    let js_name = format!("{prefix}{}", enum_def.name);
    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        lines.push(doc);
    }
    lines.extend([
        "#[wasm_bindgen]".to_string(),
        "#[derive(Clone, Copy, PartialEq, Eq)]".to_string(),
        format!("pub enum {} {{", js_name),
    ]);

    // The SAME authority the conversion arms consult
    // (`codegen::conversions::enum_variant_declaration_without_cfg_attribute`) decides which
    // variants this wrapper declares -- fully present or fully absent, never conditionally --
    // keeping this declaration, the `to_api_str`/`from_api_str` matches below, and the `From`
    // impls' match arms from ever disagreeing about which variants exist. ~keep
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let declared_variants: Vec<&EnumVariant> = enum_def
        .variants
        .iter()
        .filter(|variant| {
            matches!(
                crate::codegen::conversions::enum_variant_declaration_without_cfg_attribute(
                    variant,
                    is_host_enum,
                    configured_features,
                ),
                crate::codegen::conversions::VariantDeclaration::Keep { .. }
            )
        })
        .collect();

    for (idx, variant) in declared_variants.iter().enumerate() {
        lines.push(format!("    {} = {},", variant.name, idx));
    }

    lines.push("}".to_string());

    let default_variant = declared_variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| declared_variants.first());
    if let Some(dv) = default_variant {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {} {{", js_name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", dv.name));
        lines.push("}".to_string());
    }

    if !declared_variants.is_empty() {
        lines.push(String::new());
        lines.push(format!("impl {} {{", js_name));
        lines.push(
            "    /// Returns the serde wire string for this variant (e.g. `\"stop\"`, `\"tool_calls\"`).".to_string(),
        );
        lines.push("    pub fn to_api_str(self) -> &'static str {".to_string());
        lines.push("        match self {".to_string());
        for variant in &declared_variants {
            let wire = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            lines.push(format!("            Self::{} => \"{}\",", variant.name, wire));
        }
        lines.push("        }".to_string());
        lines.push("    }".to_string());

        lines.push(String::new());
        lines.push(
            "    /// Parses a serde wire string and returns the corresponding variant, or None if unrecognized."
                .to_string(),
        );
        lines.push("    pub fn from_api_str(s: &str) -> Option<Self> {".to_string());
        lines.push("        match s {".to_string());
        for variant in &declared_variants {
            let wire = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            lines.push(format!("            \"{}\" => Some(Self::{}),", wire, variant.name));
        }
        lines.push("            _ => None,".to_string());
        lines.push("        }".to_string());
        lines.push("    }".to_string());

        lines.push("}".to_string());
    }

    lines.join("\n")
}
#[cfg(test)]
mod cfg_gate_tests;
#[cfg(test)]
mod tests;
