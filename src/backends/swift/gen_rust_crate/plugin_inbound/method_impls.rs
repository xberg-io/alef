use crate::backends::swift::gen_rust_crate::type_bridge::swift_bridge_rust_type;
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeRef};
use heck::ToSnakeCase;

use super::{inbound_bridge_type, is_optional_vec_of_named, is_vec_of_named, needs_inbound_json_bridge};

/// Returns true if `ty` references a `Named(name)` at any depth where `name` resolves
/// to a trait — either present in `api.types` or stripped from the binding surface
/// (`api.excluded_trait_names`). Such methods return references to trait objects
/// (`&dyn Trait`, `Option<&dyn Trait>`, `Box<dyn Trait>`) which the Rust IR flattens
/// to `Named(name)`. They cannot be bridged across the Swift FFI, so the trait-bridge
/// generator skips them and falls back to the trait's default impl.
fn return_type_references_trait(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(name) => {
            api.types.iter().any(|t| t.is_trait && &t.name == name) || api.excluded_trait_names.contains(name)
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => return_type_references_trait(inner, api),
        TypeRef::Map(k, v) => return_type_references_trait(k, api) || return_type_references_trait(v, api),
        _ => false,
    }
}

/// Emit one `impl Trait for SwiftWrapper` method body.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_inbound_method_impl(
    out: &mut String,
    method: &MethodDef,
    trait_snake: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    emit_plugin: bool,
    lifetime_type_names: &std::collections::HashSet<String>,
    api: &ApiSurface,
) {
    if emit_plugin && return_type_references_trait(&method.return_type, api) {
        return;
    }

    let method_snake = method.name.to_snake_case();

    let receiver_token = match &method.receiver {
        Some(crate::core::ir::ReceiverKind::RefMut) => "&mut self",
        Some(crate::core::ir::ReceiverKind::Owned) => "self",
        _ => "&self",
    };
    let mut sig_params = vec![receiver_token.to_string()];
    for p in &method.params {
        let mut prefix = String::new();
        if p.is_ref {
            prefix.push('&');
        }
        if p.is_mut {
            prefix.push_str("mut ");
        }
        let inner_ty = if p.is_ref {
            match &p.ty {
                TypeRef::Vec(inner) => {
                    let elem = inbound_native_ty_owned(inner, source_crate, type_paths);
                    format!("[{elem}]")
                }
                TypeRef::Named(name) => {
                    let base = resolve_named_path(name, source_crate, type_paths);
                    if lifetime_type_names.contains(name.as_str()) {
                        format!("{base}<'_>")
                    } else {
                        base
                    }
                }
                other => inbound_native_ty(other, source_crate, type_paths),
            }
        } else {
            inbound_native_ty_owned(&p.ty, source_crate, type_paths)
        };
        let full_ty = if p.optional {
            format!("Option<{prefix}{inner_ty}>")
        } else {
            format!("{prefix}{inner_ty}")
        };
        sig_params.push(format!("{}: {full_ty}", p.name.to_snake_case()));
    }

    let return_ty = inbound_impl_return_type(method, source_crate, type_paths, error_type);

    let async_kw = if method.is_async { "async " } else { "" };
    let params = sig_params.join(", ");
    out.push_str(&crate::backends::swift::template_env::render(
        "inbound_method_open.rs.jinja",
        crate::alef_context! {
            async_kw => async_kw,
            method_snake => &method_snake,
            params => &params,
            return_ty => &return_ty,
        },
    ));

    for p in &method.params {
        if let Some(line) = inbound_param_to_bridge(p) {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_binding.rs.jinja",
                crate::alef_context! {
                    line => &line,
                },
            ));
        }
    }

    let call_args: Vec<String> = method.params.iter().map(inbound_local_name).collect();
    let call_expr = format!("self.inner.alef_{method_snake}({})", call_args.join(", "));

    let is_mime_types_pattern = method.returns_ref
        && matches!(&method.return_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));

    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_result_unit.rs.jinja",
                crate::alef_context! {
                    call_expr => &call_expr,
                },
            ));
        } else {
            let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_result_value.rs.jinja",
                crate::alef_context! {
                    call_expr => &call_expr,
                    native_ty => &native_ty,
                },
            ));
        }
    } else if is_mime_types_pattern {
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_mime_types.rs.jinja",
            crate::alef_context! {
                call_expr => &call_expr,
            },
        ));
    } else if is_vec_of_named(&method.return_type) {
        // alef-tasks #308: `Vec<Named>` crosses per-element (`Vec<String>`), not as one JSON
        // blob, so it needs its own decode -- `needs_inbound_json_bridge` deliberately excludes
        // this shape (see the rule in `plugin_inbound::inbound_bridge_type`). `call_expr` really
        // returns `Vec<String>` here (matching `inbound_bridge_type`'s extern-block type), and
        // each element is decoded on its own, then collected into the native `Vec<T>`. ~keep
        let elem_native_ty = match &method.return_type {
            TypeRef::Vec(inner) => inbound_native_return_ty(inner, source_crate, type_paths),
            _ => unreachable!("is_vec_of_named guarantees a Vec"),
        };
        let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_json_vec_return.rs.jinja",
            crate::alef_context! {
                call_expr => &call_expr,
                elem_native_ty => &elem_native_ty,
                native_ty => &native_ty,
                trait_snake => trait_snake,
                method_snake => &method_snake,
            },
        ));
    } else if is_optional_vec_of_named(&method.return_type) {
        // alef-tasks #333: `Optional<Vec<Named>>` extends the #308 per-element rule through the
        // `Option` wrapper -- `inbound_bridge_type` declares `Option<Vec<String>>` for the extern
        // block, so `call_expr` really returns that shape here. `needs_inbound_json_bridge`
        // deliberately excludes this shape too (its blanket `Optional(_) => true` would otherwise
        // route it into the single-blob branch below, which expects `call_expr` to be a bare
        // `String` and would not compile against an `Option<Vec<String>>` value). `None` passes
        // through unchanged; each `Some` element is decoded on its own. See the rule in
        // `plugin_inbound::inbound_bridge_type`. ~keep
        let inner_vec = match &method.return_type {
            TypeRef::Optional(inner) => inner.as_ref(),
            _ => unreachable!("is_optional_vec_of_named guarantees an Optional"),
        };
        let elem_native_ty = match inner_vec {
            TypeRef::Vec(inner) => inbound_native_return_ty(inner, source_crate, type_paths),
            _ => unreachable!("is_optional_vec_of_named guarantees an inner Vec"),
        };
        let vec_native_ty = format!("Vec<{elem_native_ty}>");
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_json_optional_vec_return.rs.jinja",
            crate::alef_context! {
                call_expr => &call_expr,
                elem_native_ty => &elem_native_ty,
                vec_native_ty => &vec_native_ty,
                trait_snake => trait_snake,
                method_snake => &method_snake,
            },
        ));
    } else if needs_inbound_json_bridge(&method.return_type) {
        let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_json_return.rs.jinja",
            crate::alef_context! {
                call_expr => &call_expr,
                native_ty => &native_ty,
                trait_snake => trait_snake,
                method_snake => &method_snake,
            },
        ));
    } else {
        match &method.return_type {
            TypeRef::Unit => out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_unit_call.rs.jinja",
                crate::alef_context! {
                    call_expr => &call_expr,
                },
            )),
            _ => out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_value_call.rs.jinja",
                crate::alef_context! {
                    call_expr => &call_expr,
                },
            )),
        }
    }

    out.push_str("    }\n\n");
}

