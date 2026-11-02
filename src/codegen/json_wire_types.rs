//! Shared JSON re-casing "wire types" for fully-flattened internally-tagged enums.
//!
//! [`crate::backends::napi::gen_bindings::enums::is_fully_flattened_internal_enum`] and its wasm
//! analogue identify internally-tagged enums (`#[serde(tag = "...")]`) whose every data-carrying
//! variant is a newtype around a resolvable named struct. Such an enum has no single flat
//! napi/wasm struct representation -- different variants can want the same field name at
//! incompatible types -- so both backends bridge it through `serde_json::Value` instead. Handed
//! `serde_json::to_value`/`from_value` directly against the core type, that passthrough is
//! snake_case all the way down, because that is what the core's own serde derive produces.
//!
//! This module emits a pair of mirror Rust types per struct/enum reachable from such an enum's
//! payloads -- one that accepts core's snake_case and emits camelCase (`Family::Out`, core ->
//! JS), one that accepts JS's camelCase and emits core's snake_case (`Family::In`, JS -> core) --
//! so a binding can re-case the JSON at the boundary with no hand-written field-by-field
//! conversion logic. See [`JsonWireTypes`] for the public entry point.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::codegen::naming::{apply_serde_rename_all, wire_field_name, wire_variant_value};
use crate::codegen::serde_enum_repr::tagged_object_tag_key;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::core::keywords::{is_valid_rust_ident_chars, rust_raw_ident};

/// Which direction of re-casing a wire type mirror belongs to.
///
/// `Out` types deserialize core's own wire shape (snake_case, via `alias`) and serialize
/// camelCase (via a container-level `#[serde(rename_all = "camelCase")]`). `In` types are the
/// mirror image: they deserialize camelCase (via `alias`) and serialize core's snake_case shape
/// with no container-level rename, so the result round-trips straight into
/// `serde_json::from_value::<core::T>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Out,
    In,
}

/// Registers Rust "wire type" mirrors for a fully-flattened internally-tagged enum and every
/// struct reachable from its payloads, so a binding backend can re-case the JSON passthrough
/// path from core's snake_case to JavaScript's camelCase (and back) with no hand-written
/// per-field conversion code.
///
/// `declarations()` never observes non-determinism: every internal collection is a `BTreeMap`/
/// `BTreeSet`, and a type is visited (and therefore declared) exactly once per [`Family`] no
/// matter how many enums or fields reference it, which also makes self-referential and mutually
/// recursive payload graphs terminate instead of recursing forever.
pub struct JsonWireTypes<'a> {
    api: &'a ApiSurface,
    prefix: String,
    /// Name of the shared `skip_serializing_if` predicate this instance emits (folds in
    /// `prefix` for the same collision-avoidance reason as every emitted type name).
    absent_fn_name: String,
    retag_fn_name: String,
    /// Declarations keyed by their final emitted Rust name, so the map itself is the
    /// deduplication authority and `declarations()` can just return the (already name-sorted)
    /// values.
    decls: BTreeMap<String, String>,
    /// Guards recursion: `(Family, original IR name)` -- structs and enums cannot share a name
    /// within the same Rust crate, so one flat set suffices for both.
    visited: BTreeSet<(Family, String)>,
}

