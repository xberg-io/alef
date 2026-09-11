use crate::backends::swift::gen_rust_crate::type_bridge::field_needs_json_bridge;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::e2e::escape::escape_swift as escape_swift_str;
use crate::e2e::field_access::SwiftFirstClassMap;
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

/// Returns true when `element_type` names a scalar Rust/Swift element type.
///
/// Scalar element types describe `Vec<T>` Rust parameters that the swift-bridge
/// surface exposes as native Swift `[T]` arrays — these can be constructed from
/// a Swift array literal without any opaque-type intermediate. Object element
/// types (everything else) require an `options_via` configuration to construct.
pub(super) fn is_scalar_element_type(element_type: Option<&str>) -> bool {
    matches!(
        element_type.map(str::trim),
        Some(
            "String"
                | "str"
                | "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "usize"
                | "f32"
                | "f64",
        )
    )
}

pub(super) fn from_json_helper_for_arg(arg: &crate::e2e::config::ArgMapping, options_type: Option<&str>) -> String {
    let type_name = arg
        .element_type
        .as_deref()
        .or(options_type)
        .unwrap_or(arg.name.as_str());
    format!("{}FromJson", type_name.to_lower_camel_case())
}

pub(super) fn json_to_swift(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_swift(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_swift).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_swift(&json_str))
        }
    }
}

/// When comparing numeric values in Swift, integer and floating-point literals
/// should not be wrapped in type constructors. Swift's type inference will infer
/// the correct type based on the field expression's return type.
///
/// Booleans ("true"/"false") are never wrapped — they are Swift `Bool` literals
/// and should never be cast to numeric types.
///
/// Floating-point literals should never be wrapped, as they may compare against
/// fields that return `Double` or other floating-point types.
pub(super) fn swift_numeric_literal_cast(_field_expr: &str, numeric_literal: &str) -> String {
    // Never wrap booleans.
    if numeric_literal == "true" || numeric_literal == "false" {
        return numeric_literal.to_string();
    }

    // Don't wrap any numeric literals — Swift's type inference will handle it.
    // This avoids type mismatches when fields return specific types like UInt16,
    // UInt32, Int, etc. The comparison operator and field type will guide inference.
    numeric_literal.to_string()
}

/// Escape a string for embedding in a Swift double-quoted string literal.
pub(super) fn escape_swift(s: &str) -> String {
    escape_swift_str(s)
}

/// Resolve the IR type name backing this call's result.
///
/// Lookup order mirrors PHP's `derive_root_type` for `[crates.e2e.calls.*]`
/// configs: any of `c, csharp, java, kotlin, go, php` overrides may carry a
/// `result_type = "ChatCompletionResponse"` field. The first non-empty value
/// wins. These overrides are language-agnostic IR type names — they were
/// originally added for the C/C# backends and other backends piggy-back on them
/// because the IR names are shared across every binding.
///
/// Returns `None` when no override sets `result_type`; the renderer then falls
/// back to the workspace-default heuristic in `SwiftFirstClassMap` (which
/// defaults to property access — the right call for first-class result types
/// like `FileObject` but wrong for opaque types like `ChatCompletionResponse`).
pub(super) fn swift_call_result_type(call_config: &crate::core::config::e2e::CallConfig) -> Option<String> {
    const LOOKUP_LANGS: &[&str] = &["c", "csharp", "java", "kotlin", "go", "php"];
    for lang in LOOKUP_LANGS {
        if let Some(o) = call_config.overrides.get(*lang)
            && let Some(rt) = o.result_type.as_deref()
            && !rt.is_empty()
        {
            return Some(rt.to_string());
        }
    }
    None
}

pub(super) fn swift_client_factory_call(factory: &str, api_key: &str, base_url: &str) -> String {
    format!("let _client = try {factory}(apiKey: {api_key}, baseUrl: {base_url})")
}

