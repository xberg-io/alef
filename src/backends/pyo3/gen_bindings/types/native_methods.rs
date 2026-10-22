//! Native instance methods that a public `options.py` dataclass forwards to its native class.
//!
//! A dataclass keeps the fields of the native `#[pyclass]` it shadows. Every other method of
//! the native class is missing from it unless it is forwarded here, so a caller who parses a
//! reply and reads `message.text()` gets an `AttributeError` although the native class, the
//! `.pyi` stub and every other language binding have the method. A forwarded method converts
//! `self` to the native object with the same `_to_rust_*` converter `api.py` uses for function
//! arguments, calls the native method, and converts the result back with the `_from_native_*`
//! converters of this file.

use super::*;
use crate::core::ir::{MethodDef, ReceiverKind, TypeRef};
use heck::ToSnakeCase;

/// True when `options.py` renders a `@dataclass` body for `typ`: the same test the rendering
/// loop of `gen_options_py` applies, so a converter or an import is never asked for a class
/// the file does not define.
pub(in crate::backends::pyo3::gen_bindings) fn renders_dataclass(
    typ: &TypeDef,
    input_names: &std::collections::HashSet<String>,
    published_return_names: &std::collections::HashSet<String>,
    reexported_types: &[String],
) -> bool {
    !typ.is_trait
        && !reexported_types.contains(&typ.name)
        && !typ.name.ends_with("Update")
        && (typ.has_default || input_names.contains(&typ.name))
        && (!typ.is_return_type || published_return_names.contains(&typ.name))
}

/// Names of the dataclasses that forward at least one native method.
///
/// `api.py` must emit a `_to_rust_*` converter for each of these, and for every dataclass
/// reachable through their fields, so the set is computed once and read by both generators.
pub(in crate::backends::pyo3::gen_bindings) fn delegating_dataclass_names(
    api: &ApiSurface,
    dto: &DtoConfig,
    reexported_types: &[String],
) -> AHashSet<String> {
    let input_names = options_dataclass_type_names(api, reexported_types);
    let return_names = options_return_dataclass_names(api, dto, reexported_types);
    api.types
        .iter()
        .filter(|typ| renders_dataclass(typ, &input_names, &return_names, reexported_types))
        .filter(|typ| !delegated_methods(typ, api).is_empty())
        .map(|typ| typ.name.clone())
        .collect()
}

/// For each dataclass that forwards native methods, the names of the methods it forwards.
///
/// The Python end-to-end suite reads this list to assert that every forwarded method exists on
/// the public class at runtime, so the generated package and the generated test share one
/// answer. Both lists are sorted, so the output is stable. ~keep
pub(crate) fn forwarded_method_names(
    api: &ApiSurface,
    dto: &DtoConfig,
    reexported_types: &[String],
) -> Vec<(String, Vec<String>)> {
    let input_names = options_dataclass_type_names(api, reexported_types);
    let return_names = options_return_dataclass_names(api, dto, reexported_types);
    let mut forwarded: Vec<(String, Vec<String>)> = api
        .types
        .iter()
        .filter(|typ| renders_dataclass(typ, &input_names, &return_names, reexported_types))
        .filter_map(|typ| {
            let mut names: Vec<String> = delegated_methods(typ, api).iter().map(|m| m.name.clone()).collect();
            names.sort();
            names.dedup();
            (!names.is_empty()).then(|| (typ.name.clone(), names))
        })
        .collect();
    forwarded.sort();
    forwarded
}

/// The instance methods of `typ` that its dataclass forwards to the native class.
///
/// A method qualifies when `self` crosses the boundary by shared reference and every other
/// value is a plain scalar, string, byte string or collection of those, or (for the result) a
/// named type. Methods that take a named type, an enum value or a mutable receiver need a
/// forward conversion this generator does not have, so the dataclass leaves them out.
pub(in crate::backends::pyo3::gen_bindings) fn delegated_methods<'a>(
    typ: &'a TypeDef,
    api: &ApiSurface,
) -> Vec<&'a MethodDef> {
    if typ.is_trait || typ.fields.is_empty() {
        return Vec::new();
    }
    let simple_enums: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| !generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();
    let field_names: AHashSet<String> = binding_fields(&typ.fields)
        .map(|f| crate::core::keywords::python_ident(&f.name))
        .collect();
    typ.methods
        .iter()
        .filter(|method| method_qualifies(method, &field_names, &simple_enums))
        .collect()
}

