mod table_cells;
pub(crate) use table_cells::escape_table_cell;
mod defaults;
mod identifiers;
mod keyword_tables;
mod type_shapes;

pub(crate) use defaults::format_field_default;
#[cfg(test)]
pub(crate) use defaults::format_typed_default;
pub(crate) use identifiers::{
    IdentifierPosition, IdentifierViolation, identifier_violation, report_identifier_violation,
};
pub(crate) use type_shapes::{doc_type_with_optional, format_error_phrase};

pub(crate) fn check_identifier(
    name: &str,
    lang: crate::core::config::Language,
    position: IdentifierPosition,
    context: &str,
) -> Result<(), IdentifierViolation> {
    identifiers::check_identifier(name, lang, position, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::core::ir::{DefaultValue, PrimitiveType, TypeRef};
    use crate::docs::test_helpers::{TEST_CRATE_NAME, TEST_PREFIX, empty_api, make_field};
    use crate::docs::type_mapping::doc_type;

    // ~keep The identifier gate: rejects a reserved-word collision (Java/Dart's `new`,
    // etc.) that a curated or templated name might introduce. These are the per-language
    // ground truths the tslp `DownloadManager` audit measured against a real compiler
    // (`javac`) and real language specs, not inference -- see reserved_words' doc comment.
    #[test]
    fn test_identifier_violation_rejects_new_in_declaration_position_wherever_it_is_reserved() {
        for lang in [
            Language::Java,
            Language::Dart,
            Language::Php,
            Language::Node,
            Language::Csharp,
        ] {
            assert_eq!(
                identifier_violation("new", lang, IdentifierPosition::Declaration),
                Some("reserved word"),
                "`new` must be rejected in declaration position for {lang:?}"
            );
        }
    }

    /// The tslp crash. `new` in *member* position is legal TypeScript and legal PHP 7+, and
    /// both backends emit it verbatim -- napi's `index.d.ts` really carries
    /// `static new(version: string): DownloadManager`. A position-blind gate called that a
    /// reserved word and panicked the docs run over correct output.
    #[test]
    fn test_identifier_violation_accepts_new_in_member_position_where_the_grammar_allows_it() {
        for lang in [Language::Node, Language::Wasm, Language::Php] {
            assert_eq!(
                identifier_violation("new", lang, IdentifierPosition::Member),
                None,
                "`new` is a legal member name in {lang:?}"
            );
        }
    }

    /// Position-awareness is not a blanket amnesty. Java, C# and Dart all take an
    /// `identifier` in member position, which excludes their reserved words, so the gate
    /// must keep firing there -- these are the positive controls that separate the fix from
    /// "make the gate always pass".
    #[test]
    fn test_identifier_violation_still_rejects_new_in_member_position_where_the_grammar_forbids_it() {
        for lang in [Language::Java, Language::Csharp, Language::Dart] {
            assert_eq!(
                identifier_violation("new", lang, IdentifierPosition::Member),
                Some("reserved word"),
                "`new` is not a legal member name in {lang:?}"
            );
        }
    }

    /// A language whose member slot *is* relaxed still cannot take a keyword as the binding
    /// identifier of a top-level `function` declaration -- which is exactly what
    /// `render_typescript_fn_sig` emits for a free function.
    #[test]
    fn test_identifier_violation_node_relaxation_does_not_reach_declaration_position() {
        assert_eq!(
            identifier_violation("class", Language::Node, IdentifierPosition::Member),
            None
        );
        assert_eq!(
            identifier_violation("class", Language::Node, IdentifierPosition::Declaration),
            Some("reserved word")
        );
    }

    /// Syntactic well-formedness is position-independent: no member slot here accepts an
    /// unquoted name starting with a digit, so relaxing the reserved-word rule must not
    /// relax this one with it.
    #[test]
    fn test_identifier_violation_rejects_malformed_names_in_member_position_too() {
        for lang in [Language::Node, Language::Wasm, Language::Php] {
            assert_eq!(
                identifier_violation("1invalid", lang, IdentifierPosition::Member),
                Some("not a syntactically valid identifier"),
                "{lang:?}"
            );
            assert_eq!(
                identifier_violation("", lang, IdentifierPosition::Member),
                Some("empty identifier"),
                "{lang:?}"
            );
        }
    }

    #[test]
    fn test_identifier_violation_accepts_new_in_languages_where_it_is_not_reserved() {
        // Rust, Go, Kotlin, Zig, and Elixir do not reserve `new` -- this is exactly why
        // `Type::new()`/`func_name(...)`-style constructors are idiomatic there. Elixir's
        // real `DownloadManager.new/1` (the tslp audit's one correct case) depends on this.
        for lang in [
            Language::Rust,
            Language::Go,
            Language::Kotlin,
            Language::Zig,
            Language::Elixir,
        ] {
            for position in [IdentifierPosition::Declaration, IdentifierPosition::Member] {
                assert_eq!(
                    identifier_violation("new", lang, position),
                    None,
                    "`new` must be accepted for {lang:?} in {position:?} position"
                );
            }
        }
    }

    #[test]
    fn test_identifier_violation_csharp_new_only_collides_lowercase() {
        // C#'s real defect (a fabricated static factory instead of a real constructor) is
        // a name-*selection* problem, not a reserved-word one: `func_name` PascalCases to
        // `New`, which is not the reserved `new` and so the gate correctly does not fire on
        // it -- catching this class of bug needs item 1 (modeling the curated
        // constructor), not the gate.
        assert_eq!(
            identifier_violation("new", Language::Csharp, IdentifierPosition::Declaration),
            Some("reserved word")
        );
        assert_eq!(
            identifier_violation("New", Language::Csharp, IdentifierPosition::Declaration),
            None
        );
    }

    #[test]
    fn test_identifier_violation_rejects_python_default_arg_style_keywords() {
        assert_eq!(
            identifier_violation("class", Language::Python, IdentifierPosition::Declaration),
            Some("reserved word")
        );
        assert_eq!(
            identifier_violation("import", Language::Python, IdentifierPosition::Declaration),
            Some("reserved word")
        );
    }

    #[test]
    fn test_identifier_violation_kotlin_default_is_not_reserved() {
        // Regression guard for the existing `default` -> `@JvmStatic fun default()` test
        // fixture: Kotlin does not reserve `default` as a hard keyword.
        assert_eq!(
            identifier_violation("default", Language::Kotlin, IdentifierPosition::Declaration),
            None
        );
    }

    #[test]
    fn test_identifier_violation_rejects_empty_identifier() {
        assert_eq!(
            identifier_violation("", Language::Python, IdentifierPosition::Declaration),
            Some("empty identifier")
        );
    }

    #[test]
    fn test_identifier_violation_rejects_identifier_starting_with_digit() {
        assert_eq!(
            identifier_violation("1invalid", Language::Python, IdentifierPosition::Declaration),
            Some("not a syntactically valid identifier")
        );
    }

    #[test]
    fn test_identifier_violation_accepts_ordinary_identifier_in_every_language() {
        for lang in [
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::Elixir,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Rust,
            Language::Kotlin,
            Language::KotlinAndroid,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
            Language::R,
            Language::Ffi,
            Language::C,
            Language::Jni,
        ] {
            assert_eq!(
                identifier_violation("classifyLink", lang, IdentifierPosition::Declaration),
                None,
                "an ordinary camelCase identifier must be accepted for {lang:?}"
            );
        }
    }

    /// The gate no longer aborts the process; a rejected name comes back as an inspectable
    /// error whose text carries every field the old panic message did, plus the position
    /// that decided the verdict.
    #[test]
    fn test_check_identifier_returns_a_structured_error_instead_of_panicking() {
        let violation = check_identifier("new", Language::Java, IdentifierPosition::Member, "a method signature")
            .expect_err("`new` is not a legal Java member name");

        assert_eq!(violation.name, "new");
        assert_eq!(violation.lang, Language::Java);
        assert_eq!(violation.position, IdentifierPosition::Member);
        assert_eq!(violation.reason, "reserved word");
        assert_eq!(violation.context, "a method signature");

        let rendered = violation.to_string();
        assert!(rendered.contains("`new`"), "{rendered}");
        assert!(rendered.contains("reserved word"), "{rendered}");
        assert!(rendered.contains("member position"), "{rendered}");
        assert!(rendered.contains("a method signature"), "{rendered}");
        assert!(rendered.contains("Java"), "{rendered}");
    }

    /// The exact tslp input: a napi method named `new`. This must now be `Ok`, and
    /// `report_identifier_violation` must return normally rather than unwinding.
    #[test]
    fn test_check_identifier_accepts_the_napi_static_new_that_used_to_abort_the_docs_run() {
        assert_eq!(
            check_identifier("new", Language::Node, IdentifierPosition::Member, "a method signature"),
            Ok(())
        );
        report_identifier_violation("new", Language::Node, IdentifierPosition::Member, "a method signature");
    }

    /// A violation must not unwind either -- it is reported through `tracing` and rendering
    /// continues, so one bad name costs an ERROR event rather than the whole run.
    #[test]
    fn test_report_identifier_violation_does_not_unwind_on_a_violation() {
        report_identifier_violation("new", Language::Java, IdentifierPosition::Member, "a test context");
        report_identifier_violation(
            "createClient",
            Language::Java,
            IdentifierPosition::Member,
            "a test context",
        );
    }

    #[test]
    fn test_doc_type_with_optional_true_wraps_correctly() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Node, true, TEST_PREFIX),
            "string | null"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Go, true, TEST_PREFIX),
            "*string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Csharp, true, TEST_PREFIX),
            "string?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ruby, true, TEST_PREFIX),
            "String?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Php, true, TEST_PREFIX),
            "?string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Elixir, true, TEST_PREFIX),
            "String.t() | nil"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::R, true, TEST_PREFIX),
            "character or NULL"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
        // ~keep A single pointer, not a double pointer: the real cbindgen header declares
        // optional string params as plain `const char *` (nullable in place), e.g. a
        // consumer's generated `<prefix>_create_client` and its `base_url`/`model_hint`
        // params -- confirmed against that generated header, not inferred from the type
        // system. This was the exact `const char**`-vs-header-`const char *` divergence
        // the params-table fix exists to close; pinning the old doubled value here would
        // have re-legalized the bug.
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ffi, true, TEST_PREFIX),
            "const char*"
        );
    }

    #[test]
    fn test_doc_type_with_optional_false_is_identity() {
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Rust,
        ] {
            assert_eq!(
                doc_type_with_optional(&TypeRef::String, lang, false, TEST_PREFIX),
                crate::docs::doc_type(&TypeRef::String, lang, TEST_PREFIX),
                "optional=false should be identity for {lang:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_with_optional_does_not_double_wrap_already_optional_type() {
        let already_optional = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_i32() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::I32),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "@Nullable Integer"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_bool() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::Bool),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "@Nullable Boolean"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_f64() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::F64),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "@Nullable Double"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_non_primitive_not_double_boxed() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Java, true, TEST_PREFIX),
            "@Nullable String"
        );
    }

    #[test]
    fn test_doc_type_with_optional_named_ffi_is_bare_scalar_handle_not_a_pointer() {
        let ty = TypeRef::Named("ClientConfig".to_string());
        let rendered = doc_type_with_optional(&ty, Language::Ffi, true, TEST_PREFIX);
        assert_eq!(rendered, "HTMAlefHandle");
        assert!(
            !rendered.contains('*'),
            "handle must not be pointer-suffixed: {rendered}"
        );
        assert!(
            !rendered.contains("ClientConfig"),
            "must not name the concrete Rust type: {rendered}"
        );
    }

    #[test]
    fn test_doc_type_with_optional_string_ffi_is_single_pointer_not_double() {
        // Regression: this is the params-table / struct-fields-table half of the
        // `const char**`-vs-header-`const char *` divergence; type_mapping.rs's `Optional`
        // arm covers the other half (a literal `TypeRef::Optional(String)`).
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ffi, true, TEST_PREFIX),
            "const char*"
        );
    }

    #[test]
    fn test_doc_type_with_optional_string_jni_currently_emits_c_double_pointer() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Jni, true, TEST_PREFIX),
            "const char**"
        );
    }

    /// ~keep The parameters table (`doc_type_with_optional`) and the function/method
    /// signature line (`type_mapping::doc_type`, via `signatures.rs`) render the same
    /// Named type in two different places on the same generated page. They must agree on
    /// the handle token for a required field exactly as they do for an optional one --
    /// this is the params-table analogue of `ffi_handle_consistency.rs`'s
    /// signature-vs-example cross-check.
    #[test]
    fn test_doc_type_with_optional_agrees_with_doc_type_on_handle_token() {
        let ty = TypeRef::Named("ClientConfig".to_string());
        let required = doc_type(&ty, Language::Ffi, TEST_PREFIX);
        let optional = doc_type_with_optional(&ty, Language::Ffi, true, TEST_PREFIX);
        assert_eq!(
            required, optional,
            "a Named type must render identically whether required or optional"
        );
        assert_eq!(required, "HTMAlefHandle");
    }

    // ~keep The on-error return value depends on the function's return shape; traced from
    // backends/ffi/gen_bindings/{functions/orchestration,helpers}.rs -- see ffi_error_return_phrase's
    // doc comment for the source citations. One case per row of that table.
    #[test]
    fn test_format_error_phrase_ffi_unit_return_reports_negative_one() {
        assert_eq!(
            format_error_phrase("InitError", &TypeRef::Unit, Language::Ffi, TEST_CRATE_NAME),
            "Returns `-1` on error."
        );
    }

    #[test]
    fn test_format_error_phrase_ffi_bytes_return_reports_negative_one() {
        // A Bytes result is always delivered via out-param + i32 status, fallible or not.
        assert_eq!(
            format_error_phrase("ReadError", &TypeRef::Bytes, Language::Ffi, TEST_CRATE_NAME),
            "Returns `-1` on error."
        );
    }

    #[test]
    fn test_format_error_phrase_ffi_named_return_reports_integer_sentinel_not_null() {
        let phrase = format_error_phrase(
            "ParseError",
            &TypeRef::Named("ConversionResult".to_string()),
            Language::C,
            TEST_CRATE_NAME,
        );
        assert_eq!(phrase, "Returns the sentinel handle `0` on error.");
        assert!(
            !phrase.contains("NULL"),
            "a scalar handle has no pointer to compare: {phrase}"
        );
    }

    #[test]
    fn test_format_error_phrase_ffi_string_return_stays_null() {
        // A genuine heap-allocated pointer return really does use NULL on failure.
        assert_eq!(
            format_error_phrase("ReadError", &TypeRef::String, Language::C, TEST_CRATE_NAME),
            "Returns `NULL` on error."
        );
        assert_eq!(
            format_error_phrase(
                "ReadError",
                &TypeRef::Vec(Box::new(TypeRef::String)),
                Language::C,
                TEST_CRATE_NAME
            ),
            "Returns `NULL` on error."
        );
        assert_eq!(
            format_error_phrase(
                "ReadError",
                &TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                Language::C,
                TEST_CRATE_NAME
            ),
            "Returns `NULL` on error."
        );
    }

    #[test]
    fn test_format_error_phrase_ffi_numeric_primitive_reports_zero() {
        assert_eq!(
            format_error_phrase(
                "ComputeError",
                &TypeRef::Primitive(PrimitiveType::I32),
                Language::C,
                TEST_CRATE_NAME
            ),
            "Returns `0` on error."
        );
        assert_eq!(
            format_error_phrase(
                "ComputeError",
                &TypeRef::Primitive(PrimitiveType::F64),
                Language::C,
                TEST_CRATE_NAME
            ),
            "Returns `0.0` on error."
        );
        assert_eq!(
            format_error_phrase("ComputeError", &TypeRef::Duration, Language::C, TEST_CRATE_NAME),
            "Returns `0` on error."
        );
    }

    #[test]
    fn test_format_error_phrase_ffi_optional_named_recurses_to_inner_shape() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("ClientConfig".to_string())));
        assert_eq!(
            format_error_phrase("AttachError", &ty, Language::C, TEST_CRATE_NAME),
            "Returns the sentinel handle `0` on error."
        );
    }

    #[test]
    fn test_format_error_phrase_jni_currently_emits_fixed_null_phrase() {
        // ~keep Characterisation only: this pins what the Jni arm currently emits, NOT what it
        // ought to emit. Jni is reachable and the phrase is wrong -- a JNI shim throws
        // `<Bridge>Exception` and returns a type-specific zero, never `NULL`. See
        // docs::examples::sample_param_value; expect this test to change when the arm is fixed.
        assert_eq!(
            format_error_phrase("InitError", &TypeRef::Unit, Language::Jni, TEST_CRATE_NAME),
            "Returns `NULL` on error."
        );
        assert_eq!(
            format_error_phrase(
                "ParseError",
                &TypeRef::Named("ConversionResult".to_string()),
                Language::Jni,
                TEST_CRATE_NAME
            ),
            "Returns `NULL` on error."
        );
    }

    #[test]
    fn test_format_default_bool_literal_python_uses_capitalised_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        let field_false = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(false)),
        );
        assert_eq!(
            format_field_default(&field_true, Language::Python, &api, TEST_PREFIX),
            "`True`"
        );
        assert_eq!(
            format_field_default(&field_false, Language::Python, &api, TEST_PREFIX),
            "`False`"
        );
    }

    #[test]
    fn test_format_default_bool_literal_non_python_uses_lowercase_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        for lang in [Language::Rust, Language::Java, Language::Go, Language::Node] {
            assert_eq!(
                format_field_default(&field_true, lang, &api, TEST_PREFIX),
                "`true`",
                "bool literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_string_literal_all_languages_produce_quoted_form() {
        let api = empty_api();
        let field = make_field(
            "name",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral("hello".to_string())),
        );
        for lang in [
            Language::Python,
            Language::Rust,
            Language::Java,
            Language::Go,
            Language::Node,
        ] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`\"hello\"`",
                "string literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal() {
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::IntLiteral(42)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Node] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`42`",
                "int literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal_on_duration_field_shows_ms_suffix() {
        let api = empty_api();
        let field = make_field(
            "timeout",
            TypeRef::Duration,
            false,
            Some(DefaultValue::IntLiteral(5000)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Go] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`5000ms`",
                "duration field should show ms suffix for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_float_literal() {
        let api = empty_api();
        let field = make_field(
            "confidence",
            TypeRef::Primitive(PrimitiveType::F32),
            false,
            Some(DefaultValue::FloatLiteral(0.85)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`0.85`",
                "float literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_enum_variant_qualified_python_and_rust() {
        let api = empty_api();
        let field = make_field(
            "style",
            TypeRef::Named("HeadingStyle".to_string()),
            false,
            Some(DefaultValue::EnumVariant("HeadingStyle::Atx".to_string())),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HeadingStyle::Atx`"
        );
        // Java declares the enum constant in SCREAMING_SNAKE, its own convention
        // (`backends/java/gen_bindings/types/enums.rs::gen_enum_class`, which routes the constant
        // through the same `public_casing` authority these docs do). This comment previously said
        // the opposite and was accurate at the time: the generator pushed `variant.name` verbatim,
        // so the docs were correctly describing a generator that was itself wrong. ~keep
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        assert_eq!(
            format_field_default(&field, Language::Ruby, &api, TEST_PREFIX),
            "`:atx`"
        );
        // PHP's class constant uppercases the whole variant name with no underscores inserted
        // (`backends/php/gen_bindings/types/enums.rs::enum_constant_entries`). ~keep
        assert_eq!(
            format_field_default(&field, Language::Php, &api, TEST_PREFIX),
            "`HeadingStyle::ATX`"
        );
    }

    #[test]
    fn test_format_default_tuple_variant_qualifies_variant_and_renders_arguments() {
        let api = empty_api();
        let field = make_field(
            "kind",
            TypeRef::Named("Kind".to_string()),
            false,
            Some(DefaultValue::TupleVariant(
                "Scaled".to_string(),
                vec![
                    DefaultValue::IntLiteral(5),
                    DefaultValue::StringLiteral("x".to_string()),
                ],
            )),
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`Kind::Scaled(5, \"x\")`"
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`Kind.SCALED(5, \"x\")`"
        );
    }

    #[test]
    fn test_format_default_struct_variant_qualifies_variant_and_renders_named_fields() {
        let api = empty_api();
        let field = make_field(
            "kind",
            TypeRef::Named("Kind".to_string()),
            false,
            Some(DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![
                    ("label".to_string(), DefaultValue::StringLiteral("balanced".to_string())),
                    ("weight".to_string(), DefaultValue::IntLiteral(3)),
                ],
            )),
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`Kind::Curated { label: \"balanced\", weight: 3 }`"
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`Kind.CURATED { label: \"balanced\", weight: 3 }`"
        );
    }

    #[test]
    fn test_format_default_empty_vec_field() {
        let api = empty_api();
        let field = make_field(
            "items",
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`vec![]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyList()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new List<string>()`"
        );
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`list()`");
        assert_eq!(format_field_default(&field, Language::Ruby, &api, TEST_PREFIX), "`[]`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_empty_map_field() {
        let api = empty_api();
        let field = make_field(
            "attributes",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HashMap::new()`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyMap()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`%{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new Dictionary<string, string>()`"
        );
    }

    #[test]
    fn test_format_default_none_on_optional_field() {
        let api = empty_api();
        let field = make_field("label", TypeRef::String, true, Some(DefaultValue::None));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(
            format_field_default(&field, Language::Node, &api, TEST_PREFIX),
            "`null`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_none_on_non_optional_field_returns_dash() {
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::None),
        );
        assert_eq!(format_field_default(&field, Language::Python, &api, TEST_PREFIX), "—");
    }

    #[test]
    fn test_format_default_empty_duration_shows_zero_ms_for_non_rust() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`0ms`"
        );
        assert_eq!(format_field_default(&field, Language::Java, &api, TEST_PREFIX), "`0ms`");
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`0ms`");
    }

    #[test]
    fn test_format_default_empty_duration_rust_shows_duration_default() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`Duration::default()`"
        );
    }

    #[test]
    fn test_format_default_multiline_raw_string_collapsed() {
        let api = empty_api();
        let mut field = make_field("config", TypeRef::String, false, None);
        field.default = Some("line1\nline2\nline3".to_string());
        let result = format_field_default(&field, Language::Python, &api, TEST_PREFIX);
        assert_eq!(result, "`line1 line2 line3`", "multiline defaults must be collapsed");
        assert!(!result.contains('\n'), "backtick code span must be single-line");
    }

    #[test]
    fn test_format_default_raw_string_with_extra_spaces_collapsed() {
        let api = empty_api();
        let mut field = make_field("config", TypeRef::String, false, None);
        field.default = Some("value   with    spaces".to_string());
        let result = format_field_default(&field, Language::Python, &api, TEST_PREFIX);
        assert!(result.contains("value"), "default should be wrapped");
        assert!(!result.contains("   "), "extra spaces should be normalized");
    }

    #[test]
    fn test_format_default_string_literal_with_newlines_collapsed() {
        let api = empty_api();
        let field = make_field(
            "marker_format",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral("line1\nline2\nline3".to_string())),
        );
        let result = format_field_default(&field, Language::Python, &api, TEST_PREFIX);
        assert_eq!(result, "`\"line1 line2 line3\"`");
        assert!(!result.contains('\n'), "newlines must be removed");
    }

    #[test]
    fn test_format_default_string_literal_with_pipes_and_newlines() {
        let api = empty_api();
        let field = make_field(
            "marker_format",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral(
                "\n\n<!-- PAGE {page_num} -->\n\n".to_string(),
            )),
        );
        let result = format_field_default(&field, Language::Python, &api, TEST_PREFIX);
        assert!(!result.contains('\n'), "newlines must be collapsed");
        let without_backticks = result.trim_matches('`').trim_matches('"');
        assert!(
            without_backticks.starts_with("<!-- PAGE"),
            "normalized: {}",
            without_backticks
        );
    }

    #[test]
    fn test_format_default_string_literal_with_extra_spaces() {
        let api = empty_api();
        let field = make_field(
            "options",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral("option1  option2   option3".to_string())),
        );
        let result = format_field_default(&field, Language::Python, &api, TEST_PREFIX);
        assert!(!result.contains("  "), "multiple spaces must be normalized: {}", result);
    }

    #[test]
    fn nested_defaults_preserve_backtick_trimming_and_language_literals() {
        let value = DefaultValue::ListLiteral(vec![
            DefaultValue::BoolLiteral(true),
            DefaultValue::TupleVariant("Scaled".into(), vec![DefaultValue::IntLiteral(5)]),
            DefaultValue::StructVariant(
                "Curated".into(),
                vec![("label".into(), DefaultValue::StringLiteral("a\n b".into()))],
            ),
        ]);
        assert_eq!(
            format_typed_default(
                &value,
                &TypeRef::Named("Kind".into()),
                Language::Python,
                &empty_api(),
                TEST_PREFIX,
                false
            ),
            r#"`[True, Kind.SCALED(5), Kind.CURATED { label: "a b" }]`"#
        );
    }
}
