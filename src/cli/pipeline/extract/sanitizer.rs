use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use tracing::info;

pub(super) fn sanitize_unknown_types(api: &mut ApiSurface) {
    let known_types: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    let known_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Build a set of known rust_paths for types and enums.
    // This enables disambiguation of types with the same short name but different
    // module paths (e.g., `sample_core::types::OutputFormat` vs `sample_core::OutputFormat`).
    // Normalize hyphens to underscores in paths for consistent comparison.
    let known_type_paths: AHashSet<String> = api.types.iter().map(|t| t.rust_path.replace('-', "_")).collect();
    let known_enum_paths: AHashSet<String> = api.enums.iter().map(|e| e.rust_path.replace('-', "_")).collect();

    for typ in &mut api.types {
        for field in &mut typ.fields {
            let original = extract_tuple_vec_original_type(&field.ty);
            if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                field.sanitized = true;
                if let Some(orig) = original {
                    field.original_type = Some(orig);
                }
            }
            // Second pass: check type_rust_path for name-collision disambiguation.
            // If a field has a type_rust_path that doesn't match any known type/enum rust_path,
            // it references a different type that happens to share the same short name
            // (e.g., crate::types::OutputFormat vs crate::core::config::OutputFormat).
            if !field.sanitized {
                if let Some(ref path) = field.type_rust_path {
                    let normalized_path = path.replace('-', "_");
                    if let TypeRef::Named(ref name) = field.ty {
                        // Only check if the name matches a known type/enum — otherwise it's
                        // already handled by the standard sanitization above.
                        if known_types.contains(name.as_str()) || known_enums.contains(name.as_str()) {
                            // Check if the full path's last segment matches any known type/enum path's last segment.
                            // This handles cases where module paths differ but the type is the same
                            // (e.g., crate::metadata::HtmlMetadata vs sample-markdown-rs::HtmlMetadata).
                            let path_type_name = normalized_path.rsplit("::").next().unwrap_or("");
                            let path_matches = known_type_paths
                                .iter()
                                .chain(known_enum_paths.iter())
                                .any(|kp| kp.rsplit("::").next().unwrap_or("") == path_type_name);
                            if !path_matches {
                                field.ty = TypeRef::String;
                                field.sanitized = true;
                            }
                        }
                    }
                    // Also check Named types inside Optional/Vec wrappers
                    if let TypeRef::Vec(ref inner) = field.ty {
                        if let TypeRef::Named(ref name) = **inner {
                            let vec_path_type = normalized_path.rsplit("::").next().unwrap_or("");
                            let vec_matches = known_type_paths
                                .iter()
                                .chain(known_enum_paths.iter())
                                .any(|kp| kp.rsplit("::").next().unwrap_or("") == vec_path_type);
                            if (known_types.contains(name.as_str()) || known_enums.contains(name.as_str()))
                                && !vec_matches
                            {
                                field.ty = TypeRef::String;
                                field.sanitized = true;
                            }
                        }
                    }
                }
            }
        }
        let type_name = typ.name.clone();
        let is_trait = typ.is_trait;
        for method in &mut typ.methods {
            // Trait method params and return types must match the original Rust trait
            // signature exactly — bridge codegen emits `impl Trait for Wrapper { fn ... }`
            // and the impl must satisfy the trait. Sanitizing these would cause
            // E0053 (incompatible type) trait coherence errors. Internal-only param
            // types are handled by per-backend JSON serialization in the bridge body.
            if is_trait {
                continue;
            }
            let mut method_sanitized = false;
            for param in &mut method.params {
                if sanitize_type_ref(&mut param.ty, &known_types, &known_enums).is_lossy() {
                    param.sanitized = true;
                    method_sanitized = true;
                }
            }
            // Skip sanitizing return type if it's Named(parent_type) — builder/factory pattern.
            // Methods that return their own type (e.g. with_foo(&self) -> Self) should keep
            // the Named return so codegen can delegate them correctly.
            let is_self_return = matches!(&method.return_type, TypeRef::Named(n) if n == &type_name);
            if !is_self_return && sanitize_type_ref(&mut method.return_type, &known_types, &known_enums).is_lossy() {
                method_sanitized = true;
            }
            if method_sanitized {
                method.sanitized = true;
            }
        }
    }
    for func in &mut api.functions {
        let mut func_sanitized = false;
        for param in &mut func.params {
            if sanitize_type_ref(&mut param.ty, &known_types, &known_enums).is_lossy() {
                param.sanitized = true;
                func_sanitized = true;
            }
        }
        if sanitize_type_ref(&mut func.return_type, &known_types, &known_enums).is_lossy() {
            func_sanitized = true;
            func.return_sanitized = true;
        }
        if func_sanitized {
            func.sanitized = true;
        }
    }
    // Sanitize enum variant fields — tuples and other unknown types in data enum
    // variants must be replaced with String, otherwise backends emit invalid code
    // (e.g., Go emitting `[](String, String)` for Vec<(String, String)>).
    for enum_def in &mut api.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                let original = extract_tuple_vec_original_type(&field.ty);
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                    field.sanitized = true;
                    if let Some(orig) = original {
                        field.original_type = Some(orig);
                    }
                }
            }
        }
    }
    // Sanitize error variant fields as well.
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            for field in &mut variant.fields {
                let original = extract_tuple_vec_original_type(&field.ty);
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                    field.sanitized = true;
                    if let Some(orig) = original {
                        field.original_type = Some(orig);
                    }
                }
            }
        }
    }
}