impl<'a> JsonWireTypes<'a> {
    /// `prefix` is the binding's type prefix, e.g. `"Js"` or `"Wasm"`; it is folded into every
    /// emitted name (including the shared absent-predicate helper) so napi- and wasm-generated
    /// wire types can never collide if both are ever generated into one crate.
    pub fn new(api: &'a ApiSurface, prefix: &str) -> Self {
        // ~keep The backend prefix is a TYPE-name prefix ("Js", "Wasm"), so interpolating it raw
        // produced `__alef_wire_absent_Js`, which `clippy -D warnings` rejects as non_snake_case in
        // the crate we emit. Snake-case it: these are function names, not type names.
        let absent_fn_name = format!("__alef_wire_absent_{}", crate::codegen::naming::pascal_to_snake(prefix));
        let mut decls = BTreeMap::new();
        // ~keep The declared TypeScript surface these enums bridge to has no `null` state for
        // an optional field (`readonly sheetCount?: number;` -- absent or present, never null),
        // so a field that is absent OR explicitly null on the way in must be OMITTED on the way
        // out rather than round-tripped as `null`. `Option<Option<T>>` still tells the two
        // states apart during deserialization (needed so "absent" and "null" don't collapse
        // into each other going IN), but both serialize as "skip". A plain `Option::is_none`
        // `skip_serializing_if` would only catch the outer `None` (truly absent) and would
        // still emit `"field":null` for `Some(None)` (present-as-null), which is exactly the
        // shape the declared `.d.ts` forbids. Emitted once, up front, independent of whether
        // anything is ever registered, so `declarations()` is trivially deterministic.
        let helper_signature = format!("fn {absent_fn_name}<T>(value: &Option<Option<T>>) -> bool {{");
        let helper_body = "    matches!(value, None | Some(None))\n}";
        decls.insert(absent_fn_name.clone(), format!("{helper_signature}\n{helper_body}"));
        // ~keep Serde's internally-tagged representation uses ONE tag key for both directions, so
        // no single wire enum can read core's `format_type` and write JavaScript's `formatType`.
        // Each family therefore declares the tag key of the side it DESERIALIZES, and the tag is
        // renamed once, at the top level only, on the way out. Top level only is the point: a
        // blind recursive rename would also rewrite genuine data keys (a `BTreeMap<String, _>`
        // whose keys are document-supplied), which is exactly the corruption this avoids.
        let retag_fn_name = format!("__alef_wire_retag_{}", crate::codegen::naming::pascal_to_snake(prefix));
        let retag_src = [
            format!("fn {retag_fn_name}(mut value: serde_json::Value, from: &str, to: &str) -> serde_json::Value {{"),
            "    if from != to {".to_string(),
            "        if let Some(object) = value.as_object_mut() {".to_string(),
            "            if let Some(tag) = object.remove(from) {".to_string(),
            "                object.insert(to.to_string(), tag);".to_string(),
            "            }".to_string(),
            "        }".to_string(),
            "    }".to_string(),
            "    value".to_string(),
            "}".to_string(),
        ]
        .join("\n");
        decls.insert(retag_fn_name.clone(), retag_src);
        Self {
            api,
            prefix: prefix.to_string(),
            absent_fn_name,
            retag_fn_name,
            decls,
            visited: BTreeSet::new(),
        }
    }

    /// Register an enum and everything reachable from its payloads. Returns
    /// `(out_enum_type_name, in_enum_type_name)`.
    pub fn register_enum(&mut self, enum_def: &EnumDef) -> (String, String) {
        let out_name = self.register_enum_family(enum_def, Family::Out);
        let in_name = self.register_enum_family(enum_def, Family::In);
        (out_name, in_name)
    }

    /// Rust expression producing the JavaScript-facing `serde_json::Value` for a core value.
    /// `core_expr` must evaluate to something borrowable as the core enum.
    pub fn core_to_js_value_expr(&self, enum_def: &EnumDef, core_expr: &str) -> String {
        let out_name = self.wire_name(&enum_def.name, Family::Out);
        out_pipeline_expr(
            core_expr,
            &self.retag_fn_name,
            &out_name,
            core_tag_key(enum_def),
            &js_tag_key(enum_def),
        )
    }

    /// Rust expression producing the core value from a JavaScript-facing `serde_json::Value`.
    /// `core_path` is the fully qualified core type path.
    pub fn js_value_to_core_expr(&self, enum_def: &EnumDef, js_expr: &str, core_path: &str) -> String {
        let in_name = self.wire_name(&enum_def.name, Family::In);
        let inner = in_pipeline_expr(
            js_expr,
            &self.retag_fn_name,
            &in_name,
            core_tag_key(enum_def),
            &js_tag_key(enum_def),
        );
        format!("serde_json::from_value::<{core_path}>({inner}).unwrap_or_default()")
    }

