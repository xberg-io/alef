use crate::builder::StructBuilder;
use crate::generators::RustBindingConfig;
use crate::type_mapper::TypeMapper;
use alef_core::ir::{TypeDef, TypeRef};
use std::fmt::Write;

/// Check if a type's fields can all be safely defaulted.
/// Primitives, strings, collections, Options, and Duration all have Default impls.
/// Named types (custom structs) only have Default if explicitly marked with `has_default=true`.
/// If any field is a Named type without `has_default`, returning true would generate
/// code that calls `Default::default()` on a type that doesn't implement it.
pub fn can_generate_default_impl(typ: &TypeDef, known_default_types: &std::collections::HashSet<&str>) -> bool {
    for field in &typ.fields {
        if field.cfg.is_some() {
            continue; // Skip cfg-gated fields
        }
        if !field_type_has_default(&field.ty, known_default_types) {
            return false;
        }
    }
    true
}

/// Check if a specific TypeRef can be safely defaulted.
fn field_type_has_default(ty: &TypeRef, known_default_types: &std::collections::HashSet<&str>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        // Optional<T> defaults to None regardless of T
        TypeRef::Optional(inner) => field_type_has_default(inner, known_default_types),
        // Vec<T> defaults to empty vec regardless of T
        TypeRef::Vec(inner) => field_type_has_default(inner, known_default_types),
        // Map<K, V> defaults to empty map regardless of K/V
        TypeRef::Map(k, v) => {
            field_type_has_default(k, known_default_types) && field_type_has_default(v, known_default_types)
        }
        // Named types only have Default if marked with has_default=true
        TypeRef::Named(name) => known_default_types.contains(name.as_str()),
    }
}

/// Check if any two field names are similar enough to trigger clippy::similar_names.
/// This detects patterns like "sub_symbol" and "sup_symbol" (differ by 1-2 chars).
fn has_similar_names(names: &[&String]) -> bool {
    for (i, &name1) in names.iter().enumerate() {
        for &name2 in &names[i + 1..] {
            // Simple heuristic: if names differ by <= 2 characters and have same length, flag it
            if name1.len() == name2.len() && diff_count(name1, name2) <= 2 {
                return true;
            }
        }
    }
    false
}

/// Count how many characters differ between two strings of equal length.
fn diff_count(s1: &str, s2: &str) -> usize {
    s1.chars().zip(s2.chars()).filter(|(c1, c2)| c1 != c2).count()
}

/// Check if a TypeRef references an opaque type, including through Optional and Vec wrappers.
/// Opaque types use Arc<T> which doesn't implement Serialize/Deserialize, so any struct with
/// such a field cannot derive those traits.
pub fn field_references_opaque_type(ty: &TypeRef, opaque_names: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => opaque_names.contains(name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_opaque_type(inner, opaque_names),
        TypeRef::Map(k, v) => {
            field_references_opaque_type(k, opaque_names) || field_references_opaque_type(v, opaque_names)
        }
        _ => false,
    }
}

/// Generate a struct definition using the builder, with a per-field attribute callback.
///
/// `extra_field_attrs` is called for each field and returns additional `#[...]` attributes to
/// prepend (beyond `cfg.field_attrs`). Pass `|_| vec![]` to use the default behaviour.
pub fn gen_struct_with_per_field_attrs(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    extra_field_attrs: impl Fn(&alef_core::ir::FieldDef) -> Vec<String>,
) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

    // Check if struct has similar field names (e.g., sub_symbol and sup_symbol)
    let field_names: Vec<_> = typ.fields.iter().filter(|f| f.cfg.is_none()).map(|f| &f.name).collect();
    if has_similar_names(&field_names) {
        sb.add_attr("allow(clippy::similar_names)");
    }

    for d in cfg.struct_derives {
        sb.add_derive(d);
    }
    // Track which fields are opaque so we can conditionally skip derives and add #[serde(skip)].
    let opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| f.cfg.is_none() && field_references_opaque_type(&f.ty, cfg.opaque_type_names))
        .map(|f| f.name.as_str())
        .collect();
    // Only derive Default/Serialize/Deserialize when no fields reference opaque types.
    // Opaque types (data enums, Arc-wrapped handles) don't implement these traits.
    if opaque_fields.is_empty() {
        sb.add_derive("Default");
        sb.add_derive("serde::Serialize");
        sb.add_derive("serde::Deserialize");
    }
    for field in &typ.fields {
        if field.cfg.is_some() {
            continue;
        }
        let force_optional = cfg.option_duration_on_defaults
            && typ.has_default
            && !field.optional
            && matches!(field.ty, TypeRef::Duration);
        let ty = if (field.optional || force_optional) && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            // field.ty is already Optional(T) — mapped type is already Option<T>, don't double-wrap
            mapper.map_type(&field.ty)
        };
        let mut attrs: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
        attrs.extend(extra_field_attrs(field));
        // Add #[serde(skip)] for fields that reference opaque types so the struct
        // remains serializable for the serde recovery path
        if opaque_fields.contains(&field.name.as_str()) {
            attrs.push("serde(skip)".to_string());
        }
        sb.add_field_with_doc(&field.name, &ty, attrs, &field.doc);
    }
    sb.build()
}