fn method_qualifies(method: &MethodDef, field_names: &AHashSet<String>, simple_enums: &AHashSet<&str>) -> bool {
    if method.is_static
        || method.is_async
        || method.binding_excluded
        || method.sanitized
        || method.receiver != Some(ReceiverKind::Ref)
    {
        return false;
    }
    if method.name == "from_json"
        || method.name.starts_with('_')
        || crate::core::keywords::python_safe_name(&method.name).is_some()
        || field_names.contains(&method.name)
    {
        return false;
    }
    let mut seen = AHashSet::new();
    let parameters_ok = method.params.iter().all(|p| {
        crate::core::keywords::python_safe_name(&p.name).is_none()
            && p.name != "self"
            && seen.insert(p.name.as_str())
            && is_plain(&p.ty)
    });
    if !parameters_ok || !is_returnable(&method.return_type, simple_enums) {
        return false;
    }
    // A field named like a builtin that the annotations use would shadow it in the class scope.
    let mut spelled = String::new();
    for p in &method.params {
        spelled.push_str(&crate::backends::pyo3::type_map::python_type(&p.ty));
        spelled.push(' ');
    }
    spelled.push_str(&crate::backends::pyo3::type_map::python_type(&method.return_type));
    !spelled
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| field_names.contains(word))
}

/// A value that crosses the boundary unchanged: `Named` types and enums do not.
fn is_plain(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => false,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_plain(inner),
        TypeRef::Map(key, value) => is_plain(key) && is_plain(value),
        _ => true,
    }
}

/// A result the dataclass can hand back: plain, or a named type that is not a simple enum.
/// A simple enum is a native class in the native module but a `str` enum in `options.py`, so
/// the two spellings of its name would disagree.
fn is_returnable(ty: &TypeRef, simple_enums: &AHashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(name) => !simple_enums.contains(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_returnable(inner, simple_enums),
        TypeRef::Map(key, value) => is_returnable(key, simple_enums) && is_returnable(value, simple_enums),
        _ => true,
    }
}

/// The named types the forwarded methods of `typ` mention in their signatures.
pub(in crate::backends::pyo3::gen_bindings) fn delegated_signature_types(
    typ: &TypeDef,
    api: &ApiSurface,
    out: &mut AHashSet<String>,
) {
    for method in delegated_methods(typ, api) {
        collect_named_types(&method.return_type, out);
    }
}

/// The text of every forwarded method of `typ`, ready to append to the dataclass body.
pub(in crate::backends::pyo3::gen_bindings) fn render_delegated_methods(
    typ: &TypeDef,
    api: &ApiSurface,
    options_types: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::new();
    let to_native = format!("_to_rust_{}", typ.name.to_snake_case());
    for method in delegated_methods(typ, api) {
        let (required, optional): (Vec<_>, Vec<_>) = method.params.iter().partition(|p| !p.optional);
        let mut signature = vec!["self".to_string()];
        signature.extend(
            required
                .iter()
                .map(|p| format!("{}: {}", p.name, crate::backends::pyo3::type_map::python_type(&p.ty))),
        );
        signature.extend(optional.iter().map(|p| {
            let hint = crate::backends::pyo3::type_map::python_type(&p.ty);
            let hint = if hint.ends_with("| None") {
                hint
            } else {
                format!("{hint} | None")
            };
            format!("{}: {hint} = None", p.name)
        }));
        let call_args = method
            .params
            .iter()
            .map(|p| format!("{0}={0}", p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let result_expr = from_native_field_expr(&method.return_type, options_types, "result");
        let returns_unit = matches!(method.return_type, TypeRef::Unit);
        let doc = if method.doc.is_empty() {
            format!("Call `{}` on the native object.", method.name)
        } else {
            let first = sanitize_python_doc(&doc_first_paragraph_joined(&method.doc));
            let first = if first.len() > 89 {
                first.chars().take(89).collect::<String>()
            } else {
                first
            };
            if first.ends_with(['.', '?', '!']) {
                first
            } else {
                format!("{first}.")
            }
        };
        out.push('\n');
        out.push_str(&crate::backends::pyo3::template_env::render(
            "trait_bridge/options_native_method.jinja",
            crate::alef_context! {
                name => &method.name,
                signature => signature.join(", "),
                return_type => crate::backends::pyo3::type_map::python_type(&method.return_type),
                doc => doc,
                to_native => &to_native,
                call_args => call_args,
                returns_unit => returns_unit,
                converts_result => result_expr != "result",
                result_expr => result_expr,
            },
        ));
    }
    out
}