    pub(crate) fn js_value_to_core_result_expr(&self, enum_def: &EnumDef, js_expr: &str, core_path: &str) -> String {
        let in_name = self.wire_name(&enum_def.name, Family::In);
        let from = escape_rust_string_literal(&js_tag_key(enum_def));
        let to = escape_rust_string_literal(core_tag_key(enum_def));
        format!(
            "serde_json::from_value::<{in_name}>({js_expr}).and_then(serde_json::to_value).map(|value| {}(value, \"{from}\", \"{to}\")).and_then(serde_json::from_value::<{core_path}>)",
            self.retag_fn_name
        )
    }

    /// The name of the shared top-level-tag retag helper this instance declares (see
    /// [`JsonWireTypes::new`]). Exposed so a backend whose own JS boundary is not
    /// `serde_json::Value` (e.g. wasm's `JsValue`, via `serde_wasm_bindgen`) can rebuild
    /// [`out_pipeline_expr`]/[`in_pipeline_expr`] itself from plain string pieces at a call site
    /// that only has a field name and type in hand, not a live `&EnumDef` -- see
    /// `codegen::conversions::core_to_binding::fields`'s wasm camelCase-recasing helpers for the
    /// concrete caller. ~keep
    pub fn retag_fn_name(&self) -> &str {
        &self.retag_fn_name
    }

    /// Every emitted Rust declaration, in deterministic (sorted-by-name) order.
    pub fn declarations(&self) -> Vec<String> {
        self.decls.values().cloned().collect()
    }

    fn wire_name(&self, base: &str, family: Family) -> String {
        match family {
            Family::Out => format!("__AlefWireOut{}{base}", self.prefix),
            Family::In => format!("__AlefWireIn{}{base}", self.prefix),
        }
    }

