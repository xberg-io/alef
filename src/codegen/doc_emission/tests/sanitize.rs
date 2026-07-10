use crate::codegen::doc_emission::{
    DocTarget, emit_csharp_doc, emit_rustdoc, example_for_target, sanitize_rust_idioms,
    sanitize_rust_idioms_keep_sections,
};

// --- emit_rustdoc intra-doc de-linking tests ---

/// Run `emit_rustdoc` and return the rendered doc block.
fn rustdoc(doc: &str) -> String {
    let mut out = String::new();
    emit_rustdoc(&mut out, doc, "");
    out
}

#[test]
fn emit_rustdoc_delinks_error_variant_references() {
    for variant in ["Error::LanguageNotFound", "Error::ParserSetup", "Error::Download"] {
        let out = rustdoc(&format!("Returns [`{variant}`] when the language is missing."));
        assert!(out.contains(&format!("`{variant}`")), "code span preserved: {out}");
        assert!(
            !out.contains(&format!("[`{variant}`]")),
            "intra-doc link removed: {out}"
        );
        assert!(out.contains("::"), "Rust path separator preserved: {out}");
    }
}

#[test]
fn emit_rustdoc_delinks_function_references() {
    for func in ["get_language", "download", "downloaded_languages", "configure", "init"] {
        let out = rustdoc(&format!("See [`{func}`] for details."));
        assert!(out.contains(&format!("`{func}`")), "code span preserved: {out}");
        assert!(!out.contains(&format!("[`{func}`]")), "intra-doc link removed: {out}");
    }
}

#[test]
fn emit_rustdoc_delinks_self_method_reference() {
    let out = rustdoc("Call [`Self::ensure_languages`] first.");
    assert!(out.contains("`Self::ensure_languages`"), "code span preserved: {out}");
    assert!(
        !out.contains("[`Self::ensure_languages`]"),
        "intra-doc link removed: {out}"
    );
    assert!(out.contains("::"), "`::` preserved verbatim: {out}");
}

#[test]
fn emit_rustdoc_delinks_explicit_intradoc_target() {
    let out = rustdoc("Returns [`Error::Download`](crate::Error::Download) on network failure.");
    assert!(out.contains("`Error::Download`"), "code span preserved: {out}");
    assert!(!out.contains("(crate::"), "explicit intra-doc target dropped: {out}");
    assert!(!out.contains("[`"), "no intra-doc link form remains: {out}");
}

#[test]
fn emit_rustdoc_delinks_bare_bracket_reference() {
    let out = rustdoc("See [get_language] to resolve a grammar.");
    assert!(
        out.contains("`get_language`"),
        "bare bracket wrapped in code span: {out}"
    );
    assert!(!out.contains("[get_language]"), "bare intra-doc link removed: {out}");
}

#[test]
fn emit_rustdoc_preserves_real_url_links() {
    let out = rustdoc("See [the tree-sitter docs](https://tree-sitter.github.io/) for grammars.");
    assert!(
        out.contains("[the tree-sitter docs](https://tree-sitter.github.io/)"),
        "genuine URL markdown link left intact: {out}"
    );
}

#[test]
fn emit_rustdoc_preserves_http_and_anchor_links() {
    let http = rustdoc("Mirror at [legacy](http://example.com/grammars).");
    assert!(
        http.contains("[legacy](http://example.com/grammars)"),
        "http link preserved: {http}"
    );
    let anchor = rustdoc("Jump to [the errors section](#errors).");
    assert!(
        anchor.contains("[the errors section](#errors)"),
        "anchor link preserved: {anchor}"
    );
}

#[test]
fn emit_rustdoc_does_not_mangle_existing_code_spans_with_brackets() {
    let out = rustdoc("Index with `slice[0]` to read the first element.");
    assert!(out.contains("`slice[0]`"), "inline code span preserved verbatim: {out}");
}

#[test]
fn emit_rustdoc_leaves_non_identifier_brackets_alone() {
    let out = rustdoc("Optional [see below] for the rationale.");
    assert!(out.contains("[see below]"), "non-identifier bracket left alone: {out}");
}

#[test]
fn sanitize_intradoc_link_with_path_separator_java() {
    let input = "See [`ConversionOptions::builder()`] for details.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("`ConversionOptions.builder()`"), "got: {out}");
    assert!(!out.contains("[`"), "brackets must be removed, got: {out}");
}

#[test]
fn sanitize_intradoc_link_simple_type_php() {
    let input = "Returns a [`ConversionResult`].";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("`ConversionResult`"), "got: {out}");
    assert!(!out.contains("[`"), "got: {out}");
}