pub(super) fn strip_binding_excluded(api: &mut ApiSurface) -> anyhow::Result<()> {
    // --- Item-level exclusions: types, enums, errors, functions ---

    // Capture rust_paths of excluded types/enums/errors before removal so that
    // trait-bridge codegen can still reference them by qualified path.
    for typ in &api.types {
        if typ.binding_excluded {
            let reason = typ
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded type: {} ({})", typ.name, reason);
            api.excluded_type_paths
                .insert(typ.name.clone(), typ.rust_path.replace('-', "_"));
            // Preserve trait-ness across the strip so trait-bridge codegen can tell
            // an excluded trait (`&dyn Trait` → non-bridgeable, skip the method) from
            // an excluded struct/enum (`&HiddenDocument` → reference by qualified path).
            if typ.is_trait {
                api.excluded_trait_names.insert(typ.name.clone());
            }
        }
    }
    for enm in &api.enums {
        if enm.binding_excluded {
            let reason = enm
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded enum: {} ({})", enm.name, reason);
            api.excluded_type_paths
                .insert(enm.name.clone(), enm.rust_path.replace('-', "_"));
        }
    }
    for err in &api.errors {
        if err.binding_excluded {
            let reason = err
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded error: {} ({})", err.name, reason);
            api.excluded_type_paths
                .insert(err.name.clone(), err.rust_path.replace('-', "_"));
        }
    }

    api.types.retain(|t| !t.binding_excluded);
    api.enums.retain(|e| !e.binding_excluded);
    api.errors.retain(|e| !e.binding_excluded);

    for func in &api.functions {
        if func.binding_excluded {
            let reason = func
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded function: {} ({})", func.name, reason);
        }
    }
    api.functions.retain(|f| !f.binding_excluded);

    // --- Method-level exclusions on retained types ---
    for typ in &mut api.types {
        let excluded_methods: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.binding_excluded)
            .map(|m| {
                let reason = m
                    .binding_exclusion_reason
                    .as_deref()
                    .unwrap_or("source binding exclusion");
                format!("{}.{} ({})", typ.name, m.name, reason)
            })
            .collect();
        if !excluded_methods.is_empty() {
            info!("Stripping excluded methods: {}", excluded_methods.join(", "));
        }
        typ.methods.retain(|m| !m.binding_excluded);
    }

    // --- Field-level exclusions ---
    // Keep excluded fields in IR so conversion generators can still initialize the
    // core field (usually with Default::default()) while public binding DTOs hide it.
    for typ in &api.types {
        let excluded: Vec<_> = typ
            .fields
            .iter()
            .filter(|field| field.binding_excluded)
            .map(|field| {
                let reason = field
                    .binding_exclusion_reason
                    .as_deref()
                    .unwrap_or("source binding exclusion");
                format!("{}.{} ({reason})", typ.name, field.name)
            })
            .collect();
        if !excluded.is_empty() {
            info!("Hiding binding-excluded fields: {}", excluded.join(", "));
        }
    }

    // Enum variant binding_excluded fields are RETAINED in the IR (like struct fields) so
    // that "to core" conversion codegen can initialize them with Default::default().
    // The `originally_had_data_fields` flag is set when all fields are binding_excluded so
    // that codegen can emit wildcard patterns on the core-type side. Mirror emitters skip
    // binding_excluded fields when building the public binding surface.
    for enum_def in &mut api.enums {
        let excluded: Vec<String> = enum_def
            .variants
            .iter()
            .flat_map(|variant| {
                variant.fields.iter().filter(|f| f.binding_excluded).map(|f| {
                    let reason = f
                        .binding_exclusion_reason
                        .as_deref()
                        .unwrap_or("source binding exclusion");
                    format!("{}::{}.{} ({reason})", enum_def.name, variant.name, f.name)
                })
            })
            .collect();
        if !excluded.is_empty() {
            info!("Hiding binding-excluded enum variant fields: {}", excluded.join(", "));
        }
        for variant in &mut enum_def.variants {
            // Set flag when ALL fields are binding_excluded so codegen knows the core type
            // still has data fields even though the mirror shows a unit variant.
            if !variant.fields.is_empty() && variant.fields.iter().all(|f| f.binding_excluded) {
                variant.originally_had_data_fields = true;
            }
            // Do NOT strip — retain fields so to-core conversion codegen can use them.
        }
    }

    // Error variants: same retention policy for binding_excluded fields.
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            // Fields are retained; the is_tuple flag (set during extraction) lets codegen
            // distinguish tuple vs struct variants for wildcard/default-init patterns.
            let _ = variant; // retention is implicit — no retain() call
        }
    }

    Ok(())
}

