use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use super::render_type::render_type;
use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;

/// Dart name for a trait method or one of its parameters, renamed when the camel-cased Rust
/// name collides with the language.
///
/// Dart has no identifier-escape syntax — no `r#`, no backticks — so a collision can only be
/// resolved by renaming, which is why [`dart_safe_ident`] appends `_`. Two distinct Dart
/// categories reach this emitter, and both are hard parse errors in member-declaration
/// position:
///
/// - `new` is a **reserved word**, illegal in every identifier position. `fn new()` is the
///   canonical Rust constructor, so it arrives here routinely.
/// - `get` and `set` are **built-in identifiers** — legal as parameters and locals, but not as
///   member names: the parser reads `Future<T> get(..)` as a getter header and fails at the
///   `(`. `fn get(&self, ..)` is an equally routine Rust trait method.
///
/// Every other emitted Dart identifier (fields, enum variants, wrapper functions, wrapper
/// parameters) already routes through [`dart_safe_ident`]; trait methods and their parameters
/// were the only positions that did not. ~keep
fn dart_trait_ident(rust_name: &str) -> String {
    dart_safe_ident(&rust_name.to_lower_camel_case())
}

/// Emit the content of `packages/dart/lib/src/traits.dart` — one `abstract class`
/// per configured trait bridge name found in the API surface.
///
/// Returns the body text and any imports that should be prepended.
pub(super) fn emit_dart_traits(api: &ApiSurface, trait_names: &[&str]) -> (String, BTreeSet<String>) {
    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    for &trait_name in trait_names {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == trait_name && t.is_trait) {
            emit_trait_abstract_class(trait_def, &api.excluded_type_paths, &mut body, &mut imports);
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
fn emit_trait_abstract_class(
    trait_def: &TypeDef,
    excluded_type_paths: &std::collections::BTreeMap<String, String>,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    let trait_name = &trait_def.name;

    let own_methods: Vec<&MethodDef> = trait_def.methods.iter().filter(|m| m.trait_source.is_none()).collect();

    out.push_str(&template_env::render(
        "abstract_class_doc_comment.jinja",
        minijinja::context! {
            trait_name => trait_name.as_str(),
        },
    ));
    out.push_str(&template_env::render(
        "abstract_class_doc_code_start.jinja",
        minijinja::context! {},
    ));
    out.push_str(&template_env::render(
        "abstract_class_doc_code_impl.jinja",
        minijinja::context! {
            trait_name => trait_name.as_str(),
        },
    ));
    for method in &own_methods {
        let method_camel = dart_trait_ident(&method.name);
        out.push_str("///   @override\n");
        out.push_str(&template_env::render(
            "abstract_class_method_doc_line.jinja",
            minijinja::context! {
                return_type => substitute_excluded_named_types(
                    &dart_return_type_str(&method.return_type, imports),
                    excluded_type_paths,
                ),
                method_camel => method_camel.as_str(),
            },
        ));
    }
    out.push_str("/// }\n");
    out.push_str("///\n");
    out.push_str(&template_env::render(
        "abstract_class_doc_code_create.jinja",
        minijinja::context! {
            trait_name => trait_name.as_str(),
        },
    ));
    for method in &own_methods {
        let method_camel = dart_trait_ident(&method.name);
        out.push_str(&template_env::render(
            "trait_method_doc_field.jinja",
            minijinja::context! {
                method_camel => method_camel.as_str(),
            },
        ));
    }
    out.push_str(&template_env::render(
        "abstract_class_doc_code_end.jinja",
        minijinja::context! {},
    ));

    out.push_str(&template_env::render(
        "abstract_class_header.jinja",
        minijinja::context! {
            trait_name => trait_name.as_str(),
        },
    ));

    for method in &own_methods {
        emit_abstract_method(method, excluded_type_paths, out, imports);
    }

    out.push_str("}\n");
}

/// Emit one abstract method declaration inside an abstract class.
fn emit_abstract_method(
    method: &MethodDef,
    excluded_type_paths: &std::collections::BTreeMap<String, String>,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    if !method.doc.is_empty() {
        let doc_lines: Vec<String> = method.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "  ",
                lines => doc_lines,
            },
        ));
    }
    if let Some(ref error_ty) = method.error_type {
        out.push_str(&template_env::render(
            "function_throws_annotation.jinja",
            minijinja::context! {
                error_ty => error_ty.as_str(),
            },
        ));
    }

    let method_camel = dart_trait_ident(&method.name);
    let inner_ret =
        substitute_excluded_named_types(&dart_return_type_str(&method.return_type, imports), excluded_type_paths);

    let return_ty = if matches!(method.return_type, TypeRef::Unit) {
        "Future<void>".to_string()
    } else {
        format!("Future<{inner_ret}>")
    };

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let rendered = render_type(&p.ty, imports);
            let mapped = substitute_excluded_named_types(&rendered, excluded_type_paths);
            let ty = if p.optional { format!("{mapped}?") } else { mapped };
            format!("{ty} {}", dart_trait_ident(&p.name))
        })
        .collect();

    out.push_str(&template_env::render(
        "abstract_method_declaration.jinja",
        minijinja::context! {
            return_ty => return_ty,
            method_camel => method_camel.as_str(),
            params => params.join(", "),
        },
    ));
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