#[test]
fn sanitize_none_to_null_javadoc() {
    let input = "Returns None when no value is found.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("null"), "got: {out}");
    assert!(!out.contains("None"), "got: {out}");
}

#[test]
fn sanitize_none_to_undefined_tsdoc() {
    let input = "Returns None if absent.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("undefined"), "got: {out}");
    assert!(!out.contains("None"), "got: {out}");
}

#[test]
fn sanitize_some_x_to_the_value_x() {
    let input = "Pass Some(value) to enable.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("the value (value)"), "got: {out}");
    assert!(!out.contains("Some("), "got: {out}");
}

#[test]
fn sanitize_bare_some_followed_by_lowercase_noun_is_dropped() {
    let input = "Only specified fields (Some values) will override existing options; None values leave the previous";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(
        out.contains("(values)"),
        "bare `Some ` before lowercase noun must be stripped; got: {out}"
    );
    assert!(
        out.contains("null values"),
        "bare `None ` must also be replaced; got: {out}"
    );
    assert!(!out.contains("Some "), "Some prefix must not survive; got: {out}");
}

#[test]
fn sanitize_bare_some_does_not_touch_identifiers_or_uppercase_followers() {
    let cases = [
        "SomeType lives on.",
        "Some.method() returns Self.",
        "Some Title",
        "Some(x) is a value.",
    ];
    for case in cases {
        let out = sanitize_rust_idioms(case, DocTarget::JavaDoc);
        if case.starts_with("Some(") {
            assert!(out.contains("the value (x)"), "got: {out}");
        } else {
            assert!(out.contains("Some"), "Some must survive in {case:?}; got: {out}");
        }
    }
}

#[test]
fn sanitize_option_t_to_nullable_php() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("String?"), "got: {out}");
    assert!(!out.contains("Option<"), "got: {out}");
}

#[test]
fn sanitize_option_t_to_or_null_java() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("String | null"), "got: {out}");
}

#[test]
fn sanitize_option_t_to_or_undefined_tsdoc() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("String | undefined"), "got: {out}");
}

#[test]
fn sanitize_vec_u8_per_target() {
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::PhpDoc).contains("string"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JavaDoc).contains("byte[]"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::TsDoc).contains("Uint8Array"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JsDoc).contains("Uint8Array"));
}

#[test]
fn sanitize_vec_t_to_array() {
    let input = "Returns Vec<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("String[]"), "got: {out}");
    assert!(!out.contains("Vec<"), "got: {out}");
}

#[test]
fn sanitize_hashmap_per_target() {
    let input = "Uses HashMap<String, u32>.";
    assert!(sanitize_rust_idioms(input, DocTarget::PhpDoc).contains("array<String, u32>"));
    assert!(sanitize_rust_idioms(input, DocTarget::JavaDoc).contains("Map<String, u32>"));
    assert!(sanitize_rust_idioms(input, DocTarget::TsDoc).contains("Record<String, u32>"));
}

#[test]
fn sanitize_arc_wrapper_stripped() {
    let input = "Holds Arc<Config>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("Config"), "got: {out}");
    assert!(!out.contains("Arc<"), "got: {out}");
}

#[test]
fn sanitize_box_mutex_rwlock_rc_cell_refcell_stripped() {
    for wrapper in &["Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
        let input = format!("Contains {wrapper}<Inner>.");
        let out = sanitize_rust_idioms(&input, DocTarget::JavaDoc);
        assert!(out.contains("Inner"), "wrapper {wrapper} not stripped, got: {out}");
        assert!(
            !out.contains(&format!("{wrapper}<")),
            "wrapper {wrapper} still present, got: {out}"
        );
    }
}

#[test]
fn sanitize_send_sync_stripped() {
    let input = "The type is Send + Sync.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("Send"), "got: {out}");
    assert!(!out.contains("Sync"), "got: {out}");
}

#[test]
fn sanitize_static_lifetime_stripped() {
    let input = "Requires 'static lifetime.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("'static"), "got: {out}");
}

#[test]
fn sanitize_pub_fn_stripped() {
    let input = "Calls pub fn convert().";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("pub fn"), "got: {out}");
    assert!(out.contains("convert()"), "got: {out}");
}

#[test]
fn sanitize_crate_prefix_stripped() {
    let input = "See crate::error::ConversionError.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("crate::"), "got: {out}");
    assert!(out.contains("error.ConversionError"), "got: {out}");
}

#[test]
fn sanitize_unwrap_expect_stripped() {
    let input = "Call result.unwrap() or result.expect(\"msg\").";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains(".unwrap()"), "got: {out}");
    assert!(!out.contains(".expect("), "got: {out}");
}