pub(super) fn resolve_streaming_adapter<'a>(
    config: &'a ResolvedCrateConfig,
    call_config: &crate::core::config::e2e::CallConfig,
    function_name: &str,
    client_factory: Option<&str>,
) -> Option<&'a crate::core::config::AdapterConfig> {
    let owner_type = client_factory.filter(|value| value.chars().next().is_some_and(char::is_uppercase));
    config
        .adapters
        .iter()
        .find(|adapter| {
            matches!(adapter.pattern, AdapterPattern::Streaming)
                && adapter.name.to_lower_camel_case() == function_name
                && owner_type.is_none_or(|owner| adapter.owner_type.as_deref() == Some(owner))
        })
        .or_else(|| {
            call_config.overrides.values().find_map(|override_config| {
                override_config.result_type.as_deref().and_then(|result_type| {
                    config.adapters.iter().find(|adapter| {
                        matches!(adapter.pattern, AdapterPattern::Streaming)
                            && adapter.name.to_lower_camel_case() == function_name
                            && adapter.item_type.as_deref() == Some(result_type)
                    })
                })
            })
        })
}

/// Whether a field type keeps its owner in the Swift first-class (Codable struct) set.
///
/// Delegates to the binding emitter's own predicate rather than restating it. This was a
/// hand-copied mirror, and a mirror of an emit decision is exactly the shape that produced the
/// Ruby `_0` defect: one side was taught a new rule and the other was not, so the generated
/// assertions addressed a shape the binding had stopped emitting. Here the failure would be a
/// Swift compile error in the generated suite -- method-call navigation (`.metaTags()`) rendered
/// against a struct whose `metaTags` is a `public let`. ~keep
pub(super) fn swift_first_class_field_supported(
    ty: &crate::core::ir::TypeRef,
    known_dto_names: &HashSet<String>,
) -> bool {
    crate::backends::swift::gen_bindings::dto::first_class_field_supported(ty, known_dto_names)
}

