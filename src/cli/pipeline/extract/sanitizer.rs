use crate::core::ir::{ApiSurface, TypeRef};
use ahash::{AHashMap, AHashSet};
use tracing::info;

pub(super) fn sanitize_unknown_types(api: &mut ApiSurface) {
    let api_crate_name = api.crate_name.replace('-', "_");
    let known_types: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    let known_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let known_type_paths = rust_paths_by_name(api.types.iter().map(|t| (&t.name, &t.rust_path)));
    let known_enum_paths = rust_paths_by_name(api.enums.iter().map(|e| (&e.name, &e.rust_path)));

    for typ in &mut api.types {
        for field in &mut typ.fields {
            let original = extract_tuple_vec_original_type(&field.ty);
            if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                field.sanitized = true;
                if let Some(orig) = original {
                    field.original_type = Some(orig);
                }
            }
            if !field.sanitized {
                if let Some(path) = field.type_rust_path.as_deref() {
                    if let Some(name) = named_type_name(&field.ty) {
                        let known_name = known_types.contains(name) || known_enums.contains(name);
                        if known_name
                            && !field_path_matches_known_type(
                                path,
                                name,
                                &known_type_paths,
                                &known_enum_paths,
                                &api_crate_name,
                            )
                        {
                            field.ty = TypeRef::String;
                            field.sanitized = true;
                        }
                    }
                }
            }
        }
        let type_name = typ.name.clone();
        let is_trait = typ.is_trait;
        for method in &mut typ.methods {
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

fn rust_paths_by_name<'a>(items: impl Iterator<Item = (&'a String, &'a String)>) -> AHashMap<String, Vec<String>> {
    let mut paths = AHashMap::new();
    for (name, path) in items {
        paths
            .entry(name.clone())
            .or_insert_with(Vec::new)
            .push(path.replace('-', "_"));
    }
    paths
}

fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_type_name(inner),
        TypeRef::Map(_, value) => named_type_name(value),
        _ => None,
    }
}

fn field_path_matches_known_type(
    field_path: &str,
    name: &str,
    known_type_paths: &AHashMap<String, Vec<String>>,
    known_enum_paths: &AHashMap<String, Vec<String>>,
    api_crate_name: &str,
) -> bool {
    let field_path = field_path.replace('-', "_");
    known_type_paths
        .get(name)
        .into_iter()
        .chain(known_enum_paths.get(name))
        .flatten()
        .any(|known_path| paths_compatible(&field_path, known_path, api_crate_name))
}

fn paths_compatible(a: &str, b: &str, api_crate_name: &str) -> bool {
    if a == b {
        return true;
    }

    let a_root = a.split("::").next().unwrap_or("");
    let b_root = b.split("::").next().unwrap_or("");
    let a_name = a.rsplit("::").next().unwrap_or("");
    let b_name = b.rsplit("::").next().unwrap_or("");
    if a_name != b_name {
        return false;
    }
    a_root == b_root || a_root == api_crate_name
}

pub(super) fn strip_binding_excluded(api: &mut ApiSurface) -> anyhow::Result<()> {
    for typ in &api.types {
        if typ.binding_excluded {
            let reason = typ
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded type: {} ({})", typ.name, reason);
            api.excluded_type_paths
                .insert(typ.name.clone(), typ.rust_path.replace('-', "_"));
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
            if !variant.fields.is_empty() && variant.fields.iter().all(|f| f.binding_excluded) {
                variant.originally_had_data_fields = true;
            }
        }
    }

    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            let _ = variant;
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
            if name == "Value" || name == "JsonValue" {
                return TypeSanitization::Unchanged;
            }
            if let Some(elem_ty) = parse_homogeneous_tuple(name) {
                *ty = TypeRef::Vec(Box::new(elem_ty));
                return TypeSanitization::Lossy;
            }
            *ty = TypeRef::String;
            TypeSanitization::Lossy
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => sanitize_type_ref(inner, known_types, known_enums),
        TypeRef::Map(k, v) => {
            if contains_ambiguous_bare_value(k) || contains_ambiguous_bare_value(v) {
                return TypeSanitization::Lossy;
            }
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
