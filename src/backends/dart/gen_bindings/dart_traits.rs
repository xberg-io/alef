use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use super::render_type::render_type;
use crate::backends::dart::template_env;

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

    // Emit trait bridge stub types required by e2e test fixtures.
    // These types are internal to the Rust core but must be exposed to Dart tests
    // so test fixtures can construct mock implementations.
    emit_trait_stub_types(&mut body);

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
    let own_methods: Vec<&MethodDef> = trait_def.methods.iter().filter(|m| m.trait_source.is_none()).collect();

    // Doc comment: registration pattern.
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
        let method_camel = method.name.to_lower_camel_case();
        out.push_str("///   @override\n");
        out.push_str(&template_env::render(
            "abstract_class_method_doc_line.jinja",
            minijinja::context! {
                return_type => dart_return_type_str(&method.return_type, imports),
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
        let method_camel = method.name.to_lower_camel_case();
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
        emit_abstract_method(method, out, imports);
    }

    out.push_str("}\n");
}

/// Emit one abstract method declaration inside an abstract class.
fn emit_abstract_method(method: &MethodDef, out: &mut String, imports: &mut BTreeSet<String>) {
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

    let method_camel = method.name.to_lower_camel_case();
    let inner_ret = substitute_internal_document(&dart_return_type_str(&method.return_type, imports));

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
            let rendered = render_type(&p.ty, imports);
            let mapped = substitute_internal_document(&rendered);
            let ty = if p.optional { format!("{mapped}?") } else { mapped };
            format!("{ty} {}", p.name.to_lower_camel_case())
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

/// Substitute the internal Rust type `InternalDocument` with the binding-facing
/// type `ExtractionResult`. The Rust trait signatures (e.g.
/// `DocumentExtractor::extract_bytes -> Result<InternalDocument>`,
/// `Renderer::render(&InternalDocument)`) reference an internal type, but the
/// public Dart binding surfaces this as `ExtractionResult` — so the trait
/// declarations and the test-stub overrides must agree on the public name.
/// Other backends (gleam, go, zig) handle this via explicit excluded-types
/// substitution; dart applies it directly to the rendered type string.
fn substitute_internal_document(rendered: &str) -> String {
    rendered.replace("InternalDocument", "ExtractionResult")
}

/// Emit trait bridge stub types required by e2e test fixtures.
///
/// These types (OcrBackendType, ProcessingStage, SyncExtractor) are used by
/// test stub implementations to satisfy trait method signatures. `InternalDocument`
/// is intentionally not emitted as a placeholder — the trait emitter substitutes
/// it with `ExtractionResult` so the public binding surface is consistent.
fn emit_trait_stub_types(out: &mut String) {
    out.push('\n');
    out.push_str("/// OCR backend type identifier — used by e2e test plugin_api stubs.\n");
    out.push_str("enum OcrBackendType { tesseract, easyocr, paddleocr, rapidocr }\n");
    out.push('\n');
    out.push_str("/// Processing stage for post-processors — used by e2e test plugin_api stubs.\n");
    out.push_str("enum ProcessingStage { preProcessing, processing, postProcessing }\n");
    out.push('\n');
    out.push_str("/// Synchronous extractor trait stub — used by e2e test plugin_api stubs.\n");
    out.push_str("abstract class SyncExtractor {}\n");
}