/// Convert a trait param into its bridged FFI form via a `let` binding when needed.
fn inbound_param_to_bridge(p: &ParamDef) -> Option<String> {
    let local = inbound_local_name(p);
    let name = p.name.to_snake_case();

    if is_vec_of_named(&p.ty) {
        // alef-tasks #308: mirror of the per-element return decode above. The extern "Swift"
        // declaration expects `Vec<String>` (`inbound_bridge_type`), so each native element is
        // encoded on its own before the call, rather than serializing the whole Vec as one blob.
        return Some(inbound_vec_named_param_bridge(p));
    }

    if needs_inbound_json_bridge(&p.ty) {
        if p.optional {
            return Some(format!(
                "let {local} = {name}.map(|v| ::serde_json::to_string(&v).expect(\"serializable param {name}\"));"
            ));
        }
        return Some(format!(
            "let {local} = ::serde_json::to_string(&{name}).expect(\"serializable param {name}\");"
        ));
    }

    if p.optional {
        return match &p.ty {
            TypeRef::Path => Some(format!(
                "let {local} = {name}.map(|v| v.to_string_lossy().into_owned());"
            )),
            TypeRef::Bytes if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            TypeRef::String if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_string());")),
            TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            _ => None,
        };
    }

    match &p.ty {
        TypeRef::Path => Some(format!("let {local} = {name}.to_string_lossy().into_owned();")),
        TypeRef::Bytes => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_vec();"))
            } else {
                None
            }
        }
        TypeRef::String => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_string();"))
            } else {
                None
            }
        }
        TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.to_vec();")),
        _ => None,
    }
}