/// If `ty` is `Vec<(...)>` or `Option<Vec<(...)>>` — a Vec whose inner element is a tuple
/// type name — return a human-readable string capturing the original shape before sanitization
/// (e.g. `"Vec<(String, String)>"`).  Returns `None` for all other shapes.
///
/// This is called *before* `sanitize_type_ref` rewrites the inner `Named("(String, String)")`
/// to `String`, so backends can store this string in `FieldDef::original_type` and later emit
/// language-native pair types instead of a plain list.
fn extract_tuple_vec_original_type(ty: &TypeRef) -> Option<String> {
    fn inner_tuple_name(ty: &TypeRef) -> Option<String> {
        if let TypeRef::Vec(inner) = ty {
            if let TypeRef::Named(name) = inner.as_ref() {
                if name.trim_start().starts_with('(') {
                    return Some(format!("Vec<{name}>"));
                }
            }
        }
        None
    }
    /// Detect fixed-size tuple-array strings like `[(u32, u32); 4]`.
    ///
    /// The extractor emits these as `TypeRef::Named("[(u32, u32); 4]")` because there is no
    /// dedicated IR variant for fixed-size arrays.  We capture the string before sanitization
    /// so the wasm backend can reconstruct the type via `serde_wasm_bindgen::from_value`.
    fn fixed_tuple_array_name(name: &str) -> Option<String> {
        let s = name.trim();
        if s.starts_with("[(") && s.contains(");") {
            Some(s.to_string())
        } else {
            None
        }
    }
    match ty {
        TypeRef::Vec(_) => inner_tuple_name(ty),
        TypeRef::Optional(inner) => inner_tuple_name(inner),
        // Fixed-size tuple arrays arrive as Named("[(T, U); N]") from the extractor.
        TypeRef::Named(name) => fixed_tuple_array_name(name),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypeSanitization {
    Unchanged,
    Lossless,
    Lossy,
}

impl TypeSanitization {
    pub(super) fn is_lossy(self) -> bool {
        self == Self::Lossy
    }

    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Lossy, _) | (_, Self::Lossy) => Self::Lossy,
            (Self::Lossless, _) | (_, Self::Lossless) => Self::Lossless,
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
        }
    }
}