    fn struct_by_name(&self, name: &str) -> Option<&'a TypeDef> {
        self.api.types.iter().find(|t| t.name == name)
    }

    /// Maps a field/variant-payload `TypeRef` to the Rust type text a wire mirror should use,
    /// registering (and recursing into) any nested struct it names along the way.
    ///
    /// Per the design this module implements: a `Named` type that resolves to a struct in
    /// `api.types` becomes the corresponding wire struct name; a `Named` type that resolves to
    /// an enum, or does not resolve at all, becomes `serde_json::Value` (recasing an enum's own
    /// wire shape is out of scope here -- it keeps whatever shape its own serde impl produces);
    /// `Vec`/`Optional`/`Map` recurse structurally; everything else becomes `serde_json::Value`,
    /// since every leaf round-trips losslessly through `serde_json::Value` regardless of its
    /// concrete Rust type.
    fn wire_type_for(&mut self, ty: &TypeRef, family: Family) -> String {
        match ty {
            TypeRef::Named(name) => match self.struct_by_name(name) {
                Some(type_def) => self.register_struct(type_def, family),
                None => "serde_json::Value".to_string(),
            },
            TypeRef::Vec(inner) => format!("Vec<{}>", self.wire_type_for(inner, family)),
            TypeRef::Optional(inner) => self.wire_type_for(inner, family),
            TypeRef::Map(_, value) => {
                format!(
                    "std::collections::BTreeMap<String, {}>",
                    self.wire_type_for(value, family)
                )
            }
            _ => "serde_json::Value".to_string(),
        }
    }

    fn register_struct(&mut self, type_def: &'a TypeDef, family: Family) -> String {
        let wire_name = self.wire_name(&type_def.name, family);
        if !self.visited.insert((family, type_def.name.clone())) {
            return wire_name;
        }

        let mut field_decls = Vec::with_capacity(type_def.fields.len());
        for field in &type_def.fields {
            if field.serde_skip {
                continue;
            }
            if field.serde_flatten {
                let inner_ty = self.wire_type_for(&field.ty, family);
                let ident = rust_raw_ident(&field.name);
                field_decls.push(format!("    #[serde(flatten)]\n    pub {ident}: {inner_ty},"));
                continue;
            }

            let (ident_text, alias) = field_rust_and_alias(field, type_def.serde_rename_all.as_deref(), family);
            let rust_ident = rust_raw_ident(&ident_text);
            let inner_ty = if field.serde_with.is_some() {
                // ~keep A `#[serde(with = "...")]`/`serialize_with` field's wire shape comes
                // from a hand-written codec, not from serde's derive -- alef cannot know what
                // an arbitrary codec emits, so the safe, generic shape is `serde_json::Value`.
                // Its NAME handling (ident/alias) is unaffected and still follows the same
                // rule as every other field.
                "serde_json::Value".to_string()
            } else {
                self.wire_type_for(&field.ty, family)
            };
            let inner_ty = maybe_box(inner_ty, field.is_boxed);
            field_decls.push(self.render_optional_field(&rust_ident, alias.as_deref(), &inner_ty));
        }

        let mut src = String::new();
        src.push_str("#[derive(Default, serde::Serialize, serde::Deserialize)]\n");
        if family == Family::Out {
            src.push_str("#[serde(rename_all = \"camelCase\")]\n");
        }
        src.push_str(&format!("pub struct {wire_name} {{\n"));
        for line in &field_decls {
            src.push_str(line);
            src.push('\n');
        }
        src.push('}');

        self.decls.insert(wire_name.clone(), src);
        wire_name
    }

    fn render_optional_field(&self, rust_ident: &str, alias: Option<&str>, inner_ty: &str) -> String {
        let mut attr = format!("#[serde(default, skip_serializing_if = \"{}\"", self.absent_fn_name);
        if let Some(alias) = alias {
            attr.push_str(&format!(", alias = \"{}\"", escape_rust_string_literal(alias)));
        }
        attr.push_str(")]");
        format!("    {attr}\n    pub {rust_ident}: Option<Option<{inner_ty}>>,")
    }

    fn register_enum_family(&mut self, enum_def: &EnumDef, family: Family) -> String {
        let wire_name = self.wire_name(&enum_def.name, family);
        if !self.visited.insert((family, enum_def.name.clone())) {
            return wire_name;
        }

        // ~keep Each family declares the tag key of the side it DESERIALIZES: the OUT family is
        // fed core's JSON (snake_case tag), the IN family is fed JavaScript's (camelCase tag).
        // The rename to the other side happens once, at the top level, via the retag helper --
        // see `core_to_js_value_expr` / `js_value_to_core_expr`.
        let tag_key = match family {
            Family::Out => core_tag_key(enum_def).to_string(),
            Family::In => js_tag_key(enum_def),
        };
        // ~keep A "fully flattened internal enum" is defined by its DATA-carrying variants all
        // flattening; it may still legitimately mix in unit variants (e.g. an `Unknown`
        // fallback), which serialize as the bare tag with no payload key at all
        // (`{"format_type":"unknown"}`). Serde's internally-tagged deserializer rejects a tag
        // value with no matching variant ("unknown variant"), so a wire enum that only declared
        // the data-carrying variants would fail to parse a perfectly real payload. Every variant
        // is therefore mirrored, not only the ones with data.
        let has_default = enum_def.has_default && enum_def.variants.iter().any(|v| v.is_default && v.fields.is_empty());

        let mut variant_lines = Vec::with_capacity(enum_def.variants.len());
        for variant in &enum_def.variants {
            variant_lines.push(self.render_enum_variant(enum_def, variant, family, has_default));
        }

        let derive = if has_default {
            "#[derive(Default, serde::Serialize, serde::Deserialize)]"
        } else {
            "#[derive(serde::Serialize, serde::Deserialize)]"
        };
        let mut src = String::new();
        src.push_str(derive);
        src.push('\n');
        src.push_str(&format!(
            "#[serde(tag = \"{}\")]\n",
            escape_rust_string_literal(&tag_key)
        ));
        src.push_str(&format!("pub enum {wire_name} {{\n"));
        for line in &variant_lines {
            src.push_str(line);
            src.push('\n');
        }
        src.push('}');

        self.decls.insert(wire_name.clone(), src);
        wire_name
    }

    fn render_enum_variant(
        &mut self,
        enum_def: &EnumDef,
        variant: &EnumVariant,
        family: Family,
        has_default: bool,
    ) -> String {
        let tag_value = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let variant_ident = rust_raw_ident(&variant.name);
        let is_default_unit = has_default && variant.is_default && variant.fields.is_empty();
        let default_attr = if is_default_unit { "    #[default]\n" } else { "" };
        let rename_attr = format!("    #[serde(rename = \"{}\")]", escape_rust_string_literal(&tag_value));

        if variant.fields.is_empty() {
            return format!("{default_attr}{rename_attr}\n    {variant_ident},");
        }

        // A `Vec<(K, V)>` sanitized field is not a "struct" fields payload, but no
        // fully-flattened-internal-enum variant this module is contracted to handle should hit
        // this branch (its single field's `TypeRef` still resolves through `wire_type_for` like
        // any other, or falls back to `serde_json::Value` if it does not fit). A variant that
        // does not fit the expected single-Named-field newtype shape (defensive: alef's own
        // `is_fully_flattened_internal_enum` guarantees every DATA variant does, but this
        // module must not panic if a caller ever hands it one that does not) degrades to a bare
        // `serde_json::Value` payload rather than panicking.
        let payload_ty = match variant.fields.as_slice() {
            [field] => maybe_box(self.wire_type_for(&field.ty, family), field.is_boxed),
            _ => "serde_json::Value".to_string(),
        };
        format!("{default_attr}{rename_attr}\n    {variant_ident}({payload_ty}),")
    }
}

