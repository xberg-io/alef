use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use super::render_type::render_type;

/// Emit the content of `packages/dart/lib/src/traits.dart` — one `abstract class`
/// per configured trait bridge name found in the API surface.
///
/// Returns the body text and any imports that should be prepended.
pub(super) fn emit_dart_traits(api: &ApiSurface, trait_names: &[&str]) -> (String, BTreeSet<String>) {
    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    for &trait_name in trait_names {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == trait_name && t.is_trait) {
            emit_trait_abstract_class(trait_def, &mut body, &mut imports);
            body.push('\n');
        }
    }

    (body, imports)
}

/// Emit a single `abstract class {TraitName}` for `trait_def`.
///
/// The class contains one abstract `Future<{Ret}> {method}(...)` per own method
/// (methods without a `trait_source`). A doc comment shows the registration
/// pattern using `create_{snake}_dart_impl(...)`.
fn emit_trait_abstract_class(trait_def: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    let trait_name = &trait_def.name;

    // Filter to own methods only (no inherited super-trait methods).
    let own_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none())
        .collect();

    // Doc comment: registration pattern.
    out.push_str(&format!("/// Abstract class for the `{trait_name}` Rust trait.\n"));
    out.push_str("///\n");
    out.push_str("/// Implement this class and register your implementation via:\n");
    out.push_str("/// ```dart\n");
    out.push_str(&format!("/// class My{trait_name} implements {trait_name} {{\n"));
    for method in &own_methods {
        let method_camel = method.name.to_lower_camel_case();
        out.push_str("///   @override\n");
        out.push_str(&format!(
            "///   Future<{}> {}(...) async {{ ... }}\n",
            dart_return_type_str(&method.return_type, imports),
            method_camel,
        ));
    }
    out.push_str("/// }\n");
    out.push_str("///\n");
    out.push_str(&format!("/// final impl = create{trait_name}DartImpl(\n"));
    for method in &own_methods {
        let method_camel = method.name.to_lower_camel_case();
        out.push_str(&format!(
            "///   {method_camel}: (...) => myInstance.{method_camel}(...),\n"
        ));
    }
    out.push_str("/// );\n");
    out.push_str("/// ```\n");

    out.push_str(&format!("abstract class {trait_name} {{\n"));

    for method in &own_methods {
        emit_abstract_method(method, out, imports);
    }

    out.push_str("}\n");
}

/// Emit one abstract method declaration inside an abstract class.
fn emit_abstract_method(method: &MethodDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !method.doc.is_empty() {
        for line in method.doc.lines() {
            out.push_str("  /// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(ref error_ty) = method.error_type {
        out.push_str(&format!("  /// throws {error_ty} on failure\n"));
    }

    let method_camel = method.name.to_lower_camel_case();
    let inner_ret = dart_return_type_str(&method.return_type, imports);

    // All trait methods are bridged as async from the Dart side — they always
    // use DartFnFuture on the Rust side, so we always emit `Future<T>`.
    let return_ty = if matches!(method.return_type, TypeRef::Unit) {
        "Future<void>".to_string()
    } else {
        format!("Future<{inner_ret}>")
    };

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = if p.optional {
                format!("{}?", render_type(&p.ty, imports))
            } else {
                render_type(&p.ty, imports)
            };
            format!("{ty} {}", p.name.to_lower_camel_case())
        })
        .collect();

    out.push_str(&format!("  {return_ty} {}({});\n", method_camel, params.join(", ")));
}

/// Render the inner Dart type for a return type (the `T` in `Future<T>`).
///
/// Returns `"void"` for `TypeRef::Unit`.
fn dart_return_type_str(ty: &TypeRef, imports: &mut BTreeSet<String>) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        _ => render_type(ty, imports),
    }
}
