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
    // Always derive Default/Serialize/Deserialize. Opaque fields get #[serde(skip)]
    // so they use Default::default() during deserialization. This is needed for the
    // serde recovery path where binding types round-trip through JSON.
    sb.add_derive("Default");
    sb.add_derive("serde::Serialize");
    sb.add_derive("serde::Deserialize");
    let has_serde = true;
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
        // Add #[serde(skip)] for opaque fields or sanitized fields when the struct derives serde.
        // Opaque fields use Arc<T> which is not Serialize/Deserialize.
        // Sanitized fields have types replaced with String placeholders (e.g. CancellationToken →
        // String, OutputFormat → String) — including them in serde JSON round-trips causes
        // "unknown field" or "unknown variant" errors at runtime.
        if has_serde && (opaque_fields.contains(&field.name.as_str()) || field.sanitized) {
            attrs.push("serde(skip)".to_string());
        }
        sb.add_field_with_doc(&field.name, &ty, attrs, &field.doc);
    }
    sb.build()
}

/// Generate a struct definition using the builder, with per-field attribute and name override callbacks.
///
/// This is the most flexible variant.  Use it when the target language may need to escape
/// reserved keywords in field names (e.g. Python's `class` → `class_`).
///
/// * `extra_field_attrs` — called per field, returns additional `#[…]` attribute strings to
///   append **after** `cfg.field_attrs`.  Return an empty vec for the default behaviour.
/// * `field_name_override` — called per field, returns `Some(escaped_name)` when the Rust
///   binding struct field name should differ from `field.name` (e.g. for keyword escaping),
///   or `None` to keep the original name.
///
/// When a field name is overridden the caller is responsible for adding the appropriate
/// language attribute (e.g. `pyo3(get, name = "original")`) via `extra_field_attrs`.
/// `cfg.field_attrs` is **still** applied for non-renamed fields; for renamed fields the
/// caller should replace the default field attrs entirely by returning them from
/// `extra_field_attrs` and passing a modified `cfg` with empty `field_attrs`.
pub fn gen_struct_with_rename(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    extra_field_attrs: impl Fn(&alef_core::ir::FieldDef) -> Vec<String>,
    field_name_override: impl Fn(&alef_core::ir::FieldDef) -> Option<String>,
) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

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
    sb.add_derive("Default");
    sb.add_derive("serde::Serialize");
    sb.add_derive("serde::Deserialize");
    let has_serde = true;
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
            mapper.map_type(&field.ty)
        };
        let name_override = field_name_override(field);
        let extra_attrs = extra_field_attrs(field);
        // When the field name is overridden (keyword-escaped), skip cfg.field_attrs so the
        // caller's extra_field_attrs callback can supply the full replacement attr set
        // (e.g. `pyo3(get, name = "class")` instead of the default `pyo3(get)`).
        let mut attrs: Vec<String> = if name_override.is_some() && !extra_attrs.is_empty() {
            extra_attrs
        } else {
            let mut a: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
            a.extend(extra_attrs);
            a
        };
        // Add #[serde(skip)] for opaque fields or sanitized fields — same rationale as in
        // gen_struct_with_per_field_attrs: sanitized fields have placeholder String types that
        // cause JSON round-trip failures with "unknown variant ''" errors.
        if has_serde && (opaque_fields.contains(&field.name.as_str()) || field.sanitized) {
            attrs.push("serde(skip)".to_string());
        }
        let emit_name = name_override.unwrap_or_else(|| field.name.clone());
        sb.add_field_with_doc(&emit_name, &ty, attrs, &field.doc);
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
    let _opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| f.cfg.is_none() && field_references_opaque_type(&f.ty, cfg.opaque_type_names))
        .map(|f| f.name.as_str())
        .collect();
    sb.add_derive("Default");
    sb.add_derive("serde::Serialize");
    sb.add_derive("serde::Deserialize");
    let _has_serde = true;
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
        let attrs: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
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

/// Check if a type wrapping `Arc<Mutex<T>>` should use `tokio::sync::Mutex` instead
/// of `std::sync::Mutex` because every `&mut self` method is `async`.
///
/// `std::sync::MutexGuard` is `!Send`, so holding a guard across `.await` makes the
/// surrounding future `!Send`, which fails to compile in PyO3 / NAPI-RS bindings that
/// require `Send` futures. `tokio::sync::MutexGuard` IS `Send`, so swapping the lock
/// type fixes the entire async-locking story for these structs.
///
/// The condition is tight: every method that takes `&mut self` MUST be async. If even
/// one sync method takes `&mut self`, switching to `tokio::sync::Mutex` would break
/// it (since `tokio::sync::Mutex::lock()` returns a `Future` and cannot be awaited
/// from sync context). In that mixed case we keep `std::sync::Mutex`.
pub fn type_needs_tokio_mutex(typ: &TypeDef) -> bool {
    use alef_core::ir::ReceiverKind;
    if !type_needs_mutex(typ) {
        return false;
    }
    let refmut_methods = typ.methods.iter().filter(|m| m.receiver == Some(ReceiverKind::RefMut));
    let mut any = false;
    for m in refmut_methods {
        any = true;
        if !m.is_async {
            return false;
        }
    }
    any
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

#[cfg(test)]
mod tests {
    use super::{type_needs_mutex, type_needs_tokio_mutex};
    use alef_core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};

    fn method(name: &str, receiver: Option<ReceiverKind>, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.into(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn type_with_methods(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.into(),
            rust_path: format!("my_crate::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }
    }

    #[test]
    fn tokio_mutex_when_all_refmut_methods_async() {
        let typ = type_with_methods(
            "WebSocketConnection",
            vec![
                method("send_text", Some(ReceiverKind::RefMut), true),
                method("receive_text", Some(ReceiverKind::RefMut), true),
                method("close", None, true),
            ],
        );
        assert!(type_needs_mutex(&typ));
        assert!(type_needs_tokio_mutex(&typ));
    }

    #[test]
    fn no_tokio_mutex_when_any_refmut_is_sync() {
        let typ = type_with_methods(
            "Mixed",
            vec![
                method("async_op", Some(ReceiverKind::RefMut), true),
                method("sync_op", Some(ReceiverKind::RefMut), false),
            ],
        );
        assert!(type_needs_mutex(&typ));
        assert!(!type_needs_tokio_mutex(&typ));
    }

    #[test]
    fn no_tokio_mutex_when_no_refmut() {
        let typ = type_with_methods("ReadOnly", vec![method("get", Some(ReceiverKind::Ref), true)]);
        assert!(!type_needs_mutex(&typ));
        assert!(!type_needs_tokio_mutex(&typ));
    }

    #[test]
    fn no_tokio_mutex_when_empty_methods() {
        let typ = type_with_methods("Empty", vec![]);
        assert!(!type_needs_mutex(&typ));
        assert!(!type_needs_tokio_mutex(&typ));
    }
}
