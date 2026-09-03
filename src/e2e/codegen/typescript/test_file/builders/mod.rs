mod enum_members;

use super::*;
pub(in crate::e2e::codegen::typescript::test_file) use enum_members::node_enum_string_literal;
use enum_members::{
    declared_enum_member_for_prefixed, is_tagged_data_enum, node_tagged_unit_variant_literal,
    wasm_enum_bridged_as_raw_value,
};

use crate::e2e::codegen::fixture_refusal::RefusalSite;

/// Build a TypeScript expression to construct an options object.
///
/// Node: configured options types can be TypeScript interfaces — return a plain object literal
/// with a type assertion (`{ key: val } as TypeName`). No Update class or fromUpdate().
///
/// WASM: alef-backend-wasm does not emit `*Update` builder classes, so we
/// instantiate the main type directly. Every wasm-bindgen-emitted struct
/// exposes an all-optional positional constructor (`new T()`) plus per-field
/// setters, so we build the value with `new T()` followed by setter
/// assignments wrapped in an IIFE so the expression can be inlined as a
/// function argument. Nested object values follow the same pattern.
#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn ts_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    ts_builder_expression_inner(
        obj,
        type_name,
        nested_types,
        lang,
        enum_fields,
        bigint_fields,
        type_defs,
        enums,
        wasm_type_prefix,
        docs_files,
        "",
        0,
        referenced_enums,
    )
}

