use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::e2e::escape::escape_java as escape_swift_str;
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
    let type_name = options_type.unwrap_or(arg.name.as_str());
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

/// Returns true when the field type would be emitted as a Swift primitive value
/// or a known first-class Codable struct/unit-enum, so it can appear on a
/// first-class Codable Swift struct without forcing the host type into a
/// typealias. Mirrors `first_class_field_supported` in alef-backend-swift.
///
/// Accepts:
/// - `Primitive` and `String`
/// - `Named(S)` when `S` is in `known_dto_names` (seeded with unit-serde enums and
///   grown via fixed-point iteration over candidate struct DTOs)
/// - `Vec<T>` and `Optional<T>` recursively
///
/// Rejects `Map`, `Path`, `Bytes`, `Duration`, `Char`, `Json`, and unknown
/// `Named(_)` references (the backend treats those as typealias-to-opaque).
pub(super) fn swift_first_class_field_supported(
    ty: &crate::core::ir::TypeRef,
    known_dto_names: &HashSet<String>,
) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(_) | TypeRef::String => true,
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => swift_first_class_field_supported(inner, known_dto_names),
        _ => false,
    }
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
    for td in type_defs {
        let mut td_field_types: HashMap<String, String> = HashMap::new();
        let mut td_stringy: Vec<StringyField> = Vec::new();
        for f in &td.fields {
            if let Some(named) = inner_named(&f.ty) {
                td_field_types.insert(f.name.clone(), named);
            }
            if is_vec_ty(&f.ty) {
                vec_field_names.insert(f.name.clone());
            }
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
        root_type,
        stringy_fields_by_type,
    }
}