/// Emit the `let` binding that encodes a `Vec<Named>` param element-wise into `Vec<String>`,
/// matching the `Vec<String>` the extern "Swift" declaration expects (`inbound_bridge_type`).
/// A single `serde_json::to_string` of the whole `Vec` would produce one `String`, not a
/// `Vec<String>`, and would not compile against the extern block's param type. ~keep
fn inbound_vec_named_param_bridge(p: &ParamDef) -> String {
    let local = inbound_local_name(p);
    let name = p.name.to_snake_case();
    let iter_method = if p.is_ref { "iter" } else { "into_iter" };
    let elem_encode = if p.is_ref {
        "::serde_json::to_string(e)"
    } else {
        "::serde_json::to_string(&e)"
    };
    if p.optional {
        format!(
            "let {local} = {name}.map(|v| v.{iter_method}().map(|e| {elem_encode}.expect(\"serializable param {name} element\")).collect::<Vec<String>>());"
        )
    } else {
        format!(
            "let {local} = {name}.{iter_method}().map(|e| {elem_encode}.expect(\"serializable param {name} element\")).collect::<Vec<String>>();"
        )
    }
}

fn inbound_local_name(p: &ParamDef) -> String {
    p.name.to_snake_case()
}

/// FFI shim return type for `extern "Swift"` declarations.
///
/// Returns `String` for fallible methods (carrying a JSON envelope `{"ok": ...}` /
/// `{"err": "..."}`) instead of `Result<T, String>`. swift-bridge 0.1.59's
/// `Result<RustString, RustString>` codegen has a bug — `convert_ffi_result_ok_value_to_rust_value`
/// emits `result.ok_or_err` on a bare `*mut RustString` instead of the `ResultPtrAndPtr`
/// wrapper, producing `error[E0609]: no field 'ok_or_err' on type '*mut RustString'`.
/// Encoding the result as a JSON envelope sidesteps the limitation while preserving the
/// error-channel semantics; the Rust-side wrapper deserialises and reconstitutes the
/// `Result` after the FFI call.
pub(super) fn inbound_return_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
        return "String".to_string();
    }
    inbound_bridge_type(&method.return_type)
}

fn inbound_impl_return_type(
    method: &MethodDef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
) -> String {
    if method.returns_ref
        && let TypeRef::Vec(inner) = &method.return_type
    {
        let elem = match inner.as_ref() {
            TypeRef::String => "&'static str".to_string(),
            other => inbound_native_ty(other, source_crate, type_paths),
        };
        return format!("&'static [{elem}]");
    }

    let inner = inbound_native_ty_owned(&method.return_type, source_crate, type_paths);
    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            result_type(source_crate, error_type, "()")
        } else {
            result_type(source_crate, error_type, &inner)
        }
    } else {
        inner
    }
}

pub(super) fn result_type(source_crate: &str, error_type: &str, ok_type: &str) -> String {
    format!(
        "std::result::Result<{ok_type}, {}>",
        error_type_path(source_crate, error_type)
    )
}

pub(super) fn error_type_path(source_crate: &str, error_type: &str) -> String {
    if error_type.contains("::") || error_type.contains('<') {
        error_type.to_string()
    } else {
        format!("{source_crate}::{error_type}")
    }
}