fn maybe_box(inner_ty: String, is_boxed: bool) -> String {
    if is_boxed { format!("Box<{inner_ty}>") } else { inner_ty }
}

/// Builds the `serde_json::Value`-producing retag+wire-encode formula shared by
/// [`JsonWireTypes::core_to_js_value_expr`] (napi's `serde_json::Value` boundary) and wasm's
/// field-level camelCase recasing (`codegen::conversions::core_to_binding::fields`), which
/// rebuilds this exact formula from plain string pieces (`retag_fn_name`/`out_wire_type` from
/// [`JsonWireTypes::register_enum`] and [`JsonWireTypes::retag_fn_name`], `core_tag_key`/
/// `js_tag_key` from the free functions below) rather than calling this method through a live
/// `&JsonWireTypes`/`&EnumDef`: registration happens once per enum at the codegen top level,
/// where those live, while the field-conversion functions run per struct field with only a
/// `TypeRef` and field name in hand. `core_expr` must evaluate to something borrowable as the
/// core enum. ~keep
pub(crate) fn out_pipeline_expr(
    core_expr: &str,
    retag_fn_name: &str,
    out_wire_type: &str,
    core_tag_key: &str,
    js_tag_key: &str,
) -> String {
    let from = escape_rust_string_literal(core_tag_key);
    let to = escape_rust_string_literal(js_tag_key);
    format!(
        "{retag_fn_name}(serde_json::to_value(&{core_expr}).ok()\
         .and_then(|raw| serde_json::from_value::<{out_wire_type}>(raw).ok())\
         .and_then(|wire| serde_json::to_value(wire).ok())\
         .unwrap_or_default(), \"{from}\", \"{to}\")"
    )
}

/// The retag+wire-decode half of [`JsonWireTypes::js_value_to_core_expr`], factored out for the
/// same reason as [`out_pipeline_expr`] -- see that function's doc comment. `js_expr` must
/// evaluate to a `serde_json::Value`. Unlike `js_value_to_core_expr`, this stops short of the
/// final `serde_json::from_value::<core_path>(...)` decode into the real core type: a wasm field
/// call site has no `core_path` to spell out and instead relies on the surrounding struct-literal
/// field type to infer it, the same way this file's sibling `untagged_data_enum_names` conversion
/// branch already does. ~keep
pub(crate) fn in_pipeline_expr(
    js_expr: &str,
    retag_fn_name: &str,
    in_wire_type: &str,
    core_tag_key: &str,
    js_tag_key: &str,
) -> String {
    let from = escape_rust_string_literal(js_tag_key);
    let to = escape_rust_string_literal(core_tag_key);
    format!(
        "{retag_fn_name}(serde_json::from_value::<{in_wire_type}>({js_expr}).ok()\
         .and_then(|wire| serde_json::to_value(wire).ok())\
         .unwrap_or_default(), \"{from}\", \"{to}\")"
    )
}