/// Sanitize a type reference while preserving whether the change is lossy.
pub(super) fn sanitize_type_ref(
    ty: &mut TypeRef,
    known_types: &AHashSet<String>,
    known_enums: &AHashSet<String>,
) -> TypeSanitization {
    match ty {
        TypeRef::Named(name) if !known_types.contains(name.as_str()) && !known_enums.contains(name.as_str()) => {
            // `Value` and `JsonValue` are bare names for serde_json::Value that the extractor
            // preserves as Named types. They are not unknown types to be collapsed to String,
            // but rather pseudo-types that should be preserved through Option/Vec/Map wrappers
            // so that type mappers can handle them appropriately. Do not sanitize.
            if name == "Value" || name == "JsonValue" {
                return TypeSanitization::Unchanged;
            }
            // Detect homogeneous numeric tuple types such as `(u32, u32)` that serde serializes
            // as JSON arrays.  Map them to Vec<ElemType> so backends emit array types (e.g.
            // `[]uint32` in Go) rather than falling back to `string`.  This preserves round-trip
            // JSON compatibility: `null | [800, 600]` unmarshals correctly into `*[]uint32`.
            if let Some(elem_ty) = parse_homogeneous_tuple(name) {
                *ty = TypeRef::Vec(Box::new(elem_ty));
                return TypeSanitization::Lossy; // Sanitized — the core type is a tuple, not a Vec
            }
            *ty = TypeRef::String;
            TypeSanitization::Lossy
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => sanitize_type_ref(inner, known_types, known_enums),
        TypeRef::Map(k, v) => {
            if contains_ambiguous_bare_value(k) || contains_ambiguous_bare_value(v) {
                return TypeSanitization::Lossy;
            }
            // Sanitize inner key and value types (e.g. Named("str") → String) so
            // backends receive clean Map(String, Json) rather than Map(Named("str"), Json).
            let key_status = sanitize_map_inner_type(k, known_types, known_enums);
            let value_status = sanitize_map_inner_type(v, known_types, known_enums);
            key_status.combine(value_status)
        }
        _ => TypeSanitization::Unchanged,
    }
}

fn sanitize_map_inner_type(
    ty: &mut TypeRef,
    known_types: &AHashSet<String>,
    known_enums: &AHashSet<String>,
) -> TypeSanitization {
    if matches!(ty, TypeRef::Named(name) if name == "str") {
        *ty = TypeRef::String;
        return TypeSanitization::Lossless;
    }
    sanitize_type_ref(ty, known_types, known_enums)
}

fn contains_ambiguous_bare_value(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(name) => name == "Value" || name == "JsonValue",
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_ambiguous_bare_value(inner),
        TypeRef::Map(key, value) => contains_ambiguous_bare_value(key) || contains_ambiguous_bare_value(value),
        _ => false,
    }
}

/// Parse a homogeneous numeric tuple type string such as `"(u32,u32)"` or `"(u64, u64)"`.
///
/// Returns `Some(TypeRef)` for the element type when all comma-separated elements inside the
/// parentheses are the same primitive type.  Returns `None` for heterogeneous tuples, non-tuple
/// strings, or unsupported element types.
///
/// This lets `sanitize_type_ref` map `Option<(u32, u32)>` → `Optional(Vec(Primitive(U32)))`
/// instead of falling back to `String`, preserving JSON array round-trip compatibility.
fn parse_homogeneous_tuple(name: &str) -> Option<TypeRef> {
    use crate::core::ir::PrimitiveType;
    let name = name.trim();
    let inner = name.strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    let first = parts[0];
    if !parts.iter().all(|p| *p == first) {
        return None;
    }
    // Homogeneous String tuples (e.g. `(String, String)`) serialize as JSON arrays of strings,
    // so map them to Vec<String> like numeric homogeneous tuples.
    if first == "String" {
        return Some(TypeRef::String);
    }
    let prim = match first {
        "u8" => PrimitiveType::U8,
        "u16" => PrimitiveType::U16,
        "u32" => PrimitiveType::U32,
        "u64" => PrimitiveType::U64,
        "i8" => PrimitiveType::I8,
        "i16" => PrimitiveType::I16,
        "i32" => PrimitiveType::I32,
        "i64" => PrimitiveType::I64,
        "f32" => PrimitiveType::F32,
        "f64" => PrimitiveType::F64,
        "usize" => PrimitiveType::Usize,
        "isize" => PrimitiveType::Isize,
        _ => return None,
    };
    Some(TypeRef::Primitive(prim))
}
