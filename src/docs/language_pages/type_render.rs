use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};
use crate::docs::descriptions::generate_field_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_field_default};
use crate::docs::naming::{field_name, type_name};
use crate::docs::rust_types::rust_field_type;
use crate::docs::type_mapping::FFI_HANDLE_TYPE_NAME;
use crate::docs::{clean_doc, template_env};

use super::function_render::push_version_annotation;
use super::streaming::{method_visible_in_lang, render_method};

const TYPE_DOC_FIRST_HEADING_LEVEL: usize = 5;

/// ~keep True when `method` is the synthetic `Default::default()` associated function that
/// `extract_trait_impl_methods` (`src/extract/extractor/functions/impl_blocks.rs`) deliberately
/// keeps in `TypeDef::methods` instead of dropping like every other std-trait impl -- with
/// `trait_source` cleared to `None`, so nothing downstream can tell it apart from a hand-written
/// inherent method by that field alone. Go's own generator hardcodes the same shape check
/// (`if method.name == "default" { continue; }` in `gen_bindings/binding_file.rs`) to drop it
/// before emitting a single symbol; measured docs audits found zero bindings for it across Go,
/// C#, Elixir and Java. The four-part shape match (name, staticness, arity, return type) keeps
/// this from ever matching a real, differently-shaped inherent method that happens to be named
/// `default`.
fn is_phantom_default_method(ty: &TypeDef, method: &MethodDef, lang: Language) -> bool {
    // ~keep The Rust page documents the actual Rust crate, not a binding -- `Default::default`
    // really exists there, so this gate (which exists to stop *bindings* from documenting a
    // symbol they never emit) must not touch it. `method_visible_in_lang` follows the same
    // `lang == Rust` carve-out for `binding_excluded`.
    lang != Language::Rust
        && ty.has_default
        && method.name == "default"
        && method.is_static
        && method.receiver.is_none()
        && method.params.is_empty()
        && matches!(&method.return_type, TypeRef::Named(name) if name == &ty.name)
}

/// ~keep True when `lang`'s backend can bind at least some of `ty`'s methods. A non-opaque
/// `TypeDef` (plain fields, no handle) crosses the FFI boundary marshalled by value, and every
/// backend read while building this gate treats that as fields-only: pyo3 only emits a
/// `#[pymethods] impl` when `typ.is_opaque` (`gen_bindings/mod.rs`), flutter_rust_bridge mirrors
/// a non-opaque type as a plain Dart data class with no methods, and Java binds it as a record.
/// Go is the one exception actually found: `binding_file.rs` skips a type's methods with
/// `if !typ.is_opaque && !typ.has_serde { continue; }`, i.e. it still binds methods on a
/// non-opaque type that round-trips through JSON. Backends not directly inspected while tracing
/// this (Kotlin, Swift, Ruby, PHP, Elixir, Node, WASM, Zig, ...) are conservatively folded into
/// the opaque-only default rather than left rendering every non-opaque method unconditionally --
/// suppressing a member some unverified backend does bind is the safer failure than continuing
/// to fabricate one for the backends this gate did confirm never bind it.
fn methods_bound_in_lang(ty: &TypeDef, lang: Language) -> bool {
    // ~keep The Rust page documents the actual Rust crate: every inherent/trait method in
    // `ty.methods` really exists there, opaque or not, so this binding-availability gate does
    // not apply to it -- same carve-out `method_visible_in_lang` gives `binding_excluded`.
    lang == Language::Rust || ty.is_opaque || (lang == Language::Go && ty.has_serde)
}

/// ~keep The inverse of the two gates above: a member the backend *does* emit that never enters
/// `ty.methods` because it is not part of the Rust surface at all. `opaque.rs`'s
/// `render_opaque_class_body` calls `gen_opaque_handle_class` for every `typ.is_opaque` type
/// unconditionally, appending `opaque_handle_close.jinja` -- a hand-written
/// `public synchronized void close()` implementing `AutoCloseable`, injected by the Java
/// template, not derived from any Rust method. Without this, every opaque Java handle type's
/// reference page omits the one method callers must call to release the native `MemorySegment`,
/// which teaches a resource leak by omission.
fn java_opaque_close_method(ty: &TypeDef, lang: Language) -> Option<MethodDef> {
    if lang != Language::Java || !ty.is_opaque || ty.is_trait {
        return None;
    }
    Some(MethodDef {
        name: "close".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        cfg: None,
        return_type: TypeRef::Unit,
        doc: "Releases the native handle backing this instance. Implements `AutoCloseable`; \
              safe to call more than once."
            .to_string(),
        ..Default::default()
    })
}