/// Resolve a Named type to its fully-qualified Rust path. Falls back to `{source_crate}::{name}`
/// when the lookup misses (covers shared types declared at the crate root).
fn resolve_named_path(
    name: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(path) = type_paths.get(name) {
        return path.replace('-', "_");
    }
    format!("{source_crate}::{name}")
}

/// Render the owned native return type (used in JSON-deserialise calls). Named types are
/// resolved via `type_paths`. Inner types in containers use the owned form.
fn inbound_native_return_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_return_ty(k, source_crate, type_paths),
            inbound_native_return_ty(v, source_crate, type_paths)
        ),
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Render a TypeRef in its native (non-bridged) Rust form, qualifying Named types via
/// `type_paths`. Used for the `impl Trait` signature.
fn inbound_native_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Unit => "()".to_string(),
        TypeRef::String => "str".to_string(),
        TypeRef::Bytes => "[u8]".to_string(),
        TypeRef::Path => "::std::path::Path".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Json => "::serde_json::Value".to_string(),
        TypeRef::Duration => "::std::time::Duration".to_string(),
        TypeRef::Primitive(p) => primitive_str(p).to_string(),
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_ty_owned(k, source_crate, type_paths),
            inbound_native_ty_owned(v, source_crate, type_paths)
        ),
    }
}

/// Owned form (for use inside `Vec`/`Option`/`HashMap`): swap unsized types (`str`,
/// `[u8]`, `Path`) with their owned equivalents.
fn inbound_native_ty_owned(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => inbound_native_ty(ty, source_crate, type_paths),
    }
}

fn primitive_str(p: &crate::core::ir::PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType::*;
    match p {
        Bool => "bool",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        Isize => "isize",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        Usize => "usize",
        F32 => "f32",
        F64 => "f64",
    }
}

/// UNVERIFIED end-to-end by any compile gate: `trait_box_swiftpm_compile.rs` only compiles the
/// generator's outbound `gen_bindings` output, never the `gen_rust_crate` Rust source these
/// functions produce. These unit tests are the only coverage for alef-tasks #308's fix. ~keep
#[cfg(test)]
mod tests {
    use super::*;