/// Generate a struct definition using the builder.
pub fn gen_struct(typ: &TypeDef, mapper: &dyn TypeMapper, cfg: &RustBindingConfig) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

    // Check if struct has similar field names (e.g., sub_symbol and sup_symbol)
    let field_names: Vec<_> = typ.fields.iter().filter(|f| f.cfg.is_none()).map(|f| &f.name).collect();
    if has_similar_names(&field_names) {
        sb.add_attr("allow(clippy::similar_names)");
    }

    for d in cfg.struct_derives {
        sb.add_derive(d);
    }
    let opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| f.cfg.is_none() && field_references_opaque_type(&f.ty, cfg.opaque_type_names))
        .map(|f| f.name.as_str())
        .collect();
    if opaque_fields.is_empty() {
        sb.add_derive("Default");
        sb.add_derive("serde::Serialize");
        sb.add_derive("serde::Deserialize");
    }
    for field in &typ.fields {
        // Skip cfg-gated fields — they depend on features that may not be enabled
        // for this binding crate. Including them would require the binding struct to
        // handle conditional compilation which struct literal initializers can't express.
        if field.cfg.is_some() {
            continue;
        }
        // When option_duration_on_defaults is set, wrap non-optional Duration fields in
        // Option<u64> for has_default types so the binding constructor can accept None
        // and the From conversion falls back to the core type's Default.
        let force_optional = cfg.option_duration_on_defaults
            && typ.has_default
            && !field.optional
            && matches!(field.ty, TypeRef::Duration);
        let ty = if (field.optional || force_optional) && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            // field.ty is already Optional(T) — mapped type is already Option<T>, don't double-wrap
            mapper.map_type(&field.ty)
        };
        let mut attrs: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
        // Only add #[serde(default)] when serde derives are present on the struct
        // (opaque_fields empty = serde derives added, opaque field needs serde(default))
        // This can't happen: if opaque_fields is empty, no field matches this check.
        // If opaque_fields is non-empty, serde derives were suppressed → skip serde attr.
        // So this block is effectively dead — remove it to prevent stale serde attrs.
        sb.add_field_with_doc(&field.name, &ty, attrs, &field.doc);
    }
    sb.build()
}