fn escape_rust_string_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Falls back to `field.name` when `core_wire_name` is not itself a syntactically valid Rust
/// identifier (e.g. an exotic `#[serde(rename = "self-harm/intent")]`). Known limitation: in
/// that fallback case the field's wire name and its Rust identifier text diverge, so the
/// container-level rename_all-derived primary name computed from the identifier no longer
/// matches the real wire name -- see [`field_rust_and_alias`]'s doc comment.
fn sanitized_field_identifier(core_wire_name: &str, fallback: &str) -> String {
    if is_valid_rust_ident_chars(core_wire_name) {
        core_wire_name.to_string()
    } else {
        fallback.to_string()
    }
}

/// Computes the Rust identifier text (pre-keyword-escaping) and the `alias` this field needs for
/// `family`, given the ACTUAL wire name core's own serde derive produces for it.
///
/// The core trick that lets both families avoid a per-field `#[serde(rename = "...")]`: the
/// Rust field identifier in *both* wire mirrors is set to the field's real core wire name
/// (`core_wire_name`, not necessarily `field.name` -- see [`sanitized_field_identifier`] for the
/// one exception), rather than to the core struct's own Rust field name. That means:
///
/// - `Family::In` sets no container-level `rename_all`, so serde's default (identity) naming
///   already makes the primary wire name equal to `core_wire_name`, which is exactly the name
///   core's own `Deserialize` expects back -- no explicit `rename` needed. The `alias` accepts
///   the camelCase spelling JavaScript actually sends.
/// - `Family::Out` sets `#[serde(rename_all = "camelCase")]`, so the primary (serialize) name
///   becomes `camelCase(core_wire_name)` for free. The `alias` accepts `core_wire_name` itself,
///   which is what `serde_json::to_value(core_value)` actually produces on deserialize-in.
///
/// A single-word field (`core_wire_name` already camelCase-identical, e.g. `"producer"`) needs
/// no alias at all -- emitting one identical to the primary name is a serde derive compile
/// error, so it is omitted whenever the two coincide. ~keep
fn field_rust_and_alias(
    field: &FieldDef,
    container_rename_all: Option<&str>,
    family: Family,
) -> (String, Option<String>) {
    let core_wire_name = wire_field_name(&field.name, field.serde_rename.as_deref(), container_rename_all);
    let ident_text = sanitized_field_identifier(&core_wire_name, &field.name);
    let camel = apply_serde_rename_all(&ident_text, Some("camelCase"));

    let alias = match family {
        Family::Out => (camel != ident_text).then_some(ident_text.clone()),
        Family::In => (camel != ident_text).then_some(camel),
    };
    (ident_text, alias)
}

#[cfg(test)]
mod tests;

/// The tag key as serde writes it on the CORE wire (what `#[serde(tag = "...")]` declares).
/// `pub(crate)` so a backend outside this module can compute the same piece for
/// [`out_pipeline_expr`]/[`in_pipeline_expr`] -- see those functions' doc comments.
pub(crate) fn core_tag_key(enum_def: &EnumDef) -> &str {
    tagged_object_tag_key(enum_def)
}

/// The tag key as JavaScript sees it. The discriminant is an identifier a caller reads and
/// compares, so it follows the host idiom like every other member -- leaving it snake_case
/// produces `format.format_type === "excel" ? ... .sheetCount`, one object with two conventions.
/// Tag VALUES (`"excel"`, `"fiction_book"`) are data and are never re-cased. ~keep
pub(crate) fn js_tag_key(enum_def: &EnumDef) -> String {
    use heck::ToLowerCamelCase;
    core_tag_key(enum_def).to_lower_camel_case()
}