/// Build the per-type Swift first-class/opaque classification map used by
/// `render_swift_with_first_class_map`.
///
/// A TypeDef is treated as first-class (Codable Swift struct → property access)
/// when it is not opaque, has serde derives, has at least one field, and every
/// binding field is supported by `swift_first_class_field_supported` against the
/// current first-class set. All other public types end up as typealiases to
/// opaque `RustBridge.X` classes whose fields are swift-bridge methods
/// (`.id()`, `.status()`).
///
/// Mirrors the fixed-point iteration in `alef-backend-swift::gen_bindings.rs`
/// (lines 100-130). Without the fixed point, a type like `TranscriptionResponse`
/// that holds `Option<Vec<TranscriptionSegment>>` would be wrongly classified
/// opaque, causing the renderer to emit `.text()` against a first-class struct
/// whose `text` is a `public let` property.
///
/// `field_types` records the next-type that each Named field traverses into,
/// so the renderer can advance its current-type cursor through nested
/// `data[0].id` style paths.
///
/// `call_config` is used to resolve the explicit `result_type` override via
/// `swift_call_result_type()`. When available, this override takes precedence
/// over the fallback heuristic of finding a TypeDef that contains all
/// `result_fields` (which fails when result_fields is workspace-global across
/// many call sites with different result types like ChatCompletionResponse,
/// EmbeddingResponse, ModelsListResponse, etc.).
pub(super) fn build_swift_first_class_map(
    type_defs: &[crate::core::ir::TypeDef],
    enum_defs: &[crate::core::ir::EnumDef],
    e2e_config: &crate::e2e::config::E2eConfig,
    call_config: &crate::core::config::e2e::CallConfig,
) -> SwiftFirstClassMap {
    use crate::core::ir::TypeRef;
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut vec_field_names: HashSet<String> = HashSet::new();
    // ~keep Field names whose swift-bridge getter collapses the whole field to one
    // `RustString` of JSON. Such a leaf has no `.count` AND no elements, so neither a count
    // suffix nor an index step can be spelled against it — `assertions.rs` reads this set to
    // refuse both, which is why it records every JSON-bridged field (maps included) and not
    // only the `Vec`-shaped ones.
    let mut json_bridged_field_names: HashSet<String> = HashSet::new();
    fn inner_named(ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::Named(n) => Some(n.clone()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    fn is_vec_ty(ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Vec(_) => true,
            TypeRef::Optional(inner) => is_vec_ty(inner),
            _ => false,
        }
    }
    // Seed with unit serde enum names — Codable on the Swift side and can appear
    // as leaf fields on struct DTOs. Also seed data-variant enums (tagged + untagged)
    // that have any fields, matching gen_bindings.rs which seeds both unit + data enums.
    // This ensures containing structs (like ChatCompletionResponse holding Choice holding
    // AssistantContent) are classified as first-class when all their fields are supported.
    let unit_serde_enum_names: HashSet<String> = enum_defs
        .iter()
        .filter(|e| e.has_serde && e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.clone())
        .collect();

    let data_variant_enum_names: HashSet<String> = enum_defs
        .iter()
        .filter(|e| e.has_serde && e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.clone())
        .collect();

    let mut known_dto_names: HashSet<String> = unit_serde_enum_names.clone();
    known_dto_names.extend(data_variant_enum_names.iter().cloned());

    // Candidate struct DTOs: non-opaque, has_serde, non-empty fields.
    // Trait types and binding-excluded types are skipped (matches backend semantics
    // — note backend further filters via `exclude_types`, which we don't have here,
    // but accepting a superset is safe: types not actually emitted simply never
    // appear in path-access chains).
    let candidates: Vec<&crate::core::ir::TypeDef> = type_defs
        .iter()
        .filter(|td| !td.is_trait && !td.is_opaque && td.has_serde && !td.fields.is_empty())
        .collect();

    loop {
        let prev = known_dto_names.len();
        for td in &candidates {
            if known_dto_names.contains(&td.name) {
                continue;
            }
            let all_supported = td
                .fields
                .iter()
                .filter(|f| !f.binding_excluded)
                .all(|f| swift_first_class_field_supported(&f.ty, &known_dto_names));
            if all_supported {
                known_dto_names.insert(td.name.clone());
            }
        }
        if known_dto_names.len() == prev {
            break;
        }
    }

    // The first-class set on SwiftFirstClassMap conceptually represents structs
    // accessed via property syntax. Unit enums never appear as the *owner* of a
    // chain segment (they are leaves), but including them is harmless since
    // `advance()` never returns them as a current_type for further traversal.
    let first_class_types: HashSet<String> = candidates
        .iter()
        .filter(|td| known_dto_names.contains(&td.name))
        .map(|td| td.name.clone())
        .collect();

    use crate::e2e::field_access::{StringyField, StringyFieldKind};
    // Enums are bridged as `String` on the swift-bridge surface (the binding
    // emits `fn kind(&self) -> String` for `kind: SomeEnum`), so they must
    // also count as text-bearing accessors when aggregating contains-matchers.
    let enum_names: HashSet<&str> = enum_defs.iter().map(|e| e.name.as_str()).collect();
    // ~keep Mirrors `gen_rust_crate::mod`'s own `tagged_enum_names` filter exactly (no `has_serde`
    // gate -- a data-carrying enum only ever reaches codegen once it can serialize, so gating here
    // too would just duplicate a fact codegen already enforces). A data-carrying (tagged-union)
    // enum field's getter is NOT reached by `field_needs_json_bridge`: `emit_getters` routes it
    // through `is_enum_named` + `emit_enum_string_getter` instead, which for a non-unit variant
    // emits `serde_json::to_string(&self.0.field)` -- the same whole-value JSON serialization as
    // the generic bridge path, just via a second mechanism `field_needs_json_bridge` cannot see.
    // A struct-typed field must not be swept in here: only a `Named` type this set names is a
    // tagged union, so `pages: Option<PageStructure>` and friends stay correctly non-bridged.
    let tagged_enum_names: HashSet<&str> = enum_defs
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();
    let classify_stringy = |ty: &TypeRef, field_optional: bool| -> Option<StringyFieldKind> {
        match ty {
            TypeRef::String => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Optional),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Optional),
                _ => None,
            },
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Vec),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Vec),
                _ => None,
            },
            _ => None,
        }
    };
    let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
    let mut getter_optionality: HashMap<String, HashMap<String, bool>> = HashMap::new();
    let mut json_bridged_by_type: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for td in type_defs {
        let mut td_field_types: HashMap<String, String> = HashMap::new();
        let mut td_stringy: Vec<StringyField> = Vec::new();
        let mut td_getter_optionality: HashMap<String, bool> = HashMap::new();
        let mut td_json_bridged: HashMap<String, bool> = HashMap::new();
        for f in &td.fields {
            if let Some(named) = inner_named(&f.ty) {
                td_field_types.insert(f.name.clone(), named);
            }
            // ~keep `field_needs_json_bridge` is the *same* call `wrappers::getters::emit_getters`
            // makes to pick a getter's Rust return type (`String` when it answers true), so this
            // asks the binding generator for the shape rather than re-deriving it. The re-derivation
            // this replaced tracked only `Vec<Vec<_>>`/`Map<_>` plus two hand-enumerated
            // `Option<Vec<Named(..)>>` special cases, so it called every other optional `Vec`
            // countable — `Option<Vec<String>>` among them, which really emits
            // `fn og_locale_alternates(&self) -> String` and made the e2e emit `?.count` on a
            // `RustString`. Two generators reading one IR and disagreeing about one field is the
            // shape this whole map exists to avoid, so there is now one predicate, not two.
            let plain_json_bridge = field_needs_json_bridge(&f.ty, f.optional);
            // ~keep `emit_getters` reaches a whole-value JSON string by a SECOND route
            // `field_needs_json_bridge` does not cover: a `Named` field whose type is a
            // data-carrying (tagged-union) enum is routed through `is_enum_named` instead, and for
            // a non-unit variant that path also emits `serde_json::to_string(&self.0.field)` (see
            // `emit_enum_string_getter`). A fieldless (unit) enum getter is a bare variant-name
            // `to_string()` -- readable text, not a JSON blob -- so only `tagged_enum_names` (never
            // the broader `enum_names`) may fold in here.
            let tagged_enum_json_bridge = matches!(&f.ty, TypeRef::Named(n) if tagged_enum_names.contains(n.as_str()));
            let getter_is_json_bridged_string = plain_json_bridge || tagged_enum_json_bridge;
            if getter_is_json_bridged_string {
                json_bridged_field_names.insert(f.name.clone());
            }
            // ~keep Recorded for every field, bridged or not: an absent entry has to keep meaning
            // "the scan never saw this field on this type" so a caller can tell that apart from a
            // negative answer. See `SwiftFirstClassMap::json_bridged_by_type`.
            td_json_bridged.insert(f.name.clone(), getter_is_json_bridged_string);
            if is_vec_ty(&f.ty) && !getter_is_json_bridged_string {
                vec_field_names.insert(f.name.clone());
            }
            // ~keep Mirrors `emit_getters`' own `bridge_ty_owned` choice: a JSON-bridged field
            // collapses to a bare `String`, so only a non-bridged optional field gets the
            // `Option<..>` return type that forces `?.` on whatever the caller chains next. Gated
            // on `plain_json_bridge` alone, NOT `getter_is_json_bridged_string`: unlike the generic
            // bridge path, a tagged-union enum's `bridge_ty_owned` stays `Option<String>` when the
            // field is optional (`bridge_type_enum_aware_ref` collapses the NAMED type to `String`,
            // but the surrounding `field.optional` branch in `emit_getters` still wraps it in
            // `Option<..>`) -- folding the enum case in here would wrongly report a getter that
            // Swift declares `Optional<RustString>` as non-optional.
            td_getter_optionality.insert(f.name.clone(), f.optional && !plain_json_bridge);
            if f.binding_excluded {
                continue;
            }
            if let Some(kind) = classify_stringy(&f.ty, f.optional) {
                td_stringy.push(StringyField {
                    name: f.name.clone(),
                    kind,
                });
            }
        }
        if !td_field_types.is_empty() {
            field_types.insert(td.name.clone(), td_field_types);
        }
        if !td_stringy.is_empty() {
            stringy_fields_by_type.insert(td.name.clone(), td_stringy);
        }
        if !td_getter_optionality.is_empty() {
            getter_optionality.insert(td.name.clone(), td_getter_optionality);
        }
        if !td_json_bridged.is_empty() {
            json_bridged_by_type.insert(td.name.clone(), td_json_bridged);
        }
    }
    // Root-type detection: first check for an explicit `result_type` override
    // in the call config. If present, use that directly. Otherwise fall back to
    // picking a unique TypeDef that contains all `result_fields`.
    let root_type = swift_call_result_type(call_config).or_else(|| {
        if e2e_config.result_fields.is_empty() {
            None
        } else {
            let matches: Vec<&crate::core::ir::TypeDef> = type_defs
                .iter()
                .filter(|td| {
                    let names: HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
                    e2e_config.result_fields.iter().all(|rf| names.contains(rf.as_str()))
                })
                .collect();
            if matches.len() == 1 {
                Some(matches[0].name.clone())
            } else {
                None
            }
        }
    });
    SwiftFirstClassMap {
        first_class_types,
        field_types,
        vec_field_names,
        json_bridged_field_names,
        json_bridged_by_type,
        getter_optionality,
        root_type,
        stringy_fields_by_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{CallConfig, E2eConfig};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    fn named_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// `Option<Vec<Named>>` is JSON-bridged, not countable.
    ///
    /// ~keep This test previously asserted the opposite, on the theory that swift-bridge exposes
    /// the field natively as `Optional<RustVec<T>>`. Generating a Swift package from this exact
    /// shape disproves it: the wrapper emits `fn headings(&self) -> String` (whole-field
    /// `serde_json::to_string`), because `emit_getters` picks the getter's return type with
    /// `field_needs_json_bridge`, which answers true for *every* optional `Vec`. The Swift leaf is
    /// therefore a `RustString` with no `.count`, and the compile error the old test cited
    /// ("`RustVec<Element>?` has no member `toString`") cannot have come from this classification
    /// — no accessor of that type is ever produced for this shape.
    #[test]
    fn optional_vec_of_named_leaf_is_json_bridged_not_countable() {
        let element_dto = TypeDef {
            name: "Element".to_string(),
            fields: vec![named_field("kind", TypeRef::String, false)],
            ..Default::default()
        };
        let result_dto = TypeDef {
            name: "ExtractionResult".to_string(),
            fields: vec![named_field(
                "elements",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Element".to_string()))))),
                true,
            )],
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[element_dto, result_dto],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            !map.is_vec_field_name("elements"),
            "Option<Vec<Named>> emits a whole-field String getter and must not be marked countable"
        );
        assert!(
            map.is_json_bridged_field_name("elements"),
            "Option<Vec<Named>> must be recorded as JSON-bridged so index and count steps are refused"
        );
    }

    /// Genuinely JSON-bridged shapes — `Vec<Vec<T>>` (and `Option<Vec<Vec<T>>>`) —
    /// return a plain `RustString` getter with no `.count`, so they must stay excluded
    /// from `vec_field_names` (and thus never get a `.count` assertion emitted).
    #[test]
    fn optional_vec_of_vec_stays_json_bridged() {
        let result_dto = TypeDef {
            name: "TableResult".to_string(),
            fields: vec![named_field(
                "rows",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(
                    TypeRef::String,
                )))))),
                true,
            )],
            ..Default::default()
        };

        let map = build_swift_first_class_map(&[result_dto], &[], &E2eConfig::default(), &CallConfig::default());

        assert!(
            !map.is_vec_field_name("rows"),
            "Vec<Vec<T>> field is JSON-bridged to RustString and must not be marked countable"
        );
    }

    /// Regression test for the `headings()?.count` swift e2e compile failure:
    /// `headings: Option<Vec<HeadingInfo>>` where `HeadingInfo` is a serde struct.
    /// Because the parent type (`Metadata`) is first-class and `HeadingInfo` has serde,
    /// `emit_vec_struct_serde_getter` (`gen_rust_crate::wrappers::getters`) collapses the
    /// whole optional field to `fn headings(&self) -> String` (whole-field
    /// `serde_json::to_string`), not a countable `RustVec`/`Vec<String>`. Emitting
    /// `headings()?.count` against that `String` fails with "cannot use optional
    /// chaining on non-optional value of type 'RustString'" / "value of type
    /// 'RustString' has no member 'count'". `headings` must be classified as
    /// JSON-bridged (non-countable) so the e2e generator never emits `.count` on it.
    #[test]
    fn optional_vec_of_serde_struct_on_first_class_parent_is_json_bridged() {
        let heading_info = TypeDef {
            name: "HeadingInfo".to_string(),
            fields: vec![named_field("text", TypeRef::String, false)],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "headings",
                TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                true,
            )],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[heading_info, metadata],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            !map.is_vec_field_name("headings"),
            "Option<Vec<Named(struct)>> on a first-class parent is a whole-field String getter \
             and must not be marked countable"
        );
    }

    /// Regression test for the non-optional sibling of the `headings` shape above:
    /// `Vec<Named(struct)>` (not optional) on a first-class serde parent routes through
    /// `emit_vec_struct_serde_getter`'s non-optional template, which returns a per-element
    /// `Vec<String>` — real, countable via `.count`. Only the `Option<...>` variant
    /// collapses to a whole-field `String`.
    #[test]
    fn non_optional_vec_of_serde_struct_on_first_class_parent_is_countable() {
        let heading_info = TypeDef {
            name: "HeadingInfo".to_string(),
            fields: vec![named_field("text", TypeRef::String, false)],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "headings",
                TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                false,
            )],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[heading_info, metadata],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            map.is_vec_field_name("headings"),
            "non-optional Vec<Named(struct)> on a first-class parent returns a countable Vec<String>"
        );
    }

    fn named_enum_variant(name: &str, fields: Vec<FieldDef>) -> crate::core::ir::EnumVariant {
        crate::core::ir::EnumVariant {
            name: name.to_string(),
            fields,
            ..Default::default()
        }
    }

    /// A data-carrying (tagged-union) enum field is JSON-bridged, and its getter STAYS
    /// `Option<..>`-shaped when the field is optional.
    ///
    /// ~keep Pins the second route `emit_getters` uses to reach a whole-value JSON `String`:
    /// `field_needs_json_bridge` never sees this field (its `TypeRef` is a bare `Named`, not a
    /// `Vec`/`Map`/`Optional`), so a `Named` field whose type is a tagged-union enum was
    /// classified `json_bridged_by_type == false` even though `emit_enum_string_getter`'s
    /// non-unit-variant branch emits `self.0.format.clone().map(|w| serde_json::to_string(&w)…)` —
    /// the same whole-field JSON serialization, reached through `is_enum_named` instead. That
    /// silent disagreement made a fixture path stepping past the leaf (`format.excel.sheet_count`)
    /// resolve to `None` and get dropped as `FieldSkip::CountOnJsonBridgedLeafInSwift`, even though
    /// `swift_json_bridged_navigation` can walk right through it once the map says `true`.
    #[test]
    fn tagged_union_enum_field_is_json_bridged_and_stays_optional() {
        let format_metadata_enum = crate::core::ir::EnumDef {
            name: "FormatMetadata".to_string(),
            has_serde: true,
            variants: vec![named_enum_variant(
                "Excel",
                vec![named_field("excel", TypeRef::Named("ExcelMetadata".to_string()), false)],
            )],
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "format",
                TypeRef::Named("FormatMetadata".to_string()),
                true,
            )],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[metadata],
            &[format_metadata_enum],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert_eq!(
            map.json_bridged_getter("Metadata", "format"),
            Some(true),
            "a tagged-union enum field's getter serializes the whole value to JSON, same as a \
             generic json-bridged container"
        );
        assert_eq!(
            map.getter_is_optional("Metadata", "format"),
            Some(true),
            "the real getter stays `Optional<RustString>` for an optional tagged-union enum field \
             -- only the generic json-bridge path collapses `Option` away"
        );
    }

    /// A plain `Option<Named(struct))` field (no enum involved at all) is NOT JSON-bridged: it
    /// stays an opaque, per-type-wrapped optional -- exactly the `pages`/`imagePreprocessing`/
    /// `error` shapes this fix must not regress.
    #[test]
    fn plain_named_struct_field_is_not_json_bridged() {
        let page_structure = TypeDef {
            name: "PageStructure".to_string(),
            fields: vec![named_field(
                "page_count",
                TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                false,
            )],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field("pages", TypeRef::Named("PageStructure".to_string()), true)],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[page_structure, metadata],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert_eq!(
            map.json_bridged_getter("Metadata", "pages"),
            Some(false),
            "a plain struct field must stay non-bridged -- folding every Option<Named> into the \
             bridge set would wrongly flip opaque-handle fields like pages/imagePreprocessing/error"
        );
        assert_eq!(
            map.getter_is_optional("Metadata", "pages"),
            Some(true),
            "the opaque getter is genuinely Optional<PageStructure>"
        );
    }

    /// A unit (fieldless) enum field is readable text (`to_string()` yields the variant name),
    /// not a JSON blob -- it must not be swept into the json-bridge set alongside tagged unions.
    #[test]
    fn unit_enum_field_is_not_json_bridged() {
        let status_enum = crate::core::ir::EnumDef {
            name: "Status".to_string(),
            has_serde: true,
            variants: vec![
                named_enum_variant("Ready", vec![]),
                named_enum_variant("Pending", vec![]),
            ],
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field("status", TypeRef::Named("Status".to_string()), false)],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[metadata],
            &[status_enum],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert_eq!(
            map.json_bridged_getter("Metadata", "status"),
            Some(false),
            "a unit enum's getter is a bare variant-name String, not a JSON-serialized blob"
        );
        assert_eq!(map.getter_is_optional("Metadata", "status"), Some(false));
    }

    /// End-to-end: the exact fixture shape from the bug report,
    /// `results[0].metadata.format.excel.sheet_count`, resolved through the REAL IR-derived map
    /// (not a hand-built stub) into a real `FieldResolver`. Before this fix, `format`'s absence
    /// from `json_bridged_by_type` made `swift_json_bridged_navigation` return `None`, which the
    /// e2e generator turned into a silently dropped `FieldSkip::CountOnJsonBridgedLeafInSwift`.
    #[test]
    fn tagged_union_field_is_navigable_end_to_end_from_real_ir() {
        use crate::e2e::field_access::{FieldResolver, JsonNavStep};

        let format_metadata_enum = crate::core::ir::EnumDef {
            name: "FormatMetadata".to_string(),
            has_serde: true,
            variants: vec![named_enum_variant(
                "Excel",
                vec![named_field("excel", TypeRef::Named("ExcelMetadata".to_string()), false)],
            )],
            ..Default::default()
        };
        let extracted_document = TypeDef {
            name: "ExtractedDocument".to_string(),
            is_opaque: true,
            fields: vec![named_field("metadata", TypeRef::Named("Metadata".to_string()), false)],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "format",
                TypeRef::Named("FormatMetadata".to_string()),
                true,
            )],
            has_serde: true,
            ..Default::default()
        };

        let mut map = build_swift_first_class_map(
            &[extracted_document, metadata],
            &[format_metadata_enum],
            &E2eConfig::default(),
            &CallConfig::default(),
        );
        map.root_type = Some("ExtractedDocument".to_string());

        let resolver = FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );

        let (leaf_field, steps) = resolver
            .swift_json_bridged_navigation("metadata.format.excel.sheet_count")
            .expect("format must resolve as a navigable JSON-bridged leaf on the real IR map");

        assert_eq!(leaf_field, "metadata.format");
        // `FormatMetadata` is internally tagged, so `excel` names a variant, not a JSON key --
        // there is no `"excel"` key on the wire to look up. See
        // `field_access::format_metadata_variants` for the shared skip list.
        assert_eq!(steps, vec![JsonNavStep::Key("sheet_count".to_string())]);
    }
}