/// Substitute excluded named Rust types with explicit JSON-backed bridge types.
/// The generated bridge serializes/deserializes these opaque carriers at the Rust edge.
fn substitute_excluded_named_types(
    rendered: &str,
    excluded_type_paths: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut mapped = rendered.to_string();
    for name in excluded_type_paths.keys() {
        mapped = replace_token(&mapped, name, &format!("{name}Bridge"));
    }
    mapped
}

fn replace_token(input: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(index) = rest.find(needle) {
        let (before, after_start) = rest.split_at(index);
        out.push_str(before);

        let after = &after_start[needle.len()..];
        let before_ok = out.chars().last().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if before_ok && after_ok {
            out.push_str(replacement);
        } else {
            out.push_str(needle);
        }
        rest = after;
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, PrimitiveType, ReceiverKind};
    use std::collections::BTreeMap;

    fn trait_with_method(method: MethodDef) -> TypeDef {
        TypeDef {
            name: "SampleBackend".to_string(),
            is_trait: true,
            methods: vec![method],
            ..Default::default()
        }
    }

    fn method(name: &str, params: Vec<ParamDef>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: true,
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }
    }

    fn param(name: &str) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }
    }

    fn emit(trait_def: &TypeDef) -> String {
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_trait_abstract_class(trait_def, &BTreeMap::new(), &mut out, &mut imports);
        out
    }

    /// `new` is a Dart *reserved word*: illegal in every identifier position. `fn new()` is
    /// the canonical Rust constructor, so it reaches the trait emitter routinely.
    #[test]
    fn trait_method_named_new_is_renamed_in_the_declaration() {
        let body = emit(&trait_with_method(method("new", vec![])));

        assert!(
            body.contains("Future<bool> new_();"),
            "reserved word `new` must be renamed in the abstract method declaration:\n{body}"
        );
        assert!(
            !body.contains("Future<bool> new("),
            "`new` must never be emitted as a bare Dart identifier:\n{body}"
        );
    }

    /// `get` is a Dart *built-in identifier*, not a reserved word — legal as a parameter or
    /// local, but not as a member name: `Future<bool> get(..)` parses as a getter header and
    /// fails at the `(`.
    #[test]
    fn trait_method_named_get_is_renamed_in_the_declaration() {
        let body = emit(&trait_with_method(method("get", vec![param("key")])));

        assert!(
            body.contains("Future<bool> get_(String key);"),
            "built-in identifier `get` must be renamed in member position:\n{body}"
        );
        assert!(
            !body.contains("Future<bool> get("),
            "`get` must never open a Dart method declaration:\n{body}"
        );
    }

    /// The same rename must reach `set`, the setter-syntax twin of `get`.
    #[test]
    fn trait_method_named_set_is_renamed_in_the_declaration() {
        let body = emit(&trait_with_method(method("set", vec![param("key")])));

        assert!(
            body.contains("Future<bool> set_(String key);"),
            "built-in identifier `set` must be renamed in member position:\n{body}"
        );
    }

    /// A reserved word in *parameter* position is equally illegal, and `new` is a legal Rust
    /// parameter name.
    #[test]
    fn trait_method_parameter_named_new_is_renamed() {
        let body = emit(&trait_with_method(method("apply", vec![param("new")])));

        assert!(
            body.contains("Future<bool> apply(String new_);"),
            "reserved word `new` must be renamed in parameter position:\n{body}"
        );
    }

    /// The `///` usage block above the class is what a reader copies into their `@override`
    /// implementation and into the `create<Trait>DartImpl(..)` call, so it must name the
    /// method exactly as the declaration does — a rename applied to only one of the three
    /// emission sites would hand the reader a name that does not exist.
    #[test]
    fn renamed_method_name_is_consistent_across_doc_block_and_declaration() {
        let body = emit(&trait_with_method(method("get", vec![])));

        assert!(
            body.contains("///   Future<bool> get_(...) async { ... }"),
            "@override doc line must show the renamed method:\n{body}"
        );
        assert!(
            body.contains("///   get_: (...) => myInstance.get_(...),"),
            "create<Trait>DartImpl doc line must show the renamed method:\n{body}"
        );
        assert!(
            body.contains("Future<bool> get_();"),
            "declaration must show the renamed method:\n{body}"
        );
    }

    #[test]
    fn ordinary_method_and_parameter_names_are_left_alone() {
        let body = emit(&trait_with_method(method("process_image", vec![param("mime_type")])));

        assert!(
            body.contains("Future<bool> processImage(String mimeType);"),
            "non-colliding names must pass through unchanged:\n{body}"
        );
    }
}