/// Generate a `Default` impl for a non-opaque binding struct with `has_default`.
/// All fields use their type's Default::default().
/// Optional fields use None instead of Default::default().
/// This enables the struct to be used with `unwrap_or_default()` in config constructors.
///
/// WARNING: This assumes all field types implement Default. If a Named field type
/// doesn't implement Default, this impl will fail to compile. Callers should verify
/// that the struct's fields can be safely defaulted before calling this function.
pub fn gen_struct_default_impl(typ: &TypeDef, name_prefix: &str) -> String {
    let full_name = format!("{}{}", name_prefix, typ.name);
    let mut out = String::with_capacity(256);
    writeln!(out, "impl Default for {} {{", full_name).ok();
    writeln!(out, "    fn default() -> Self {{").ok();
    writeln!(out, "        Self {{").ok();
    for field in &typ.fields {
        if field.cfg.is_some() {
            continue;
        }
        let default_val = match &field.ty {
            TypeRef::Optional(_) => "None".to_string(),
            _ => "Default::default()".to_string(),
        };
        writeln!(out, "            {}: {},", field.name, default_val).ok();
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

/// Check if any method on a type takes `&mut self`, meaning the opaque wrapper
/// must use `Arc<Mutex<T>>` instead of `Arc<T>` to allow interior mutability.
pub fn type_needs_mutex(typ: &TypeDef) -> bool {
    typ.methods
        .iter()
        .any(|m| m.receiver == Some(alef_core::ir::ReceiverKind::RefMut))
}

/// Generate an opaque wrapper struct with `inner: Arc<core::Type>`.
/// For trait types, uses `Arc<dyn Type + Send + Sync>`.
/// For types with `&mut self` methods, uses `Arc<Mutex<core::Type>>`.
///
/// Special case: if ALL methods on this type are sanitized, the type was created by the
/// impl-block fallback for a generic core type (e.g. `GraphQLExecutor<Q,M,S>`). Sanitized
/// methods never access `self.inner` (they emit `gen_unimplemented_body`), so we omit the
/// `inner` field entirely. This avoids generating `Arc<CoreType>` with missing generic
/// parameters, which would fail to compile.
pub fn gen_opaque_struct(typ: &TypeDef, cfg: &RustBindingConfig) -> String {
    let needs_mutex = type_needs_mutex(typ);
    // Omit the inner field only when the rust_path contains generic type parameters
    // (angle brackets), which means the concrete types are unknown at codegen time and
    // `Arc<CoreType<_, _, _>>` would fail to compile. This typically occurs for types
    // created from a generic impl block where all methods are sanitized.
    // We do NOT omit inner solely because all_methods_sanitized is true: even when no
    // methods delegate to self.inner, the inner field may be required by From impls
    // generated for non-opaque structs that have this type as a field.
    let core_path = typ.rust_path.replace('-', "_");
    let has_unresolvable_generics = core_path.contains('<');
    let all_methods_sanitized = !typ.methods.is_empty() && typ.methods.iter().all(|m| m.sanitized);
    let omit_inner = all_methods_sanitized && has_unresolvable_generics;
    let mut out = String::with_capacity(512);
    if !cfg.struct_derives.is_empty() {
        writeln!(out, "#[derive(Clone)]").ok();
    }
    for attr in cfg.struct_attrs {
        writeln!(out, "#[{attr}]").ok();
    }
    writeln!(out, "pub struct {} {{", typ.name).ok();
    if !omit_inner {
        if typ.is_trait {
            writeln!(out, "    inner: Arc<dyn {core_path} + Send + Sync>,").ok();
        } else if needs_mutex {
            writeln!(out, "    inner: Arc<std::sync::Mutex<{core_path}>>,").ok();
        } else {
            writeln!(out, "    inner: Arc<{core_path}>,").ok();
        }
    }
    write!(out, "}}").ok();
    out
}

/// Generate an opaque wrapper struct with `inner: Arc<core::Type>` and a name prefix.
/// For types with `&mut self` methods, uses `Arc<Mutex<core::Type>>`.
///
/// Special case: if ALL methods on this type are sanitized, omit the `inner` field.
/// See `gen_opaque_struct` for the rationale.
pub fn gen_opaque_struct_prefixed(typ: &TypeDef, cfg: &RustBindingConfig, prefix: &str) -> String {
    let needs_mutex = type_needs_mutex(typ);
    let core_path = typ.rust_path.replace('-', "_");
    let has_unresolvable_generics = core_path.contains('<');
    let all_methods_sanitized = !typ.methods.is_empty() && typ.methods.iter().all(|m| m.sanitized);
    let omit_inner = all_methods_sanitized && has_unresolvable_generics;
    let mut out = String::with_capacity(512);
    if !cfg.struct_derives.is_empty() {
        writeln!(out, "#[derive(Clone)]").ok();
    }
    for attr in cfg.struct_attrs {
        writeln!(out, "#[{attr}]").ok();
    }
    writeln!(out, "pub struct {}{} {{", prefix, typ.name).ok();
    if !omit_inner {
        if typ.is_trait {
            writeln!(out, "    inner: Arc<dyn {core_path} + Send + Sync>,").ok();
        } else if needs_mutex {
            writeln!(out, "    inner: Arc<std::sync::Mutex<{core_path}>>,").ok();
        } else {
            writeln!(out, "    inner: Arc<{core_path}>,").ok();
        }
    }
    write!(out, "}}").ok();
    out
}