#[test]
fn sanitize_no_mutation_inside_backticks() {
    let input = "Use `None` as the argument.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("`None`"), "backtick span must be preserved, got: {out}");
}

#[test]
fn sanitize_rust_fence_dropped_for_tsdoc() {
    let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(
        !out.contains("let x = 1;"),
        "rust fence content must be dropped, got: {out}"
    );
    assert!(!out.contains("```rust"), "got: {out}");
    assert!(out.contains("Trailer."), "text after fence must survive, got: {out}");
}

#[test]
fn sanitize_rust_fence_dropped_for_java() {
    let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(
        !out.contains("let x = 1;"),
        "fence content must be dropped for Java, got: {out}"
    );
    assert!(!out.contains("```"), "fence markers must be dropped, got: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_non_rust_fence_passed_through() {
    let input = "Example:\n\n```typescript\nconst x = 1;\n```";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("```typescript"), "non-rust fence must survive, got: {out}");
    assert!(out.contains("const x = 1;"), "got: {out}");
}

#[test]
fn sanitize_backtick_code_span_not_mutated_option() {
    let input = "The type is `Option<String>`.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(
        out.contains("`Option<String>`"),
        "code span must be preserved, got: {out}"
    );
}

#[test]
fn sanitize_idempotent() {
    let input = "Returns None when Vec<String> is empty.";
    let once = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    let twice = sanitize_rust_idioms(&once, DocTarget::JavaDoc);
    assert_eq!(once, twice, "sanitize_rust_idioms should be idempotent");
}

#[test]
fn sanitize_multiline_prose() {
    let input = "Convert markup conversion.\n\nReturns None on failure.\nUse Option<String> for the result.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("null"), "None must be replaced on line 2, got: {out}");
    assert!(
        out.contains("String | null"),
        "Option<String> must be replaced on line 3, got: {out}"
    );
}

#[test]
fn sanitize_attribute_line_dropped() {
    let input = "#[derive(Debug, Clone)]\nSome documentation.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("#[derive("), "attribute line must be dropped, got: {out}");
    assert!(out.contains("documentation."), "prose must survive, got: {out}");
}

#[test]
fn sanitize_path_separator_in_prose() {
    let input = "See std::collections::HashMap for details.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("std.collections.HashMap"), ":: must become ., got: {out}");
}

#[test]
fn sanitize_none_not_replaced_inside_identifier() {
    let input = "Unlike NoneType in Python.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("NoneType"), "NoneType must not be replaced, got: {out}");
}

#[test]
fn sanitize_csharp_drops_rust_section_headings_and_example_body() {
    let input = "Convert error to HTTP status code\n\n\
            Maps GraphQL error types to status codes.\n\n\
            # Examples\n\n\
            ```ignore\n\
            use sample_router_graphql::error::GraphQLError;\n\
            let error = GraphQLError::AuthenticationError(\"Invalid token\".to_string());\n\
            assert_eq!(error.status_code(), 401);\n\
            ```\n";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(
        out.contains("Convert error to HTTP status code"),
        "summary preserved: {out}"
    );
    assert!(out.contains("Maps GraphQL error types"), "prose preserved: {out}");
    assert!(!out.contains("# Examples"), "heading dropped: {out}");
    assert!(!out.contains("```"), "code fence dropped: {out}");
    assert!(!out.contains("Self::error_code"), "Self::method dropped: {out}");
    assert!(
        !out.contains("GraphQLError::AuthenticationError"),
        "rust path dropped: {out}"
    );
}

#[test]
fn sanitize_csharp_intradoc_link_with_path_separator() {
    let input = "See [`Self::error_code`] for the variant codes.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("`Self.error_code`"), "intra-doc link normalised: {out}");
    assert!(!out.contains("[`"), "square brackets removed: {out}");
    assert!(!out.contains("::"), ":: replaced with .: {out}");
}

#[test]
fn sanitize_csharp_result_type_keeps_success_drops_error() {
    let input = "Returns Result<String, ConversionError> on failure.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("String"), "success type kept: {out}");
    assert!(!out.contains("Result<"), "Result wrapper dropped: {out}");
    assert!(!out.contains("ConversionError"), "error type dropped: {out}");
}

#[test]
fn sanitize_csharp_option_becomes_nullable() {
    let input = "Returns Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("String?"), "Option<T> -> T?: {out}");
    assert!(!out.contains("Option<"), "Option dropped: {out}");
}

#[test]
fn sanitize_csharp_vec_u8_becomes_byte_array() {
    let input = "Accepts Vec<u8>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("byte[]"), "Vec<u8> -> byte[]: {out}");
}