    fn named_vec_param(name: &str, type_name: &str, optional: bool, is_ref: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named(type_name.to_string()))),
            optional,
            is_ref,
            ..Default::default()
        }
    }

    #[test]
    fn test_param_to_bridge_vec_named_encodes_per_element_owned() {
        let p = named_vec_param("entries", "SinkStats", false, false);
        let line = inbound_param_to_bridge(&p).expect("Vec<Named> param needs a bridge line");

        assert_eq!(
            line,
            "let entries = entries.into_iter().map(|e| ::serde_json::to_string(&e)\
             .expect(\"serializable param entries element\")).collect::<Vec<String>>();"
        );
    }

    #[test]
    fn test_param_to_bridge_vec_named_encodes_per_element_ref() {
        let p = named_vec_param("entries", "SinkStats", false, true);
        let line = inbound_param_to_bridge(&p).expect("Vec<Named> param needs a bridge line");

        assert!(
            line.contains(".iter().map(|e| ::serde_json::to_string(e)"),
            "got: {line}"
        );
        assert!(
            !line.contains("to_string(&e)"),
            "a ref element must not be re-borrowed: {line}"
        );
    }

    #[test]
    fn test_param_to_bridge_vec_named_encodes_per_element_optional() {
        let p = named_vec_param("entries", "SinkStats", true, false);
        let line = inbound_param_to_bridge(&p).expect("Vec<Named> param needs a bridge line");

        assert!(
            line.starts_with("let entries = entries.map(|v| v.into_iter().map(|e|"),
            "got: {line}"
        );
        assert!(line.ends_with("collect::<Vec<String>>());"), "got: {line}");
    }

    /// A whole-Vec `serde_json::to_string` would produce one `String`, not the `Vec<String>`
    /// the extern "Swift" declaration expects (alef-tasks #308).
    #[test]
    fn test_param_to_bridge_vec_named_is_not_a_single_blob() {
        let p = named_vec_param("entries", "SinkStats", false, false);
        let line = inbound_param_to_bridge(&p).expect("Vec<Named> param needs a bridge line");

        assert!(
            !line.contains("::serde_json::to_string(&entries)"),
            "must not serialize the whole Vec as one blob: {line}"
        );
    }

    fn stats_history_method() -> MethodDef {
        MethodDef {
            name: "stats_history".to_string(),
            return_type: TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
            ..Default::default()
        }
    }

    /// alef-tasks #308: before the fix, `needs_inbound_json_bridge(Vec<Named>)` was `false`, so
    /// the method fell through to a bare passthrough of `call_expr` -- which the extern block
    /// declares `Vec<String>` -- into an impl return type of `Vec<SinkStats>`. That is a type
    /// mismatch the SwiftPM gate cannot see because it never compiles this generator's Rust
    /// output.
    #[test]
    fn test_emit_inbound_method_impl_vec_named_return_decodes_per_element() {
        let method = stats_history_method();
        let api = ApiSurface::default();
        let mut out = String::new();

        emit_inbound_method_impl(
            &mut out,
            &method,
            "document_sink",
            "fixture_core",
            &std::collections::HashMap::new(),
            "SinkError",
            false,
            &std::collections::HashSet::new(),
            &api,
        );

        assert!(
            out.contains("__strings"),
            "expected the per-element decode helper, got:\n{out}"
        );
        assert!(
            out.contains("::serde_json::from_str::<fixture_core::SinkStats>(&s)"),
            "expected each element decoded on its own, got:\n{out}"
        );
        assert!(
            out.contains(".collect::<Vec<fixture_core::SinkStats>>()"),
            "expected the collected native Vec<T>, got:\n{out}"
        );
        assert!(
            !out.contains("let json = self.inner.alef_stats_history();"),
            "must not treat Vec<String> as a single JSON blob, got:\n{out}"
        );
    }

    fn optional_stats_history_method() -> MethodDef {
        MethodDef {
            name: "stats_history".to_string(),
            return_type: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
                "SinkStats".to_string(),
            ))))),
            ..Default::default()
        }
    }

    /// alef-tasks #333: before the fix, `is_vec_of_named(Optional<Vec<Named>>)` was `false` (it
    /// only matches a bare `Vec<Named>`), so the method fell through to
    /// `needs_inbound_json_bridge` -- `true` for any `Optional(_)` -- and rendered
    /// `inbound_method_json_return.rs.jinja`, which binds `call_expr` to a `let json: String`
    /// and calls `serde_json::from_str` on it. The extern block declares
    /// `Option<Vec<String>>` for this shape (`inbound_bridge_type`), not `String`, so `let json
    /// = self.inner.alef_stats_history();` would not type-check. This is exactly the kind of
    /// generator-output type mismatch the SwiftPM compile gate cannot see, since that gate never
    /// compiles this generator's Rust output -- only these string assertions catch it.
    #[test]
    fn test_emit_inbound_method_impl_optional_vec_named_return_decodes_per_element() {
        let method = optional_stats_history_method();
        let api = ApiSurface::default();
        let mut out = String::new();

        emit_inbound_method_impl(
            &mut out,
            &method,
            "document_sink",
            "fixture_core",
            &std::collections::HashMap::new(),
            "SinkError",
            false,
            &std::collections::HashSet::new(),
            &api,
        );

        assert!(
            out.contains("__strings"),
            "expected the per-element decode helper, got:\n{out}"
        );
        assert!(
            out.contains(".map(|v| {"),
            "expected the Option::map wrapper around the per-element decode, got:\n{out}"
        );
        assert!(
            out.contains("::serde_json::from_str::<fixture_core::SinkStats>(&s)"),
            "expected each present element decoded on its own, got:\n{out}"
        );
        assert!(
            out.contains(".collect::<Vec<fixture_core::SinkStats>>()"),
            "expected the collected native Vec<T> inside the Option, got:\n{out}"
        );
        assert!(
            !out.contains("let json = self.inner.alef_stats_history();"),
            "must not treat Option<Vec<String>> as a single JSON blob, got:\n{out}"
        );
    }
}
