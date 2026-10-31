use crate::codegen::error_gen::{go_error_sentinel_name, python_exception_name};
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::Language;
use crate::core::ir::ErrorDef;
use crate::docs::descriptions::generate_error_variant_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use crate::docs::formatting::escape_table_cell;
use crate::docs::naming::{enum_variant_name, type_name};
use crate::docs::{clean_doc, template_env};

/// ~keep The identifier a target language's own error-binding codegen actually produces for a
/// Rust error variant, when that identifier differs from `naming::enum_variant_name`'s generic
/// per-language case transform (which is correct for real enums but was being reused here for
/// errors too). Each arm below calls the exact function or reproduces the exact `format!` the
/// real backend generator uses -- see `src/codegen/error_gen/host_langs.rs` for C#/Java/Go and
/// `src/backends/swift/gen_bindings/errors.rs` for Swift -- so the doc page cannot drift from
/// what the compiled binding exports. Every other language still falls back to
/// `enum_variant_name`, which was verified against its own backend (Kotlin: `data class
/// {variant.name}` in `object_wrapper/errors.rs`; Zig: `zig_error_variant_component` delegates
/// to the same `public_host_identifier` naming.rs already mirrors).
fn error_variant_identifier(
    variant_name: &str,
    error_name: &str,
    all_errors: &[ErrorDef],
    lang: Language,
    ffi_prefix: &str,
) -> String {
    match lang {
        Language::Csharp | Language::Java => format!("{variant_name}Exception"),
        Language::Go => go_error_sentinel_name(all_errors, error_name, variant_name),
        Language::Swift => public_host_identifier(lang, PublicIdentifierKind::EnumVariant, variant_name),
        _ => enum_variant_name(variant_name, lang, ffi_prefix),
    }
}

/// How to catch every error of the `family` family where a variant class it shares with another
/// family does not extend it: the marker interface every class of the family implements (PHP, C#,
/// Java), or the `Family` module every class of the family includes (Ruby).
fn family_interface_note(family: &str, lang: Language) -> Option<String> {
    let interface = crate::codegen::error_gen::family_interface_type_name(family);
    let (member, catch) = match lang {
        Language::Php => (
            format!("implements `{family}Interface`"),
            format!("`catch ({family}Interface $e)`"),
        ),
        Language::Csharp => (
            format!("implements `{interface}`"),
            format!("`catch (Exception e) when (e is {interface})`"),
        ),
        Language::Java => (
            format!("implements `{interface}`"),
            format!("`e instanceof {interface}`"),
        ),
        Language::Ruby => (
            format!("is a `{family}::Family`"),
            format!("`rescue {family}` or `error.is_a?({family}::Family)`"),
        ),
        _ => return None,
    };
    let missing = match lang {
        Language::Php => {
            " An introspection value the raising family does not report reads as the type's default \
             (0, `''` or `false`)."
        }
        Language::Java => {
            " On a shared class, an introspection value the raising family does not report is `null` when it \
             adds the value, or the type's default when its base family declares it."
        }
        Language::Csharp => {
            " On a shared class, an introspection value the raising family does not report is the type's \
             default when its base family declares it; one the class adds is `null`, or the native default \
             when the raising variant has no stable error code."
        }
        _ => "",
    };
    Some(format!(
        "Every exception of this family {member}, including a variant class it shares with another family; \
         {catch} covers all of them.{missing}"
    ))
}