#[test]
fn sanitize_csharp_hashmap_becomes_dictionary() {
    let input = "Holds HashMap<String, u32>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(
        out.contains("Dictionary&lt;String, u32&gt;"),
        "HashMap -> Dictionary with XML-escaped brackets: {out}"
    );
}

#[test]
fn sanitize_csharp_none_to_null() {
    let input = "Returns None on miss.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("null"), "None -> null: {out}");
    assert!(!out.contains("None"), "None replaced: {out}");
}

#[test]
fn sanitize_csharp_escapes_raw_angle_brackets_and_amp() {
    let input = "Accepts Box<dyn Trait> and combines a & b.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("dyn Trait"), "Box<T> stripped: {out}");
    assert!(out.contains("&amp;"), "ampersand escaped: {out}");
}

#[test]
fn sanitize_csharp_drops_rust_code_fence_entirely() {
    let input = "Intro.\n\n```rust\nlet x: Vec<u8> = vec![];\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(!out.contains("let x"), "code fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_csharp_keep_sections_does_not_drop_headings() {
    let input = "Summary.\n\n# Arguments\n\n* `name` - the value.";
    let out = sanitize_rust_idioms_keep_sections(input, DocTarget::CSharpDoc);
    assert!(out.contains("# Arguments"), "heading preserved: {out}");
    assert!(out.contains("name"), "body preserved: {out}");
}

#[test]
fn sanitize_csharp_idempotent() {
    let input = "Returns Option<String> or None.";
    let once = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    let twice = sanitize_rust_idioms(&once, DocTarget::CSharpDoc);
    assert_eq!(once, twice, "CSharpDoc sanitisation must be idempotent");
}

#[test]
fn sanitize_phpdoc_drops_unmarked_rust_code_fences() {
    let input = "Detect language name from a file extension.\n\nReturns `None` for unrecognized extensions.\n\n```\nuse sample_language_pack::detect_language_from_extension;\nassert_eq!(detect_language_from_extension(\"py\"), Some(\"python\"));\nassert_eq!(detect_language_from_extension(\"RS\"), Some(\"rust\"));\nassert_eq!(detect_language_from_extension(\"xyz\"), None);\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(
        !out.contains("use sample_language_pack"),
        "Rust use stmt dropped: {out}"
    );
    assert!(!out.contains("assert_eq!"), "Rust code dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Detect language name"), "prose before fence kept: {out}");
    assert!(out.contains("unrecognized extensions"), "prose kept: {out}");
}

#[test]
fn sanitize_javadoc_drops_unmarked_rust_code_fences() {
    let input = "Process a file.\n\n```\nlet result = process(\"def hello(): pass\", &config).unwrap();\n```";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("unwrap"), "Rust unwrap dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Process a file"), "prose kept: {out}");
}

#[test]
fn sanitize_phpdoc_drops_explicit_rust_fences() {
    let input = "Summary.\n\n```rust\nuse std::path::PathBuf;\nlet p = PathBuf::from(\"/tmp\");\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("use std::"), "Rust code dropped: {out}");
    assert!(!out.contains("PathBuf"), "Rust types dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Summary"), "prose kept: {out}");
}

#[test]
fn sanitize_no_run_fence_dropped_for_tsdoc() {
    let input = "Intro.\n\n```no_run\nuse foo::bar;\nbar::init();\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("use foo::bar"), "no_run fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_ignore_fence_dropped_for_phpdoc() {
    let input = "Summary.\n\n```ignore\nlet x = 1;\n// this would not compile\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("let x = 1"), "ignore fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Summary"), "prose kept: {out}");
}

#[test]
fn sanitize_should_panic_fence_dropped_for_javadoc() {
    let input = "Panics on null.\n\n```should_panic\nlet _ = parse(null);\n```";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("parse(null)"), "should_panic fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Panics on null"), "prose kept: {out}");
}

#[test]
fn sanitize_compile_fail_fence_dropped_for_csharp() {
    let input = "Type safety demo.\n\n```compile_fail\nlet x: u32 = \"hello\";\n```";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(!out.contains("let x:"), "compile_fail fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Type safety demo"), "prose kept: {out}");
}

#[test]
fn sanitize_edition_fence_dropped_for_tsdoc() {
    let input = "Edition example.\n\n```edition2021\nuse std::fmt;\n```\n\nSee also edition2018.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("use std::fmt"), "edition2021 fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Edition example"), "prose kept: {out}");
}

#[test]
fn sanitize_python_fence_preserved_for_tsdoc() {
    let input = "Example:\n\n```python\nimport foo\nfoo.bar()\n```";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("```python"), "python fence preserved: {out}");
    assert!(out.contains("import foo"), "python body preserved: {out}");
}

