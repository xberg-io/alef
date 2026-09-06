use crate::core::ir::{ApiSurface, FieldDef, ParamDef, TypeRef};
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
            sanitize_field(field, &known_types, &known_enums);
            if !field.sanitized
                && let Some(path) = field.type_rust_path.as_deref()
                && let Some(name) = named_type_name(&field.ty)
            {
                let known_name = known_types.contains(name) || known_enums.contains(name);
                if known_name
                    && !field_path_matches_known_type(path, name, &known_type_paths, &known_enum_paths, &api_crate_name)
                {
                    record_pre_sanitization_type(field);
                    field.ty = TypeRef::String;
                    field.sanitized = true;
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
                if sanitize_param(param, &known_types, &known_enums) {
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
            if sanitize_param(param, &known_types, &known_enums) {
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
                sanitize_field(field, &known_types, &known_enums);
            }
        }
    }
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            for field in &mut variant.fields {
                sanitize_field(field, &known_types, &known_enums);
            }
        }
    }
}

/// Sanitize `field.ty` in place, keeping a record of the Rust type it had beforehand.
///
/// `extract_tuple_vec_original_type` only recognizes the tuple-Vec and fixed-tuple-array shapes
/// the wasm backend reconstructs; every other lossy rewrite used to leave `original_type` unset,
/// which erased the declared type name for good. `type_rust_path` does not cover the gap either
/// -- `extract_field_type_rust_path` returns `None` for the single-segment path an imported type
/// has at its use site. The Rust reference page is the surface that needs the name back, since
/// it documents the crate rather than a binding. Every backend reader of `original_type` on a
/// *field* gates on a `Vec<(` or `[(` prefix (`is_sanitized_tuple_vec` /
/// `is_sanitized_fixed_tuple_array` in `backends/wasm/gen_bindings/enums.rs`), so the wider
/// population stays inert for them. ~keep
fn sanitize_field(field: &mut FieldDef, known_types: &AHashSet<String>, known_enums: &AHashSet<String>) {
    let tuple_original = extract_tuple_vec_original_type(&field.ty);
    let lowered_fixed_array = lowers_a_fixed_array(&field.ty, known_types, known_enums);
    let pre_sanitization = field.ty.rust_source_display();
    if sanitize_type_ref(&mut field.ty, known_types, known_enums).is_lossy() {
        field.sanitized = true;
        if let Some(orig) = tuple_original {
            field.original_type = Some(orig);
        } else if field.original_type.is_none() {
            field.original_type = Some(pre_sanitization);
        }
    } else if lowered_fixed_array && field.original_type.is_none() {
        // The declared length is the one fact `Vec<T>` cannot carry. Recording it costs nothing --
        // every backend reader of a *field*'s `original_type` also requires `field.sanitized`,
        // which a lossless lowering never sets. ~keep
        field.original_type = Some(pre_sanitization);
    }
}

/// Sanitize `param.ty` in place, keeping a record of the Rust type it had beforehand.
///
/// Mirrors [`sanitize_field`]: many backends (`dart`, `wasm`, `magnus`, `php`, `ffi`, `swift`,
/// and the shared `codegen::generators::binding_helpers` call-site builders) already gate
/// reconstruction logic on `param.original_type.is_some()` combined with `param.sanitized`,
/// expecting it to be populated the same way a field's is. Nothing wrote to `ParamDef::original_type`
/// before this fix, so every one of those call sites was silently inert for parameters. Returns
/// whether the rewrite was lossy, so callers can fold it into their own `*_sanitized` flag exactly
/// like the removed inline `sanitize_type_ref(..).is_lossy()` call did. ~keep
fn sanitize_param(param: &mut ParamDef, known_types: &AHashSet<String>, known_enums: &AHashSet<String>) -> bool {
    let tuple_original = extract_tuple_vec_original_type(&param.ty);
    let lowered_fixed_array = lowers_a_fixed_array(&param.ty, known_types, known_enums);
    let pre_sanitization = param.ty.rust_source_display();
    let is_lossy = sanitize_type_ref(&mut param.ty, known_types, known_enums).is_lossy();
    if is_lossy {
        param.sanitized = true;
        if let Some(orig) = tuple_original {
            param.original_type = Some(orig);
        } else if param.original_type.is_none() {
            param.original_type = Some(pre_sanitization);
        }
    } else if lowered_fixed_array && param.original_type.is_none() {
        // Same rationale as the field path: a lossless fixed-array lowering still erases the
        // declared length, so it is recorded here too -- without setting `sanitized`, which
        // stays reserved for lossy rewrites. ~keep
        param.original_type = Some(pre_sanitization);
    }
    is_lossy
}

/// Record `field`'s current type as its pre-sanitization type for a caller that is about to
/// overwrite `field.ty` itself rather than going through [`sanitize_field`]. ~keep
fn record_pre_sanitization_type(field: &mut FieldDef) {
    if field.original_type.is_none() {
        field.original_type = Some(field.ty.rust_source_display());
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
            info!("Keeping Rust-only function signature: {} ({})", func.name, reason);
        }
    }
    // Keep source-declared signatures even when the function is hidden from generated
    // bindings. Rust e2e tests call the source crate directly, and their argument and result
    // decisions therefore still need the real signature. Binding generators already enforce
    // binding_excluded at their visibility boundary; deleting the record here made that
    // language-aware policy impossible to apply downstream. ~keep

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
        if let TypeRef::Vec(inner) = ty
            && let TypeRef::Named(name) = inner.as_ref()
            && name.trim_start().starts_with('(')
        {
            return Some(format!("Vec<{name}>"));
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

/// The element name of a fixed-size array whose element type the binding surface already carries,
/// e.g. `"[Point ; 4]"` → `Some("Point")`. `None` for every other string.
///
/// `resolve_type` has no IR variant for `syn::Type::Array`, so the extractor stringifies the whole
/// array into a `TypeRef::Named` that no `known_types` lookup can ever match, and the field is
/// rewritten to a JSON `String`. Recovering the element here rather than in the resolver is what
/// keeps the `[(K, V); N]` shape on its existing JSON path: a tuple is never a known type name, so
/// it can never match this. `quote` leaves a space before the `;`, hence the trimming. ~keep
fn fixed_array_element_of_known_type(
    name: &str,
    known_types: &AHashSet<String>,
    known_enums: &AHashSet<String>,
) -> Option<String> {
    let inner = name.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (element, length) = inner.rsplit_once(';')?;
    let element = element.trim();
    if length.trim().is_empty() {
        return None;
    }
    if !known_types.contains(element) && !known_enums.contains(element) {
        return None;
    }
    Some(element.to_string())
}

/// Whether `ty` contains a fixed-size array that [`fixed_array_element_of_known_type`] will lower.
///
/// Mirrors the shapes [`sanitize_type_ref`] recurses through, so the caller can tell a lowered
/// array apart from an unchanged type without re-deriving the pre-sanitization shape. ~keep
fn lowers_a_fixed_array(ty: &TypeRef, known_types: &AHashSet<String>, known_enums: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => fixed_array_element_of_known_type(name, known_types, known_enums).is_some(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => lowers_a_fixed_array(inner, known_types, known_enums),
        TypeRef::Map(key, value) => {
            lowers_a_fixed_array(key, known_types, known_enums) || lowers_a_fixed_array(value, known_types, known_enums)
        }
        _ => false,
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
            if let Some(element) = fixed_array_element_of_known_type(name, known_types, known_enums) {
                *ty = TypeRef::Vec(Box::new(TypeRef::Named(element)));
                return TypeSanitization::Lossless;
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