/// ~keep Every `TypeRef::Named` crosses the C ABI as a scalar `AlefHandle` token, not a
/// pointer to a struct named after the Rust type (see type_mapping.rs's
/// `FFI_HANDLE_TYPE_NAME`). The page heading above still names the *logical* Rust type --
/// renaming every DTO's heading to the shared handle token would make every C type page
/// title collide, which is worse than the status quo. What a C reader cannot infer on
/// their own is that this heading is documentation-only: `tname` does not appear anywhere
/// in the generated header. This note says so explicitly, once, on every C type page.
fn push_ffi_handle_note(out: &mut String, tname: &str, lang: Language, ffi_prefix: &str) {
    if !matches!(lang, Language::Ffi | Language::C) {
        return;
    }
    let handle_type = type_name(FFI_HANDLE_TYPE_NAME, lang, ffi_prefix);
    out.push_str(&format!(
        "**C representation:** `{tname}` is a documentation-only name for this type. \
         The C ABI hands you a scalar `{handle_type}` handle -- the literal string \
         `{tname}` does not appear anywhere in the generated header.\n"
    ));
    out.push('\n');
}

pub(super) fn render_type(
    ty: &TypeDef,
    lang: Language,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let tname = type_name(&ty.name, lang, ffi_prefix);
    // Every documented type name must be a legal identifier in `lang` -- see
    // formatting.rs's `report_identifier_violation`. Rarely fires (PascalCase type names
    // don't usually collide with lowercase keywords), but a handful of languages have
    // capitalized reserved words too (Rust's `Self`, Swift's `Any`/`Self`). A class or
    // struct name is a declaration, not a member, so no language's member-position
    // relaxation applies to it. ~keep
    crate::docs::formatting::report_identifier_violation(
        &tname,
        lang,
        crate::docs::formatting::IdentifierPosition::Declaration,
        "a type heading",
    );

    out.push_str(&template_env::render(
        "heading.jinja",
        crate::alef_context! { marker => "####", title => tname },
    ));

    push_version_annotation(&mut out, &ty.version);
    push_ffi_handle_note(&mut out, &tname, lang, ffi_prefix);

    let doc = clean_doc(&ty.doc, lang);
    let doc = demote_headings_to_start_at(&doc, TYPE_DOC_FIRST_HEADING_LEVEL);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let fields: Vec<_> = if lang == Language::Rust {
        ty.fields.iter().collect()
    } else {
        binding_fields(&ty.fields).collect()
    };
    if !ty.is_opaque && !fields.is_empty() {
        out.push('\n');
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("|-------|------|---------|-------------|\n");
        for field in fields {
            let fname = field_name(&field.name, lang);
            let fty = if lang == Language::Rust {
                rust_field_type(field, ffi_prefix)
            } else {
                doc_type_with_optional(&field.ty, lang, field.optional, ffi_prefix)
            };
            let fdefault = format_field_default(field, lang, api, ffi_prefix);
            let fdoc = {
                let raw = clean_doc_inline(&field.doc, lang);
                if raw.is_empty() {
                    generate_field_description(&field.name, &field.ty)
                } else {
                    raw
                }
            };
            out.push_str(&template_env::render(
                "field_row.jinja",
                crate::alef_context! {
                    name => escape_table_cell(&fname),
                    ty => escape_table_cell(&fty),
                    default => escape_table_cell(&fdefault),
                    doc => escape_table_cell(&fdoc),
                },
            ));
        }
        out.push('\n');
    }

    let synthetic_close_method = java_opaque_close_method(ty, lang);
    let mut methods: Vec<&MethodDef> = if methods_bound_in_lang(ty, lang) {
        ty.methods
            .iter()
            .filter(|method| method_visible_in_lang(config, method, &ty.name, lang))
            .filter(|method| !is_phantom_default_method(ty, method, lang))
            .collect()
    } else {
        Vec::new()
    };
    if let Some(close_method) = &synthetic_close_method {
        methods.push(close_method);
    }
    if !methods.is_empty() {
        let methods_heading = if lang == Language::Elixir {
            "Functions"
        } else {
            "Methods"
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            crate::alef_context! { marker => "#####", title => methods_heading },
        ));
        for method in methods {
            out.push_str(&render_method(
                method,
                &ty.name,
                lang,
                config,
                ffi_prefix,
                &api.crate_name,
                api,
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeRef};

    #[test]
    fn type_doc_headings_stay_under_type_heading() {
        let ty = TypeDef {
            name: "ReportConfig".to_string(),
            doc: "## Default Behavior\n\nConfiguration notes.".to_string(),
            methods: vec![MethodDef {
                name: "validate".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                return_type: TypeRef::Unit,
                ..Default::default()
            }],
            ..Default::default()
        };

        let rendered = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "sample",
        );

        assert!(
            rendered.contains("#### ReportConfig"),
            "type heading should render at h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Default Behavior"),
            "type rustdoc heading should be demoted below h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Methods"),
            "methods heading should remain at h5; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n###### Default Behavior"),
            "type rustdoc H2 heading must start at h5, not skip to h6; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n### Default Behavior"),
            "type rustdoc heading must not be promoted above the type heading; got:\n{rendered}"
        );
    }

    #[test]
    fn test_c_type_page_states_handle_representation() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::C,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );

        assert!(
            rendered.contains("#### HTMChatCompletionRequest"),
            "the heading keeps the logical name -- one page per DTO would collide on a shared \
             handle token; got:\n{rendered}"
        );
        assert!(
            rendered.contains("**C representation:**") && rendered.contains("HTMAlefHandle"),
            "a C DTO page must state the actual handle type explicitly; got:\n{rendered}"
        );
    }

    #[test]
    fn test_non_c_type_page_has_no_handle_note() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::Python,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !rendered.contains("C representation"),
            "the handle note is a C-ABI-only concept; got:\n{rendered}"
        );
    }

    /// ~keep The handle token named in the page-level note and the handle token
    /// `doc_type` independently computes for any `TypeRef::Named` of this type must be
    /// the same string -- this is the type-page analogue of the earlier
    /// signature-vs-example and signature-vs-returns-prose cross-checks. If the note
    /// hardcoded a different spelling than the rest of the C pipeline actually emits,
    /// this test -- not a human re-reading the page -- catches the drift.
    #[test]
    fn test_c_type_page_handle_note_agrees_with_doc_type_handle_token() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::C,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );

        let handle_token = crate::docs::doc_type(&TypeRef::Named(ty.name.clone()), Language::C, "Htm");
        assert!(
            rendered.contains(&handle_token),
            "note must name the same handle token doc_type computes ({handle_token}); got:\n{rendered}"
        );
    }

    fn opaque_type_with_default_and_real_method(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            is_opaque: true,
            has_default: true,
            methods: vec![
                MethodDef {
                    name: "default".to_string(),
                    is_static: true,
                    receiver: None,
                    cfg: None,
                    params: vec![],
                    return_type: TypeRef::Named(name.to_string()),
                    ..Default::default()
                },
                MethodDef {
                    name: "parse".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    cfg: None,
                    return_type: TypeRef::Unit,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_phantom_default_method_is_suppressed_outside_rust() {
        let ty = opaque_type_with_default_and_real_method("LanguageRegistry");
        let rendered = render_type(
            &ty,
            Language::Java,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !rendered.contains("default()") && !rendered.contains("defaultOptions()"),
            "Default::default() never crosses into any binding -- go's binding_file.rs even \
             hardcodes `if method.name == \"default\" {{ continue; }}`, and this was measured \
             fabricating exactly `defaultOptions()` on Java; got:\n{rendered}"
        );
        assert!(
            rendered.contains("parse()"),
            "positive control: a real inherent method on the same type must still render; got:\n{rendered}"
        );
    }

    #[test]
    fn test_phantom_default_method_still_documented_on_rust_page() {
        let ty = opaque_type_with_default_and_real_method("LanguageRegistry");
        let rendered = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            rendered.contains("default()"),
            "the Rust page documents the actual crate, where Default::default() genuinely \
             exists -- the gate must not touch it; got:\n{rendered}"
        );
    }

    #[test]
    fn test_default_named_method_with_a_parameter_is_not_gated() {
        let ty = TypeDef {
            name: "Config".to_string(),
            is_opaque: true,
            has_default: true,
            methods: vec![MethodDef {
                name: "default".to_string(),
                is_static: true,
                receiver: None,
                cfg: None,
                params: vec![crate::docs::test_helpers::make_param("region", TypeRef::String, false)],
                return_type: TypeRef::Named("Config".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        // Go, not Java: naming.rs unconditionally renames every Java method literally named
        // `default` to `defaultInstance` regardless of shape, which would make this assertion
        // about the *gate* (not about naming.rs's rename table) give a false failure.
        let rendered = render_type(
            &ty,
            Language::Go,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            rendered.contains("Default("),
            "a `default`-named method that does not match the exact zero-arg, static, \
             returns-Self shape of `Default::default()` is a different, real method and must \
             not be swept up by the gate; got:\n{rendered}"
        );
    }

    fn non_opaque_type_with_builder_method(name: &str, has_serde: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            is_opaque: false,
            has_serde,
            methods: vec![MethodDef {
                name: "with_chunking".to_string(),
                receiver: Some(ReceiverKind::Owned),
                cfg: None,
                return_type: TypeRef::Named(name.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_non_opaque_type_methods_are_suppressed_outside_rust() {
        let ty = non_opaque_type_with_builder_method("ProcessConfig", false);
        let rendered = render_type(
            &ty,
            Language::Java,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !rendered.contains("Methods"),
            "Java binds a non-opaque type as a record with no methods -- a builder method \
             extracted from the Rust `impl` block never crosses into the binding; got:\n{rendered}"
        );
    }

    #[test]
    fn test_non_opaque_type_methods_still_documented_on_rust_page() {
        let ty = non_opaque_type_with_builder_method("ProcessConfig", false);
        let rendered = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            rendered.contains("with_chunking()"),
            "positive control: the Rust page documents the real Rust builder method; got:\n{rendered}"
        );
    }

    #[test]
    fn test_go_serde_non_opaque_type_still_binds_methods() {
        let ty = non_opaque_type_with_builder_method("ProcessConfig", true);
        let go = render_type(
            &ty,
            Language::Go,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            go.contains("Methods"),
            "Go's binding_file.rs binds methods on a non-opaque type when it round-trips \
             through JSON (`has_serde`); got:\n{go}"
        );

        let java = render_type(
            &ty,
            Language::Java,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !java.contains("Methods"),
            "the has_serde carve-out is Go-specific, confirmed only against binding_file.rs -- \
             it must not also open the gate for Java; got:\n{java}"
        );
    }

    #[test]
    fn test_java_opaque_type_gets_synthetic_close_method() {
        let ty = TypeDef {
            name: "DefaultClient".to_string(),
            is_opaque: true,
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::Java,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            rendered.contains("close()"),
            "opaque.rs's `render_opaque_class_body` appends `opaque_handle_close.jinja`'s \
             `close()` to every opaque Java handle type unconditionally; got:\n{rendered}"
        );
    }

    #[test]
    fn test_close_method_is_not_synthesized_outside_java() {
        let ty = TypeDef {
            name: "DefaultClient".to_string(),
            is_opaque: true,
            ..Default::default()
        };
        let python = render_type(
            &ty,
            Language::Python,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !python.contains("Methods"),
            "close() is a Java `AutoCloseable` template artifact, not part of the Rust surface \
             -- it must not leak into other languages' pages; got:\n{python}"
        );
        let rust = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !rust.contains("Methods"),
            "the Rust struct itself has no close() method; got:\n{rust}"
        );
    }
}