#[test]
fn sanitize_javascript_fence_preserved_for_phpdoc() {
    let input = "Usage:\n\n```javascript\nconst x = require('foo');\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("```javascript"), "javascript fence preserved: {out}");
    assert!(out.contains("require('foo')"), "javascript body preserved: {out}");
}

#[test]
fn example_for_target_no_run_fence_suppressed_for_typescript() {
    let example = "```no_run\nuse sample_language_pack::available_languages;\nlet langs = available_languages();\n```";
    assert_eq!(
        example_for_target(example, "typescript"),
        None,
        "no_run fence must be treated as Rust and suppressed for TypeScript"
    );
}

#[test]
fn example_for_target_ignore_fence_suppressed_for_php() {
    let example = "```ignore\nlet x = 1;\n```";
    assert_eq!(
        example_for_target(example, "php"),
        None,
        "ignore fence must be treated as Rust and suppressed for PHP"
    );
}

#[test]
fn example_for_target_compile_fail_fence_suppressed_for_java() {
    let example = "```compile_fail\nlet x: u32 = \"wrong\";\n```";
    assert_eq!(
        example_for_target(example, "java"),
        None,
        "compile_fail fence must be treated as Rust and suppressed for Java"
    );
}

#[test]
fn example_for_target_should_panic_fence_suppressed_for_ruby() {
    let example = "```should_panic\nlet _ = parse(None);\n```";
    assert_eq!(
        example_for_target(example, "ruby"),
        None,
        "should_panic fence must be treated as Rust and suppressed for Ruby"
    );
}

#[test]
fn example_for_target_edition_fence_suppressed_for_php() {
    let example = "```edition2021\nuse std::fmt;\n```";
    assert_eq!(
        example_for_target(example, "php"),
        None,
        "edition2021 fence must be treated as Rust and suppressed for PHP"
    );
}

#[test]
fn example_for_target_python_fence_preserved() {
    let example = "```python\nimport foo\n```";
    let result = example_for_target(example, "php");
    assert!(result.is_some(), "python fence must be preserved for PHP target");
}

#[test]
fn emit_csharp_doc_multi_paragraph_with_intra_doc_link() {
    let input = "Stream a single-URL crawl, yielding [`CrawlEvent`]s as pages are processed.\n\nReturns an async stream that emits one event per crawled page, plus a\nterminal `Complete` event.";
    let mut out = String::new();
    emit_csharp_doc(&mut out, input, "    ", "TestException");

    assert!(out.contains("<summary>"), "summary tag present: {out}");
    assert!(out.contains("</summary>"), "closing summary tag present: {out}");

    assert!(
        out.contains("Stream a single-URL crawl"),
        "first paragraph present: {out}"
    );
    assert!(
        out.contains("Returns an async stream"),
        "second paragraph present: {out}"
    );

    assert!(
        out.contains("`CrawlEvent`"),
        "intra-doc link converted to code span: {out}"
    );
    assert!(
        !out.contains("[`CrawlEvent`]"),
        "square brackets removed from intra-doc link: {out}"
    );

    let lines: Vec<&str> = out.lines().collect();
    for line in lines {
        if !line.trim().is_empty() {
            assert!(line.contains("///"), "every non-empty line has /// prefix: {}", line);
        }
    }
}

#[test]
fn sanitize_rust_idioms_escapes_jsdoc_block_close() {
    let input = "A block or multi-line comment (e.g., `/* ... */`).";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(
        result.contains("* /"),
        "JSDoc block-close sequences must be escaped: {result}"
    );
    assert!(
        !result.contains("*/"),
        "JSDoc block-close should not appear unescaped: {result}"
    );
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_preserves_content() {
    let input = "Handle `/* ... */` and `/* comment */` patterns.";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(result.contains("Handle"), "Handle keyword preserved");
    assert!(result.contains("and"), "and keyword preserved");
    assert!(result.contains("patterns"), "patterns keyword preserved");
    assert!(result.contains("* /"), "escaped block-close preserved");
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_for_tsdoc_target() {
    let input = "Code example: `/* comment */`";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(result.contains("* /"), "TsDoc target must escape */ sequences");
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_for_jsdoc_target() {
    let input = "Code example: `/* comment */`";
    let result = sanitize_rust_idioms(input, DocTarget::JsDoc);
    assert!(result.contains("* /"), "JsDoc target must escape */ sequences");
}

#[test]
fn sanitize_rust_idioms_no_jsdoc_escape_for_other_targets() {
    let input = "Code example: `/* comment */`";
    let _result_phpdoc = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    let _result_csharp = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
}