/// For a node-lang tagged-data enum whose matched variant wraps a single Named-type payload
/// (`enum Message { User(UserMessage), .. }` with `#[serde(tag = "role")]`), napi's `.d.ts`
/// union member nests that payload under a synthesized per-variant field
/// (`{ role: 'user'; user: UserMessage }`) rather than flattening its fields alongside the
/// tag (`{ role: 'user', content: '...' }`) — see `gen_tagged_enum_as_object`, which emits a
/// dedicated `Option<{prefix}{inner}>` field for exactly this shape (one struct-payload tuple
/// variant), keyed by `tagged_enum_binding_field_js_name` (variant/field `serde_rename`, else
/// the lower-camel-case variant name). Building the flattened wire-shape object and casting it
/// `as Message` type-checks against no union member, so `tsc` rejects it with TS2353.
///
/// Returns `None` for anything that doesn't need this treatment (unit variants, struct
/// variants, multi-field tuple variants, or a tag value with no matching variant) so the
/// caller falls back to the ordinary flatten path — still correct there, since napi keeps
/// those variants' fields flattened on the shared binding struct. ~keep
#[allow(clippy::too_many_arguments)]
fn build_node_tagged_enum_variant_literal(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_def: &EnumDef,
    nested_types: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    depth: usize,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> Option<String> {
    let tag_field = crate::backends::napi::tagged_enum_discriminant_js_name(enum_def);
    let (tag_value, payload): (&str, std::borrow::Cow<'_, serde_json::Map<String, serde_json::Value>>) =
        if let Some(content_field) = enum_def.serde_content.as_deref() {
            let serde_tag = enum_def.serde_tag.as_deref()?;
            (
                obj.get(serde_tag)?.as_str()?,
                std::borrow::Cow::Borrowed(obj.get(content_field)?.as_object()?),
            )
        } else if let Some(serde_tag) = enum_def.serde_tag.as_deref() {
            let tag_value = obj.get(serde_tag)?.as_str()?;
            let mut remaining = obj.clone();
            remaining.remove(serde_tag);
            (tag_value, std::borrow::Cow::Owned(remaining))
        } else {
            let mut entries = obj.iter();
            let (tag_value, payload) = entries.next()?;
            if entries.next().is_some() {
                return None;
            }
            (tag_value, std::borrow::Cow::Borrowed(payload.as_object()?))
        };
    let variant = enum_def.variants.iter().find(|v| {
        crate::codegen::naming::wire_variant_value(
            &v.name,
            v.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        ) == tag_value
    })?;
    if !variant.is_tuple || variant.fields.len() != 1 {
        return None;
    }
    let field = &variant.fields[0];
    let TypeRef::Named(inner_type_name) = &field.ty else {
        return None;
    };

    let payload_key = crate::backends::napi::tagged_enum_binding_field_js_name(enum_def, variant, field);

    let nested_with_cast = ts_builder_expression_inner(
        &payload,
        inner_type_name,
        nested_types,
        "node",
        enum_fields,
        bigint_fields,
        type_defs,
        enums,
        "",
        docs_files,
        pointer,
        depth + 1,
        referenced_enums,
    );
    let cast_suffix = format!(" as {inner_type_name}");
    let nested_expr = nested_with_cast.strip_suffix(&cast_suffix).unwrap_or(&nested_with_cast);

    let tag_key = js_object_key(tag_field);
    let payload_key = js_object_key(&payload_key);
    referenced_enums.insert(format!("type {type_name}"));
    Some(format!(
        "{{ {tag_key}: {}, {payload_key}: {nested_expr} }} as {type_name}",
        serde_json::to_string(tag_value).expect("enum wire values serialize as JSON strings")
    ))
}

/// For a node-lang tagged-data enum whose matched variant is struct-shaped (named fields
/// flattened directly alongside the tag, e.g. `AuthConfig::Basic { username, password }` under
/// `#[serde(tag = "type")]`), napi's `.d.ts` union member keeps that exact flattened shape
/// (`{ type: "basic"; username: string; password: string }`) — see `gen_tagged_enum_as_object`.
///
/// `node_value_expression`'s generic object path (the caller here) has no typed renderer for a
/// `Named` field whose type is an [`EnumDef`] rather than a [`TypeDef`], so it fell through to
/// treating the tag key like any other field and copied the fixture's wire string verbatim. That
/// string is a real value of the discriminant's declared type ("basic" IS a member of
/// `"basic" | "bearer" | "header"`), so nothing here is wrong in isolation -- the literal only
/// breaks once something ELSE binds it to an unannotated `const` or otherwise loses the calling
/// context that would contextually type it, at which point the tag widens to plain `string` and
/// no longer matches the union (`TS2345`). Casting the object literal itself `as <type_name>`
/// gives the WHOLE expression that exact type before it can be assigned to anything, so the tag
/// never gets a chance to widen -- regardless of how deeply nested this literal ends up, or
/// whether its caller binds it to a `const` first. ~keep
///
/// Returns `None` for anything this does not confidently know how to render (no tag key present,
/// no variant matching the tag's wire value, or a tuple-shaped variant -- that shape nests its
/// payload under a synthesized field name instead of flattening, and is
/// `build_node_tagged_enum_variant_literal`'s to handle, not this one) so the caller falls back
/// to the ordinary flatten path unchanged.
#[allow(clippy::too_many_arguments)]
fn node_tagged_struct_variant_literal(
    type_name: &str,
    enum_def: &EnumDef,
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &std::collections::HashMap<String, String>,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> Option<String> {
    if !crate::backends::napi::is_tagged_data_enum(enum_def) {
        return None;
    }
    let tag_field = crate::backends::napi::tagged_enum_discriminant_js_name(enum_def);
    let tag_value = obj.get(tag_field)?.as_str()?;
    let variant = enum_def.variants.iter().find(|v| {
        crate::codegen::naming::wire_variant_value(
            &v.name,
            v.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        ) == tag_value
    })?;
    if variant.is_tuple {
        return None;
    }
    let fields = obj
        .iter()
        .map(|(key, val)| {
            let js_key = node_field_public_key(None, key);
            let expr = if key == tag_field {
                serde_json::to_string(tag_value).expect("tag values serialize as JSON strings")
            } else {
                let nested_field_type = variant.fields.iter().find(|f| f.name == *key).map(|f| &f.ty);
                node_value_expression(
                    val,
                    key,
                    enum_fields,
                    docs_files,
                    &json_pointer_child(pointer, key),
                    nested_field_type,
                    type_defs,
                    enums,
                    None,
                    referenced_enums,
                )
            };
            format!("{js_key}: {expr}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    referenced_enums.insert(format!("type {type_name}"));
    Some(format!("{{ {fields} }} as {type_name}"))
}

/// Pre-process a JSON value so that napi-rs (node) binding can deserialize it.
///
/// The napi-rs backend exposes a tagged-data enum's discriminant under the
/// configured serde tag, defaulting to `type`.
///
/// This function walks the JSON tree and preserves a serde tag when its value
/// is a string that matches a known variant of the corresponding enum. Matching is limited to exact
/// variant matches so that plain struct fields that happen to share the
/// same key name as a serde_tag (e.g. `type: "function"` on
/// `ChatCompletionTool` where "function" is not a `ContentPart` variant)
/// are left unchanged.
pub(in crate::e2e::codegen::typescript::test_file) fn rename_napi_serde_tags_to_kind(
    value: &serde_json::Value,
    enums: &[EnumDef],
) -> serde_json::Value {
    // Build map: serde_tag_key → (set of variant serde-names, actual_tag_name).
    // Only include tagged-data enums (serde_tag present AND at least one
    // variant with fields so the binding is a flattened struct, not a plain
    // string enum).
    let mut tag_map: std::collections::HashMap<&str, (std::collections::HashSet<String>, &str)> =
        std::collections::HashMap::new();
    for e in enums {
        if let Some(tag) = e.serde_tag.as_deref()
            && e.variants.iter().any(|v| !v.fields.is_empty())
        {
            let variants: std::collections::HashSet<String> = e
                .variants
                .iter()
                .map(|v| v.serde_rename.as_deref().unwrap_or(&v.name).to_string())
                .collect();
            tag_map.insert(tag, (variants, tag));
        }
    }

    rename_napi_serde_tags_recursive(value, &tag_map)
}

fn rename_napi_serde_tags_recursive(
    value: &serde_json::Value,
    tag_map: &std::collections::HashMap<&str, (std::collections::HashSet<String>, &str)>,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (key, val) in map {
                // Preserve the original serde_tag key name when:
                //  1. the key is a known serde_tag name, AND
                //  2. the value is a string that matches a known variant of that enum.
                // The actual tag field name is already correct in the fixture; we only need
                // to validate and recurse.
                let new_key = key.clone();
                if let Some((variants, _)) = tag_map.get(key.as_str())
                    && !val.as_str().is_some_and(|s| variants.contains(s))
                {
                    // Not a valid variant value for this tag; leave as-is and recurse
                }
                new_map.insert(new_key, rename_napi_serde_tags_recursive(val, tag_map));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|item| rename_napi_serde_tags_recursive(item, tag_map))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The `EnumType.Member` expression the generated body will contain, recorded as it is produced.
///
/// Producing the reference and recording the identifier in the same step is the whole point: the
/// import block does not re-derive which enums a test file names, it imports exactly the strings
/// this function handed back, so the imported symbol and the referenced symbol are the same string
/// by construction — binding prefix included. A separately-derived import list is how a fixture
/// ended up emitting `WasmOutputFormat.Markdown` against an import line that carried either
/// nothing or the unprefixed `OutputFormat`, both of which fail as
/// `ReferenceError: WasmOutputFormat is not defined`. Every site in this module that emits an
/// enum member must go through here rather than formatting the reference itself. ~keep
fn enum_member_reference(
    enum_type: &str,
    member: &str,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    referenced_enums.insert(enum_type.to_string());
    format!("{enum_type}.{member}")
}

/// Convert a JS numeric literal expression to a BigInt-compatible literal
/// (`123n`, `-7n`) for wasm-bindgen `u64`/`i64` setters which reject Number.
/// Non-integer or non-numeric expressions are wrapped in `BigInt(...)` so the
/// runtime conversion still happens.
fn to_bigint_literal(value_expr: &str) -> String {
    let trimmed = value_expr.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return format!("{trimmed}n");
    }
    if let Some(rest) = trimmed.strip_prefix('-')
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit())
    {
        return format!("-{rest}n");
    }
    format!("BigInt({trimmed})")
}

/// Whether a field of `field_type` is one wasm-bindgen exposes as a JS `bigint` setter.
///
/// ~keep Answered from the IR, and from the *same* predicate that decides the `.d.ts` type the
/// wasm backend declares for the primitive. The only previous source of truth was the
/// hand-maintained `[crates.e2e.call].bigint_fields` list in `alef.toml`, so a `u64`/`i64` field
/// a consumer had not remembered to list got a plain `42` assigned to a `bigint` setter — a
/// TypeScript type error, and at runtime a `TypeError: Cannot convert a Number to a BigInt`.
/// The list stays honoured on top of this, since it also covers fields whose owner the IR
/// lookup cannot resolve. Gated on `wasm`: NAPI (`lang == "node"`) marshals `i64` as `number`.
///
/// `TypeRef::Duration` is included alongside the bigint primitives: `TypeMapper::duration()`
/// (default, unoverridden by [`crate::backends::wasm::type_map::WasmMapper`]) lowers every
/// `Duration` field to Rust `u64` before wasm-bindgen ever sees it, in `gen_struct`'s field-type
/// derivation and in the function-parameter input-DTO conversion alike — so a `Duration` field
/// is a `bigint` setter on the wasm boundary exactly as if the IR had declared `u64` directly. ~keep
fn wasm_bigint_field(lang: &str, field_type: Option<&crate::core::ir::TypeRef>) -> bool {
    if lang != "wasm" {
        return false;
    }
    matches!(
        field_type,
        Some(crate::core::ir::TypeRef::Primitive(prim))
            if crate::backends::wasm::gen_bindings::is_bigint_primitive(prim)
    ) || matches!(field_type, Some(crate::core::ir::TypeRef::Duration))
}

/// The BigInt literal for a fixture value assigned to a wasm-bindgen `u64`/`i64` setter.
///
/// ~keep Integers are lowered straight from the JSON number text. Routing them through
/// [`json_to_js`] first turned anything past 2^53 into `Number("9007199254740993")` — already a
/// double, so the precision a BigInt exists to preserve was gone before the `n` suffix could be
/// appended. Everything else (strings, expressions) still goes the old way: `BigInt("...")` on a
/// digit string is exact, and a non-integer number has no BigInt literal form at all.
fn bigint_value_literal(val: &serde_json::Value) -> String {
    if let serde_json::Value::Number(number) = val
        && (number.is_i64() || number.is_u64())
    {
        return format!("{number}n");
    }
    to_bigint_literal(&json_to_js(val))
}

/// ~keep wasm-bindgen declares `Vec<u64>`/`Vec<i64>` as `BigUint64Array`/`BigInt64Array`
/// (`wasm-bindgen-cli-support`'s `VectorKind` descriptor), so collection lowering must preserve
/// both the recursive IR element type and the typed-array ABI shape.
fn wasm_typed_value_expression(val: &serde_json::Value, field_type: &TypeRef) -> Option<String> {
    match field_type {
        TypeRef::Optional(inner) => wasm_typed_value_expression(val, inner),
        TypeRef::Primitive(primitive) if crate::backends::wasm::gen_bindings::is_bigint_primitive(primitive) => {
            Some(bigint_value_literal(val))
        }
        // `Duration` lowers to Rust `u64` on the wasm boundary — see `wasm_bigint_field`. ~keep
        TypeRef::Duration => Some(bigint_value_literal(val)),
        TypeRef::Vec(inner) => {
            let values = val.as_array()?;
            let constructor = match inner.as_ref() {
                TypeRef::Primitive(crate::core::ir::PrimitiveType::U64) => Some("BigUint64Array"),
                TypeRef::Primitive(crate::core::ir::PrimitiveType::I64) => Some("BigInt64Array"),
                _ => None,
            };
            let mut changed = constructor.is_some();
            let values = values
                .iter()
                .map(|value| {
                    if let Some(expression) = wasm_typed_value_expression(value, inner) {
                        changed = true;
                        expression
                    } else {
                        json_to_js(value)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            if let Some(constructor) = constructor {
                Some(format!("{constructor}.from([{values}])"))
            } else if changed {
                Some(format!("[{values}]"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Find the `FieldDef` on `owner_type` that fixture key `key` refers to, matching either the
/// field's Rust name or its wire name (`#[serde(rename = ...)]` / a container `rename_all`) —
/// a fixture may key a field either way, exactly as `refuse_undeclared_json_keys` accepts both
/// spellings as declared.
fn resolve_owner_field<'a>(owner_type: Option<&'a TypeDef>, key: &str) -> Option<&'a crate::core::ir::FieldDef> {
    let definition = owner_type?;
    definition.fields.iter().find(|field| {
        field.name == key
            || crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                definition.serde_rename_all.as_deref(),
            ) == key
    })
}

/// Render a single wasm handle-config scalar field, resolving enum members and bigint literals
/// from the field's IR type exactly as [`ts_builder_expression_inner`] does for the same field
/// on a nested object.
///
/// `owner_type` is the un-prefixed IR name of the struct that declares `key` (e.g.
/// `EngineConfig` for `WasmEngineConfig`), so the field's [`TypeRef`] can be resolved; `None`
/// when the caller has no IR-resolved owner at this point (a fixture key inside a plain nested
/// object this generator has no type mapping for), in which case only the `enum_fields`/
/// `bigint_fields` override maps are consulted, matching the owner-less call sites in
/// `node_value_expression`. ~keep
#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn wasm_scalar_value_expression(
    owner_type: Option<&str>,
    key: &str,
    camel_key: &str,
    val: &serde_json::Value,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    let owner = owner_type.and_then(|name| type_defs.iter().find(|definition| definition.name == name));
    let field_type = resolve_owner_field(owner, key).map(|field| match &field.ty {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    });

    if lang == "wasm"
        && let Some(field_type) = field_type
        && let Some(expression) = wasm_typed_value_expression(val, field_type)
    {
        return expression;
    }

    if let Some(crate::core::ir::TypeRef::Named(enum_type)) = field_type
        && enums.iter().any(|definition| definition.name == *enum_type)
        && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
        && let serde_json::Value::String(variant) = val
    {
        let member = declared_enum_member_for_prefixed(enum_type, enums, wasm_type_prefix, variant);
        let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
        return enum_member_reference(&enum_type, &member, referenced_enums);
    }

    if let Some(enum_type) = resolve_enum_type(enum_fields, owner_type, key, camel_key)
        && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
        && let serde_json::Value::String(s) = val
    {
        let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
        let member = declared_enum_member_for_prefixed(&enum_type, enums, wasm_type_prefix, s);
        return enum_member_reference(&enum_type, &member, referenced_enums);
    }

    let is_bigint =
        bigint_fields.contains(camel_key) || bigint_fields.contains(key) || wasm_bigint_field(lang, field_type);
    if is_bigint {
        return bigint_value_literal(val);
    }

    json_to_js(val)
}

/// Resolve the napi-rs / wasm-bindgen public JS field identifier for fixture key `key` on
/// `owner_type`. Both backends compute the public field name as `to_node_name(&field.name)` off
/// the Rust field (see `napi::gen_bindings::types` and `wasm::gen_bindings::types::gen_getter`),
/// never from the field's wire name, so a fixture keyed by the wire spelling (a field whose
/// `#[serde(rename = ...)]` diverges from its `#[napi(js_name = ...)]`/wasm-bindgen `js_name`,
/// e.g. `#[serde(rename = "type")]` + `#[napi(js_name = "toolType")]`) must still resolve
/// through the Rust field, not a generic camelCase of the wire key itself — camelCasing the
/// literal fixture key produced `type: "function"` where the binding required `toolType`,
/// throwing `Missing field 'toolType'` at runtime. Falls back to a generic camelCase of `key`
/// when no declared field matches (arbitrary/opaque payloads; enum tag keys are renamed
/// separately by the caller). ~keep
///
/// The result is quoted when it is not a bare JS identifier, via the same [`js_object_key`] the
/// untyped `json_to_js_camel` dump has always applied. The fallback arm reaches keys that are
/// data rather than field names — a `HashMap<String, String>` field's entries, e.g. a
/// `custom_headers` map keyed `Accept-Language` — and an unquoted `Accept-Language:` is a hard
/// JS syntax error, not a mis-typing. Both node object-literal emitters route their keys through
/// here so neither can lose the quoting the other applies. ~keep
fn node_field_public_key(owner_type: Option<&TypeDef>, key: &str) -> String {
    js_object_key(
        &resolve_owner_field(owner_type, key)
            .map(|field| crate::codegen::naming::to_node_name(&field.name))
            .unwrap_or_else(|| underscore_camel_case(key)),
    )
}

/// Refuses `obj` if it contains a key that `type_name`'s declared fields (in `type_defs`) don't
/// cover — the single check shared by every node-lang JSON-object-literal builder in this
/// module, so the snippet path (which binds the literal to a typed `const`, see
/// `typed_binding.jinja`, and so IS excess-property-checked by `tsc`) and the e2e test path
/// (which only ever `as`-casts the same literal, and so is NOT excess-property-checked) cannot
/// silently disagree about the same fixture: an undeclared key would otherwise be a compile
/// error (TS2353) in one and invisible in the other. This applies at every nesting depth —
/// `ts_builder_expression_inner`'s own object literal and `node_value_expression`'s nested
/// struct-field literals both call this, since both build a JSON-object-literal that ends up
/// under the same typed-const-vs-`as`-cast split at the top of whichever call tree it's part of.
/// A `serde_flatten` field makes the owning struct's accepted key set open-ended (it legitimately
/// re-exports its own inner field names, or an arbitrary string-keyed bag, at this JSON level),
/// so those types are exempted rather than filtered. A `type_name` absent from `type_defs`
/// (an external/opaque type) is likewise skipped — there is no declared field set to check
/// against.
///
/// ~keep An undeclared key is REFUSED, not silently dropped: this runs at generation time over a
/// fixture the maintainer wrote, so the only plausible causes are a fixture typo/stale field name,
/// a genuinely missing IR field, or (the case that actually shipped) an `options_type` that
/// resolves this argument to the wrong struct entirely — all three are bugs to fix, not values to
/// discard. A silent drop would still produce a compiling snippet/test that LOOKS like it
/// exercises the field the fixture named, which is the same "check that cannot fail" shape as
/// every other vacuous-assertion fix in this generator (see `apply_vacuous_assertion_fallback`,
/// `inert_example`) — the bug would hide instead of surfacing.
///
/// ~keep The refusal is RECORDED, not panicked. A `panic!` here aborted the entire `alef all`
/// process at exit 101 over one consumer misconfiguration, so every other backend, every later
/// crate and every later stage (README, docs, snippet validation) never ran. `fixture_refusal`
/// carries it to `E2eCodegen::generate_gated`, which turns it into this backend's own `Err` —
/// the failure mode `run_generators` already isolates, and `alef all`'s `StageFailures` already
/// reports as "continuing with the remaining stages". Generation continues to the end of this
/// backend and its output is then discarded wholesale by `run_generators`, so no refused literal
/// ever reaches disk.
fn refuse_undeclared_json_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    type_defs: &[TypeDef],
    site: RefusalSite,
) {
    let Some(definition) = type_defs.iter().find(|definition| definition.name == type_name) else {
        return;
    };
    if definition.fields.iter().any(|field| field.serde_flatten) {
        return;
    }
    // ~keep Both spellings count as declared. A fixture may key a field by its Rust name or by
    // its wire name (`#[serde(rename)]` / a container `rename_all`), and both reach here
    // unchanged — the camelCase conversion happens per key, after this check, in each caller.
    // Matching only `field.name` would abort generation on a correctly-authored fixture for any
    // renamed field, turning this guard into a worse bug than the one it catches.
    let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
    for field in &definition.fields {
        declared.insert(field.name.clone());
        declared.insert(crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        ));
    }
    // Every undeclared key, not just the first: one wrong `options_type` typically refuses
    // several keys at once, and reporting them one per regeneration is a serial debugging loop
    // against a build that takes minutes. ~keep
    for key in obj.keys().filter(|key| !declared.contains(key.as_str())) {
        crate::e2e::codegen::fixture_refusal::record(type_name, key, site.clone());
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn ts_builder_expression_inner(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    depth: usize,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    // Use a depth-indexed variable name so nested IFEs don't shadow each other.
    // Without this, `const _u = WasmOptions.default(); _u.preprocessing =
    // (() => { const _u = WasmOptions.default(); ... })()` triggers
    // oxlint `no-shadow` on every nested-options expression.
    let var = format!("_u{depth}");
    if lang == "node"
        && let Some(enum_def) = enums
            .iter()
            .find(|e| e.name == type_name && crate::backends::napi::is_tagged_data_enum(e))
        && let Some(nested_literal) = build_node_tagged_enum_variant_literal(
            obj,
            type_name,
            enum_def,
            nested_types,
            enum_fields,
            bigint_fields,
            type_defs,
            enums,
            docs_files,
            pointer,
            depth,
            referenced_enums,
        )
    {
        return nested_literal;
    }
    if lang == "node" || (lang == "wasm" && is_tagged_data_enum(type_name, enums, wasm_type_prefix)) {
        // For node: if this type itself is a tagged-data enum, rename its serde_tag
        // key to "kind". The napi-rs backend hardcodes `#[napi(js_name = "kind")]`
        // for every tagged-data enum discriminant, regardless of the original
        // `#[serde(tag = "...")]` attribute. For wasm tagged-data enums the plain
        // JS object is deserialized via serde_wasm_bindgen which reads the original
        // serde_tag name, so the rename only applies to the node language path.
        let serde_tag_for_this_type = if lang == "node" {
            let ir_name = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
            enums
                .iter()
                .find(|e| e.name == ir_name && crate::backends::napi::is_tagged_data_enum(e))
                .map(|e| {
                    let js_name = crate::backends::napi::tagged_enum_discriminant_js_name(e);
                    (e.serde_tag.as_deref().unwrap_or(js_name), js_name)
                })
        } else {
            None
        };

        let mut fields = Vec::new();
        let owner_type = type_defs.iter().find(|definition| definition.name == type_name);
        // The fixture's JSON object is the source of truth for VALUES, but not for which KEYS
        // belong on `type_name` — see `refuse_undeclared_json_keys` for why this is refused
        // rather than silently dropped, and why both the snippet and e2e paths must share this
        // one check.
        // `depth == 0` is the argument value itself; anything deeper is an object the fixture
        // nested inside it, and only the JSON pointer identifies which one. ~keep
        let site = if depth == 0 {
            RefusalSite::Argument
        } else {
            RefusalSite::Nested {
                via: format!("the fixture value at JSON pointer `{pointer}`"),
            }
        };
        refuse_undeclared_json_keys(obj, type_name, type_defs, site);
        for (key, val) in obj {
            let field_pointer = json_pointer_child(pointer, key);
            // Map the serde tag through the same resolver that declares the NAPI field.
            let js_key = if lang == "node" {
                match serde_tag_for_this_type {
                    Some((tag, js_name)) if key == tag => js_name.to_string(),
                    _ => node_field_public_key(owner_type, key),
                }
            } else {
                underscore_camel_case(key)
            };
            let field_expr = if lang == "node" {
                // Apply the napi serde_tag rename recursively into nested objects
                // and arrays so that tagged-enum elements inside arrays also get
                // their discriminant renamed to "kind".
                let preprocessed = rename_napi_serde_tags_to_kind(val, enums);
                // If the field is an enum (e.g. urlEscapeStyle, codeBlockStyle),
                // napi-rs constants are PascalCase variant names. Fixtures may
                // use the lowercase wire form (e.g. "percent"); convert it.
                let camel_key = underscore_camel_case(key);
                let enum_type = resolve_enum_type(enum_fields, Some(type_name), key, &camel_key);
                if let Some(enum_type) = enum_type {
                    if let serde_json::Value::String(s) = &preprocessed {
                        if let Some(literal) = node_tagged_unit_variant_literal(enum_type, enums, s, referenced_enums) {
                            literal
                        } else {
                            let member = declared_enum_member_for_prefixed(enum_type, enums, wasm_type_prefix, s);
                            enum_member_reference(enum_type, &member, referenced_enums)
                        }
                    } else {
                        json_to_js(&preprocessed)
                    }
                } else {
                    let field_type = resolve_owner_field(owner_type, key).map(|field| &field.ty);
                    node_value_expression(
                        &preprocessed,
                        key,
                        enum_fields,
                        docs_files,
                        &field_pointer,
                        field_type,
                        type_defs,
                        enums,
                        Some(type_name),
                        referenced_enums,
                    )
                }
            } else {
                match val {
                    serde_json::Value::Object(_) => json_to_js_camel(val),
                    _ => json_to_js(val),
                }
            };
            fields.push(format!("{js_key}: {field_expr}"));
        }
        let obj_literal = format!("{{ {} }}", fields.join(", "));
        if enums
            .iter()
            .any(|definition| definition.name == type_name && crate::backends::napi::is_untagged_data_enum(definition))
        {
            referenced_enums.insert(format!("type {type_name}"));
        }
        return format!("{obj_literal} as {type_name}");
    }

    // WASM path: construct the main type via its synthetic `default()` static
    // factory rather than `new WasmFoo()`. wasm-bindgen's `(constructor)` mirrors
    // the Rust ctor's arity, so any struct with a non-Optional field requires
    // positional args — `new WasmChatCompletionTool()` (no args) throws
    // because `tool_type` and `function` are required. The `default()` factory
    // (emitted unconditionally on every wasm wrapper that derives `Default`)
    // returns a fresh instance the test body can then drive via setters.
    let init_stmt = if type_name.starts_with("Wasm") {
        format!("const {var} = {type_name}.default();")
    } else {
        format!("const {var} = new {type_name}();")
    };

    // Build derived nested_types from the IR registry and merge with the
    // explicit overrides (explicit wins on collision).
    let derived = derive_nested_types_for_wasm(type_name, type_defs, wasm_type_prefix);
    let effective_nested_types: std::collections::HashMap<String, String> = {
        let mut m = derived;
        for (k, v) in nested_types {
            m.insert(k.clone(), v.clone());
        }
        m
    };

    let mut stmts: Vec<String> = vec![init_stmt];
    // Set when a field expression below contains an `await` (a bytes field whose fixture
    // value is a file path) so the emitted IIFE is declared `async`.
    let mut needs_async = false;
    let ir_owner_name = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
    let owner_type = type_defs.iter().find(|definition| definition.name == ir_owner_name);
    for (key, val) in obj {
        let camel_key = node_field_public_key(owner_type, key);
        let field_pointer = json_pointer_child(pointer, key);
        let field_type = resolve_owner_field(owner_type, key).map(|field| match &field.ty {
            crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        });
        if let Some(file) = docs_files.iter().find(|file| file.field == field_pointer) {
            stmts.push(
                crate::e2e::template_env::render(
                    "typescript/docs_file_assignment.jinja",
                    minijinja::context! { target => format!("{var}.{camel_key}"), path => escape_js(&file.path) },
                )
                .trim_end()
                .to_string(),
            );
            continue;
        }
        let is_bigint =
            bigint_fields.contains(&camel_key) || bigint_fields.contains(key) || wasm_bigint_field(lang, field_type);
        // A `bytes` field's fixture value may be a JSON array of numbers or a JSON string
        // (file path / inline text / base64) — ask `ts_bytes_value_expression` how to lower
        // it rather than assuming array-shaped input, matching every other TypeRef::Bytes
        // call site in this backend. See the `bytes` module docs for why this used to be two
        // independently-guessed rules. ~keep
        if matches!(field_type, Some(crate::core::ir::TypeRef::Bytes)) {
            let (expr, needs_await) = ts_bytes_value_expression(val);
            needs_async = needs_async || needs_await;
            stmts.push(format!("{var}.{camel_key} = {expr};"));
            continue;
        }
        if lang == "wasm"
            && let Some(field_type) = field_type
            && let Some(expression) = wasm_typed_value_expression(val, field_type)
        {
            stmts.push(format!("{var}.{camel_key} = {expression};"));
            continue;
        }
        if let serde_json::Value::Object(nested_obj) = val {
            if let Some(nested_type) = effective_nested_types.get(key.as_str()) {
                let nested_expr = ts_builder_expression_inner(
                    nested_obj,
                    nested_type,
                    nested_types,
                    lang,
                    enum_fields,
                    bigint_fields,
                    type_defs,
                    enums,
                    wasm_type_prefix,
                    docs_files,
                    &field_pointer,
                    depth + 1,
                    referenced_enums,
                );
                stmts.push(format!("{var}.{camel_key} = {nested_expr};"));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js_camel(val)));
            }
        } else if let serde_json::Value::Array(items) = val {
            // wasm-bindgen rejects plain object literals where it expects class
            // instances. When the array element type is a known binding class
            // (registered in `effective_nested_types`), wrap each object element
            // via the same builder-expression emitter; primitive elements pass
            // through as JS literals.
            if let Some(elem_type) = effective_nested_types.get(key.as_str()) {
                let element_exprs: Vec<String> = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        if let serde_json::Value::Object(item_obj) = item {
                            ts_builder_expression_inner(
                                item_obj,
                                elem_type,
                                nested_types,
                                lang,
                                enum_fields,
                                bigint_fields,
                                type_defs,
                                enums,
                                wasm_type_prefix,
                                docs_files,
                                &json_pointer_child(&field_pointer, &index.to_string()),
                                depth + 1,
                                &mut *referenced_enums,
                            )
                        } else {
                            json_to_js(item)
                        }
                    })
                    .collect();
                stmts.push(format!("{var}.{camel_key} = [{}];", element_exprs.join(", ")));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
            }
        } else if let Some(crate::core::ir::TypeRef::Named(enum_type)) = field_type
            && enums.iter().any(|definition| definition.name == *enum_type)
            && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
            && let serde_json::Value::String(variant) = val
        {
            let member = declared_enum_member_for_prefixed(enum_type, enums, wasm_type_prefix, variant);
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            let reference = enum_member_reference(&enum_type, &member, referenced_enums);
            stmts.push(format!("{var}.{camel_key} = {reference};"));
        } else if let Some(enum_type) = resolve_enum_type(enum_fields, Some(ir_owner_name), key, &camel_key)
            && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
        {
            // This is an enum field — generate EnumType.EnumValue.
            // Look up by both snake_case (fixture key) and camelCase (alef.toml override key
            // convention) so the alef.toml `enum_fields = { codeBlockStyle = "..." }` style
            // matches fixtures written with snake_case keys. Prefer an owner-qualified
            // match (from `infer_enum_fields`) over a bare-name one — see
            // `resolve_enum_type`.
            //
            // Prefix wasm-wrapped enums exactly as the typed branch above does:
            // the package exports `WasmExtractInputKind`, so a bare
            // `ExtractInputKind.Uri` references an undefined name.
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            if let serde_json::Value::String(s) = val {
                let member = declared_enum_member_for_prefixed(&enum_type, enums, wasm_type_prefix, s);
                let reference = enum_member_reference(&enum_type, &member, referenced_enums);
                stmts.push(format!("{var}.{camel_key} = {reference};"));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
            }
        } else if is_bigint {
            // wasm-bindgen u64/i64 setters require BigInt. Plain numeric
            // literals must be suffixed with `n`; non-literal numeric
            // values are wrapped in `BigInt(...)`.
            stmts.push(format!("{var}.{camel_key} = {};", bigint_value_literal(val)));
        } else {
            stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
        }
    }

    stmts.push(format!("return {var};"));
    let body = stmts.join(" ");
    crate::e2e::template_env::render(
        "typescript/builder_iife.jinja",
        minijinja::context! { body => body, is_async => !docs_files.is_empty() || needs_async },
    )
    .trim_end()
    .to_string()
}

/// `owner_type` is the IR name of the struct that declares `field`, when known —
/// see `resolve_enum_type` for why this disambiguates same-named fields on
/// unrelated structs.
#[allow(clippy::too_many_arguments)]
fn node_value_expression(
    value: &serde_json::Value,
    field: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    field_type: Option<&crate::core::ir::TypeRef>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    owner_type: Option<&str>,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    if let Some(file) = docs_files.iter().find(|file| file.field == pointer) {
        return crate::e2e::template_env::render(
            "typescript/docs_file_expression.jinja",
            minijinja::context! { path => escape_js(&file.path) },
        )
        .trim_end()
        .to_string();
    }
    let field_type = field_type.map(|field_type| match field_type {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    });
    if matches!(field_type, Some(crate::core::ir::TypeRef::Bytes)) {
        // Ask the shared classifier rather than assuming array-shaped input — a fixture's
        // `bytes` value is just as often a JSON string (file path / inline text / base64).
        // `Uint8Array.from(value)` unconditionally wrapped a string here, producing an
        // argument typed `string` where `Uint8Array.from` expects `Iterable<number>`. The
        // resulting expression is spliced inline into an already-`await`-containing snippet
        // body (see the `docs_file_expression.jinja` branch just above), so a file-path
        // expression's own `await` needs no further propagation here. ~keep
        let (expr, _needs_await) = ts_bytes_value_expression(value);
        return expr;
    }
    if let Some(crate::core::ir::TypeRef::Named(type_name)) = field_type
        && enums.iter().any(|definition| definition.name == *type_name)
        && let Some(variant) = value.as_str()
    {
        if let Some(literal) = node_tagged_unit_variant_literal(type_name, enums, variant, referenced_enums) {
            return literal;
        }
        let member = declared_enum_member_for_prefixed(type_name, enums, "", variant);
        return enum_member_reference(type_name, &member, referenced_enums);
    }
    let camel_field = underscore_camel_case(field);
    if let Some(enum_type) = resolve_enum_type(enum_fields, owner_type, field, &camel_field)
        && let Some(variant) = value.as_str()
    {
        if let Some(literal) = node_tagged_unit_variant_literal(enum_type, enums, variant, referenced_enums) {
            return literal;
        }
        let member = declared_enum_member_for_prefixed(enum_type, enums, "", variant);
        return enum_member_reference(enum_type, &member, referenced_enums);
    }
    if let Some(crate::core::ir::TypeRef::Named(type_name)) = field_type
        && let serde_json::Value::Object(object) = value
        && let Some(enum_def) = enums.iter().find(|definition| definition.name == *type_name)
        && let Some(literal) = node_tagged_struct_variant_literal(
            type_name,
            enum_def,
            object,
            enum_fields,
            docs_files,
            pointer,
            type_defs,
            enums,
            referenced_enums,
        )
    {
        return literal;
    }
    match value {
        serde_json::Value::Object(object) => {
            let nested_type = field_type
                .and_then(|field_type| match field_type {
                    crate::core::ir::TypeRef::Named(type_name) => Some(type_name.as_str()),
                    _ => None,
                })
                .and_then(|type_name| type_defs.iter().find(|definition| definition.name == type_name));
            // Nested struct-field object literals ("inner" fields reached via this function
            // rather than through `ts_builder_expression_inner` directly) go through the same
            // undeclared-key guard as the top-level object — see `refuse_undeclared_json_keys`.
            if let Some(definition) = nested_type {
                let via = match owner_type {
                    Some(owner) => format!("field `{field}` of `{owner}`"),
                    None => format!("field `{field}`"),
                };
                refuse_undeclared_json_keys(object, &definition.name, type_defs, RefusalSite::Nested { via });
            }
            let fields = object
                .iter()
                .map(|(name, value)| {
                    let nested_field_type = resolve_owner_field(nested_type, name).map(|field| &field.ty);
                    let js_key = node_field_public_key(nested_type, name);
                    format!(
                        "{}: {}",
                        js_key,
                        node_value_expression(
                            value,
                            name,
                            enum_fields,
                            docs_files,
                            &json_pointer_child(pointer, name),
                            nested_field_type,
                            type_defs,
                            enums,
                            nested_type.map(|definition| definition.name.as_str()),
                            &mut *referenced_enums,
                        )
                    )
                })
                .collect::<Vec<_>>();
            format!("{{ {} }}", fields.join(", "))
        }
        serde_json::Value::Array(values) => {
            let element_type = field_type.and_then(|field_type| match field_type {
                crate::core::ir::TypeRef::Vec(inner) => Some(inner.as_ref()),
                _ => None,
            });
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    // `field` is synthetic ("") for array elements, so there is no
                    // owning-type-qualified key to look up here; a nested object
                    // element's own fields resolve their owner from `element_type`
                    // inside the recursive call's `Object` branch above.
                    node_value_expression(
                        value,
                        "",
                        enum_fields,
                        docs_files,
                        &json_pointer_child(pointer, &index.to_string()),
                        element_type,
                        type_defs,
                        enums,
                        None,
                        &mut *referenced_enums,
                    )
                })
                .collect::<Vec<_>>();
            format!("[{}]", values.join(", "))
        }
        _ => json_to_js(value),
    }
}

/// The node expression for a fixture value the core IR types as `type_name`.
///
/// Exists for the one node value that had no typed renderer: a `handle` argument's config
/// object, which fell through to `json_to_js_camel`. That dump re-cases KEYS only, so a string
/// sitting at an enum-typed field stayed the fixture's *serde* wire value — and serde is not the
/// authority on what a napi binding accepts. napi re-cases variant names with `convert_case`,
/// which splits a letter-to-digit boundary serde does not (`Bm25` -> serde `"bm25"`, napi
/// `"bm_25"`; see `backends::napi::gen_bindings::enums::apply_napi_case`), so the generated suite
/// passed `contentFilter: "bm25"` to a binding whose only declared value is `'bm_25'` and every
/// such test failed at run time with `does not match any variant of enum JsContentFilterKind`.
///
/// Delegating to [`node_value_expression`] — the traversal every other typed node value already
/// runs — makes the emitted form the binding's own declared member (`ContentFilterKind.Bm25`),
/// which cannot drift from whatever string napi assigns it, and registers that member in
/// `referenced_enums` so the import block carries the identifier the body names. Re-deriving the
/// literal here instead would just move the second opinion. ~keep
pub(in crate::e2e::codegen::typescript::test_file) fn node_typed_value_expression(
    value: &serde_json::Value,
    type_name: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    let field_type = crate::core::ir::TypeRef::Named(type_name.to_string());
    node_value_expression(
        value,
        "",
        enum_fields,
        &[],
        "",
        Some(&field_type),
        type_defs,
        enums,
        None,
        referenced_enums,
    )
}

fn json_pointer_child(pointer: &str, field: &str) -> String {
    let field = field.replace('~', "~0").replace('/', "~1");
    format!("{pointer}/{field}")
}

#[cfg(test)]
mod bigint_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wire_name_tests;