pub(super) fn render_error(err: &ErrorDef, all_errors: &[ErrorDef], lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&err.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        crate::alef_context! { marker => "####", title => &ename },
    ));

    let doc = clean_doc(&err.doc, lang);
    // Nest under the `####` heading emitted just above, rather than shifting by a fixed
    // number of levels. A fixed `+2` assumes the doc comment starts at `#`, and a section that
    // starts anywhere else lands ABOVE its own parent: a rustdoc `# Observability` surfaced as
    // `###` under a `####` item, so it read as a sibling of the page's `### Functions` section and
    // took a bogus entry in the table of contents with it. ~keep
    let doc = demote_headings_to_start_at(&doc, 5);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    if matches!(lang, Language::Node | Language::Wasm) {
        out.push_str("Errors are thrown as plain `Error` objects with descriptive messages.\n\n");
    }

    if let Some(note) = family_interface_note(&err.name, lang) {
        out.push_str(&note);
        out.push_str("\n\n");
    }

    if lang == Language::Python {
        out.push_str(&template_env::render(
            "base_class.jinja",
            crate::alef_context! { name => &ename },
        ));
        out.push('\n');
        out.push_str("| Exception | Description |\n");
        out.push_str("|-----------|-------------|\n");
        for variant in &err.variants {
            let vname = python_exception_name(&variant.name, &err.name);
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            out.push_str(&template_env::render(
                "exception_row.jinja",
                crate::alef_context! {
                    variant => escape_table_cell(&vname),
                    error => escape_table_cell(&ename),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        }
    } else {
        out.push('\n');
        out.push_str("| Variant | Description |\n");
        out.push_str("|---------|-------------|\n");
        for variant in &err.variants {
            let vname = error_variant_identifier(&variant.name, &err.name, all_errors, lang, ffi_prefix);
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            out.push_str(&template_env::render(
                "variant_row.jinja",
                crate::alef_context! { name => escape_table_cell(&vname), doc => escape_table_cell(&vdoc) },
            ));
        }
    }
    out.push('\n');

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ErrorVariant;
    use crate::docs::test_helpers::TEST_PREFIX;

    fn make_error(name: &str, variant_names: &[&str]) -> ErrorDef {
        ErrorDef {
            exception_base: None,
            name: name.to_string(),
            rust_path: format!("mylib::{name}"),
            original_rust_path: String::new(),
            variants: variant_names
                .iter()
                .map(|variant_name| ErrorVariant {
                    name: variant_name.to_string(),
                    ..Default::default()
                })
                .collect(),
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn test_csharp_error_variant_gets_real_exception_class_name() {
        let err = make_error("MyError", &["LanguageNotFound"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Csharp, TEST_PREFIX);
        assert!(
            rendered.contains("LanguageNotFoundException"),
            "C# binds one exception subclass per variant, named `{{Variant}}Exception` \
             (host_langs.rs's `gen_csharp_error_types`); got:\n{rendered}"
        );
    }

    #[test]
    fn test_java_error_variant_gets_real_exception_class_name() {
        let err = make_error("MyError", &["DynamicLoad"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Java, TEST_PREFIX);
        assert!(
            rendered.contains("DynamicLoadException"),
            "Java follows the same `{{Variant}}Exception` convention as C# \
             (host_langs.rs's `gen_java_error_types`); got:\n{rendered}"
        );
    }

    #[test]
    fn test_go_error_variant_gets_real_sentinel_name() {
        let err = make_error("MyError", &["LanguageNotFound"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Go, TEST_PREFIX);
        assert!(
            rendered.contains("ErrLanguageNotFound"),
            "Go binds a package-level `Err{{Variant}}` sentinel, not the bare variant name \
             (host_langs.rs's `go_error_sentinel_name`); got:\n{rendered}"
        );
        assert!(
            !rendered.contains("`LanguageNotFound`"),
            "the bare, unprefixed variant name must not appear as its own table cell; got:\n{rendered}"
        );
    }

    #[test]
    fn test_swift_error_variant_uses_lower_camel_case_not_pascal_case() {
        let err = make_error("MyError", &["LanguageNotFound"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Swift, TEST_PREFIX);
        assert!(
            rendered.contains("languageNotFound"),
            "Swift emits a lowerCamelCase `case`, not a PascalCase one \
             (gen_bindings/errors.rs's central enum-variant naming helper); \
             got:\n{rendered}"
        );
        assert!(
            !rendered.contains("LanguageNotFound"),
            "the PascalCase spelling must not appear -- it is not what the Swift enum declares; \
             got:\n{rendered}"
        );
    }

    #[test]
    fn test_python_error_variant_without_error_suffix_gets_one() {
        let err = make_error("MyError", &["InvalidInput"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Python, TEST_PREFIX);
        assert!(
            rendered.contains("InvalidInputError"),
            "pyo3's `create_exception!` macro appends `Error` when the variant name lacks it \
             (N818 compliance); got:\n{rendered}"
        );
    }

    #[test]
    fn test_python_error_variant_already_ending_in_error_is_unchanged() {
        let err = make_error("MyError", &["IoError"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Python, TEST_PREFIX);
        assert!(rendered.contains("IoError"), "got:\n{rendered}");
        assert!(
            !rendered.contains("IoErrorError"),
            "a variant already ending in `Error` must not be double-suffixed; got:\n{rendered}"
        );
    }

    /// ~keep Positive control: Kotlin was independently confirmed (`object_wrapper/errors.rs`'s
    /// `format!("data class {}", variant.name)`) to bind the bare, unmodified variant name --
    /// the language-specific overrides above must not sweep it in too.
    #[test]
    fn test_kotlin_error_variant_keeps_bare_pascal_case() {
        let err = make_error("MyError", &["LanguageNotFound"]);
        let rendered = render_error(&err, std::slice::from_ref(&err), Language::Kotlin, TEST_PREFIX);
        assert!(
            rendered.contains("`LanguageNotFound`"),
            "Kotlin's real `data class` name is the bare variant name, unlike C#/Java/Go/Swift; \
             got:\n{rendered}"
        );
    }

    #[test]
    fn family_pages_name_the_type_that_covers_every_class_of_the_family() {
        let error = make_error("SchemaError", &["Invalid"]);
        let errors = std::slice::from_ref(&error);
        for (lang, expected) in [
            (
                Language::Ruby,
                "Every exception of this family is a `SchemaError::Family`, including a variant class it shares \
                 with another family; `rescue SchemaError` or `error.is_a?(SchemaError::Family)` covers all of them.",
            ),
            (Language::Php, "`catch (SchemaErrorInterface $e)`"),
            (
                Language::Php,
                "An introspection value the raising family does not report reads as the type's default \
                 (0, `''` or `false`).",
            ),
            (Language::Csharp, "`catch (Exception e) when (e is ISchemaError)`"),
            (
                Language::Csharp,
                "one the class adds is `null`, or the native default when the raising variant has no stable error \
                 code.",
            ),
            (Language::Java, "`e instanceof ISchemaError`"),
            (
                Language::Java,
                "is `null` when it adds the value, or the type's default when its base family declares it.",
            ),
        ] {
            let rendered = render_error(&error, errors, lang, TEST_PREFIX);
            assert!(rendered.contains(expected), "{lang:?}:\n{rendered}");
        }
        let python = render_error(&error, errors, Language::Python, TEST_PREFIX);
        assert!(!python.contains("Family") && !python.contains("Interface"), "{python}");
    }
}
